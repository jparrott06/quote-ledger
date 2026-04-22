//! Pure domain: quote state, events, and reduction.
//!
//! **Sequencing (`seq`)** is assigned only by the server when persisting events (Epic B). The
//! reducer is deterministic and does not depend on `seq` values.

mod error;
mod model;

pub use error::DomainError;
pub use model::{DomainCommand, DomainEvent, LineItemState, QuoteState};

/// Apply a single committed event to the current state (event sourcing replay / tail apply).
pub fn reduce(state: &QuoteState, event: &DomainEvent) -> Result<QuoteState, DomainError> {
    model::reduce(state, event)
}

/// Validate a command against the current state and emit the events that would be appended.
pub fn command_to_events(
    state: &QuoteState,
    command: &DomainCommand,
) -> Result<Vec<DomainEvent>, DomainError> {
    model::command_to_events(state, command)
}

/// Fold an event stream from an empty quote into the resulting state.
pub fn replay(events: impl IntoIterator<Item = DomainEvent>) -> Result<QuoteState, DomainError> {
    let mut state = QuoteState::default();
    for event in events {
        state = reduce(&state, &event)?;
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn created() -> DomainEvent {
        DomainEvent::QuoteCreated {
            currency_code: "USD".into(),
            jurisdiction_id: "US".into(),
        }
    }

    #[test]
    fn reduce_quote_created_from_empty() {
        let s = reduce(&QuoteState::default(), &created()).unwrap();
        assert!(s.is_created());
        assert_eq!(s.currency_code(), "USD");
        assert_eq!(s.jurisdiction_id(), "US");
        assert!(!s.finalized);
    }

    #[test]
    fn reduce_rejects_second_quote_created() {
        let s0 = reduce(&QuoteState::default(), &created()).unwrap();
        let err = reduce(&s0, &created()).unwrap_err();
        assert_eq!(err, DomainError::QuoteAlreadyCreated);
    }

    #[test]
    fn create_quote_emits_single_event() {
        let cmd = DomainCommand::CreateQuote {
            currency_code: "JPY".into(),
            jurisdiction_id: "JP".into(),
        };
        let events = command_to_events(&QuoteState::default(), &cmd).unwrap();
        assert_eq!(
            events,
            vec![DomainEvent::QuoteCreated {
                currency_code: "JPY".into(),
                jurisdiction_id: "JP".into(),
            }]
        );
    }

    #[test]
    fn create_quote_rejected_when_quote_exists() {
        let s = reduce(&QuoteState::default(), &created()).unwrap();
        let cmd = DomainCommand::CreateQuote {
            currency_code: "EUR".into(),
            jurisdiction_id: "DE".into(),
        };
        let err = command_to_events(&s, &cmd).unwrap_err();
        assert_eq!(err, DomainError::DuplicateCreateQuote);
    }

    #[test]
    fn replay_matches_folded_apply() {
        let events = command_to_events(
            &QuoteState::default(),
            &DomainCommand::CreateQuote {
                currency_code: "USD".into(),
                jurisdiction_id: "US".into(),
            },
        )
        .unwrap();
        let from_replay = replay(events.clone()).unwrap();
        let mut folded = QuoteState::default();
        for e in &events {
            folded = reduce(&folded, e).unwrap();
        }
        assert_eq!(from_replay, folded);
    }

    #[test]
    fn create_quote_rejects_blank_currency() {
        let cmd = DomainCommand::CreateQuote {
            currency_code: "  ".into(),
            jurisdiction_id: "US".into(),
        };
        let err = command_to_events(&QuoteState::default(), &cmd).unwrap_err();
        assert_eq!(
            err,
            DomainError::InvalidField {
                field: "currency_code"
            }
        );
    }

    fn state_after_create() -> QuoteState {
        reduce(&QuoteState::default(), &created()).unwrap()
    }

    #[test]
    fn add_line_then_totals_us_tax() {
        let s0 = state_after_create();
        let ev = DomainEvent::LineItemAdded {
            line_id: "L1".into(),
            sku: "SKU".into(),
            description: "Widget".into(),
            quantity: 2,
            unit_minor: 5_000,
        };
        let s1 = reduce(&s0, &ev).unwrap();
        assert_eq!(s1.subtotal_minor().unwrap(), 10_000);
        assert_eq!(s1.tax_minor().unwrap(), 800);
        assert_eq!(s1.total_minor().unwrap(), 10_800);
    }

    #[test]
    fn finalize_requires_line() {
        let s = state_after_create();
        let err = command_to_events(&s, &DomainCommand::FinalizeQuote).unwrap_err();
        assert_eq!(err, DomainError::CannotFinalizeWithoutLines);
    }

    #[test]
    fn finalize_sets_flag() {
        let s0 = state_after_create();
        let s1 = reduce(
            &s0,
            &DomainEvent::LineItemAdded {
                line_id: "L1".into(),
                sku: "SKU".into(),
                description: "Widget".into(),
                quantity: 1,
                unit_minor: 100,
            },
        )
        .unwrap();
        let s2 = reduce(&s1, &DomainEvent::QuoteFinalized).unwrap();
        assert!(s2.finalized);
        let err = command_to_events(
            &s2,
            &DomainCommand::AddLineItem {
                line_id: "L2".into(),
                sku: "SKU".into(),
                description: "Nope".into(),
                quantity: 1,
                unit_minor: 1,
            },
        )
        .unwrap_err();
        assert_eq!(err, DomainError::QuoteAlreadyFinalized);
    }

    use proptest::prelude::*;

    #[test]
    fn proptest_create_quote_emits_single_created_event() {
        proptest!(|(currency in "[A-Z]{3}", jur in "US|US-CA|DE|JP")| {
            let cmd = DomainCommand::CreateQuote {
                currency_code: currency.clone(),
                jurisdiction_id: jur.clone(),
            };
            let events = command_to_events(&QuoteState::default(), &cmd).map_err(|e| {
                proptest::test_runner::TestCaseError::fail(format!("{e:?}"))
            })?;
            prop_assert_eq!(events.len(), 1);
            let is_created = matches!(&events[0], DomainEvent::QuoteCreated { .. });
            prop_assert!(is_created);
            if let DomainEvent::QuoteCreated {
                currency_code,
                jurisdiction_id,
            } = &events[0]
            {
                prop_assert_eq!(currency_code, &currency);
                prop_assert_eq!(jurisdiction_id, &jur);
            }
            let s = reduce(&QuoteState::default(), &events[0]).map_err(|e| {
                proptest::test_runner::TestCaseError::fail(format!("{e:?}"))
            })?;
            prop_assert!(s.is_created());
        });
    }
}

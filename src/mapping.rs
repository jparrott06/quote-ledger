use tonic::Status;

use crate::domain::{self, DomainCommand, DomainError, DomainEvent, QuoteState};
use crate::v1::quote_command::Kind as CmdKind;
use crate::v1::quote_event::Kind as EvKind;
use crate::v1::{
    AddLineItem, CreateQuote, FinalizeQuote, LineItemAdded, LineItemView, QuoteCommand,
    QuoteCreated, QuoteEvent, QuoteFinalized, QuoteView,
};

pub fn proto_command_to_domain(cmd: &QuoteCommand) -> Result<DomainCommand, Status> {
    let k = cmd
        .kind
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("command.kind required"))?;
    match k {
        CmdKind::CreateQuote(CreateQuote {
            currency_code,
            jurisdiction_id,
        }) => Ok(DomainCommand::CreateQuote {
            currency_code: currency_code.clone(),
            jurisdiction_id: jurisdiction_id.clone(),
        }),
        CmdKind::AddLineItem(AddLineItem {
            line_id,
            sku,
            description,
            quantity,
            unit_minor,
        }) => Ok(DomainCommand::AddLineItem {
            line_id: line_id.clone(),
            sku: sku.clone(),
            description: description.clone(),
            quantity: *quantity,
            unit_minor: *unit_minor,
        }),
        CmdKind::FinalizeQuote(FinalizeQuote {}) => Ok(DomainCommand::FinalizeQuote),
    }
}

pub fn domain_event_to_proto(ev: &DomainEvent) -> QuoteEvent {
    match ev {
        DomainEvent::QuoteCreated {
            currency_code,
            jurisdiction_id,
        } => QuoteEvent {
            kind: Some(EvKind::QuoteCreated(QuoteCreated {
                currency_code: currency_code.clone(),
                jurisdiction_id: jurisdiction_id.clone(),
            })),
        },
        DomainEvent::LineItemAdded {
            line_id,
            sku,
            description,
            quantity,
            unit_minor,
        } => QuoteEvent {
            kind: Some(EvKind::LineItemAdded(LineItemAdded {
                line_id: line_id.clone(),
                sku: sku.clone(),
                description: description.clone(),
                quantity: *quantity,
                unit_minor: *unit_minor,
            })),
        },
        DomainEvent::QuoteFinalized => QuoteEvent {
            kind: Some(EvKind::QuoteFinalized(QuoteFinalized {})),
        },
    }
}

pub fn stored_event_to_domain_event_store(
    stored: &crate::v1::StoredEvent,
) -> Result<DomainEvent, crate::error::StoreError> {
    let ev = stored
        .event
        .as_ref()
        .ok_or_else(|| crate::error::StoreError::Corrupt("stored event missing payload".into()))?;
    match ev
        .kind
        .as_ref()
        .ok_or_else(|| crate::error::StoreError::Corrupt("stored event missing kind".into()))?
    {
        EvKind::QuoteCreated(QuoteCreated {
            currency_code,
            jurisdiction_id,
        }) => Ok(DomainEvent::QuoteCreated {
            currency_code: currency_code.clone(),
            jurisdiction_id: jurisdiction_id.clone(),
        }),
        EvKind::LineItemAdded(LineItemAdded {
            line_id,
            sku,
            description,
            quantity,
            unit_minor,
        }) => Ok(DomainEvent::LineItemAdded {
            line_id: line_id.clone(),
            sku: sku.clone(),
            description: description.clone(),
            quantity: *quantity,
            unit_minor: *unit_minor,
        }),
        EvKind::QuoteFinalized(QuoteFinalized {}) => Ok(DomainEvent::QuoteFinalized),
    }
}

pub fn replay_stored_to_state(
    events: &[crate::v1::StoredEvent],
) -> Result<QuoteState, crate::error::StoreError> {
    let mut state = QuoteState::default();
    for stored in events {
        let ev = stored_event_to_domain_event_store(stored)?;
        state = domain::reduce(&state, &ev)?;
    }
    Ok(state)
}

pub fn quote_state_to_view(quote_id: &str, state: &QuoteState) -> Result<QuoteView, DomainError> {
    let subtotal_minor = state.subtotal_minor()?;
    let tax_minor = state.tax_minor()?;
    let total_minor = state.total_minor()?;

    let mut line_items = Vec::new();
    for l in state.lines() {
        line_items.push(LineItemView {
            line_id: l.line_id.clone(),
            sku: l.sku.clone(),
            description: l.description.clone(),
            quantity: l.quantity,
            unit_minor: l.unit_minor,
            line_total_minor: l.line_total_minor()?,
        });
    }

    Ok(QuoteView {
        quote_id: quote_id.to_string(),
        currency_code: state.currency_code().to_string(),
        jurisdiction_id: state.jurisdiction_id().to_string(),
        finalized: state.finalized,
        subtotal_minor,
        tax_minor,
        total_minor,
        line_items,
    })
}

pub fn domain_error_to_status(err: DomainError) -> Status {
    match err {
        DomainError::QuoteAlreadyCreated | DomainError::DuplicateCreateQuote => {
            Status::failed_precondition(err.to_string())
        }
        DomainError::InvalidField { .. }
        | DomainError::InvalidQuantity
        | DomainError::InvalidUnitMinor
        | DomainError::DuplicateLineId => Status::invalid_argument(err.to_string()),
        DomainError::QuoteNotCreated
        | DomainError::QuoteAlreadyFinalized
        | DomainError::CannotFinalizeWithoutLines => Status::failed_precondition(err.to_string()),
        DomainError::IntegerOverflow => Status::out_of_range(err.to_string()),
    }
}

pub fn store_error_to_status(err: crate::error::StoreError) -> Status {
    use crate::error::StoreError;
    match err {
        StoreError::InvalidAfterSeq { .. } | StoreError::EmptyAppend => {
            Status::invalid_argument(err.to_string())
        }
        StoreError::Corrupt(_) => Status::data_loss(err.to_string()),
        StoreError::Sqlite(_) => Status::internal(err.to_string()),
        StoreError::Domain(err) => domain_error_to_status(err),
    }
}

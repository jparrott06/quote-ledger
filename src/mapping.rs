use tonic::Status;

use crate::domain::{self, DomainCommand, DomainError, DomainEvent, QuoteState};
use crate::v1::quote_command::Kind as CmdKind;
use crate::v1::quote_event::Kind as EvKind;
use crate::v1::{CreateQuote, QuoteCommand, QuoteCreated, QuoteEvent, QuoteView};

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

pub fn quote_state_to_view(quote_id: &str, state: &QuoteState) -> QuoteView {
    QuoteView {
        quote_id: quote_id.to_string(),
        currency_code: state.currency_code().to_string(),
        jurisdiction_id: state.jurisdiction_id().to_string(),
        finalized: state.finalized,
    }
}

pub fn domain_error_to_status(err: DomainError) -> Status {
    match err {
        DomainError::QuoteAlreadyCreated | DomainError::DuplicateCreateQuote => {
            Status::failed_precondition(err.to_string())
        }
        DomainError::InvalidField { .. } => Status::invalid_argument(err.to_string()),
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

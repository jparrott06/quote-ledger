use super::DomainError;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuoteState {
    currency_code: Option<String>,
    jurisdiction_id: Option<String>,
    pub finalized: bool,
}

impl QuoteState {
    pub fn is_created(&self) -> bool {
        self.currency_code.is_some()
    }

    pub fn currency_code(&self) -> &str {
        self.currency_code.as_deref().unwrap_or("")
    }

    pub fn jurisdiction_id(&self) -> &str {
        self.jurisdiction_id.as_deref().unwrap_or("")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainEvent {
    QuoteCreated {
        currency_code: String,
        jurisdiction_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainCommand {
    CreateQuote {
        currency_code: String,
        jurisdiction_id: String,
    },
}

pub(super) fn reduce(state: &QuoteState, event: &DomainEvent) -> Result<QuoteState, DomainError> {
    match event {
        DomainEvent::QuoteCreated {
            currency_code,
            jurisdiction_id,
        } => {
            if state.is_created() {
                return Err(DomainError::QuoteAlreadyCreated);
            }
            Ok(QuoteState {
                currency_code: Some(currency_code.clone()),
                jurisdiction_id: Some(jurisdiction_id.clone()),
                finalized: state.finalized,
            })
        }
    }
}

pub(super) fn command_to_events(
    state: &QuoteState,
    command: &DomainCommand,
) -> Result<Vec<DomainEvent>, DomainError> {
    match command {
        DomainCommand::CreateQuote {
            currency_code,
            jurisdiction_id,
        } => {
            if state.is_created() {
                return Err(DomainError::DuplicateCreateQuote);
            }
            require_non_blank("currency_code", currency_code)?;
            require_non_blank("jurisdiction_id", jurisdiction_id)?;
            Ok(vec![DomainEvent::QuoteCreated {
                currency_code: currency_code.clone(),
                jurisdiction_id: jurisdiction_id.clone(),
            }])
        }
    }
}

fn require_non_blank(field: &'static str, value: &str) -> Result<(), DomainError> {
    if value.trim().is_empty() {
        return Err(DomainError::InvalidField { field });
    }
    Ok(())
}

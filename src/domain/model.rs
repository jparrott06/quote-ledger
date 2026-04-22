use super::DomainError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineItemState {
    pub line_id: String,
    pub sku: String,
    pub description: String,
    pub quantity: i64,
    pub unit_minor: i64,
}

impl LineItemState {
    pub fn line_total_minor(&self) -> Result<i64, DomainError> {
        let p = i128::from(self.quantity) * i128::from(self.unit_minor);
        i64::try_from(p).map_err(|_| DomainError::IntegerOverflow)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuoteState {
    currency_code: Option<String>,
    jurisdiction_id: Option<String>,
    pub finalized: bool,
    lines: Vec<LineItemState>,
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

    pub fn lines(&self) -> &[LineItemState] {
        &self.lines
    }

    pub fn subtotal_minor(&self) -> Result<i64, DomainError> {
        let mut sum: i128 = 0;
        for line in &self.lines {
            sum += i128::from(line.line_total_minor()?);
        }
        i64::try_from(sum).map_err(|_| DomainError::IntegerOverflow)
    }

    /// Basis points (1/100th of a percent). Stub: US jurisdictions use 800 bps (8%).
    pub fn tax_rate_bps(&self) -> i64 {
        let jur = self.jurisdiction_id();
        if jur == "US" || jur.starts_with("US-") {
            800
        } else {
            0
        }
    }

    pub fn tax_minor(&self) -> Result<i64, DomainError> {
        let sub = self.subtotal_minor()?;
        let bps = self.tax_rate_bps();
        let num = i128::from(sub) * i128::from(bps) / 10_000;
        i64::try_from(num).map_err(|_| DomainError::IntegerOverflow)
    }

    pub fn total_minor(&self) -> Result<i64, DomainError> {
        let sub = self.subtotal_minor()?;
        let tax = self.tax_minor()?;
        let t = i128::from(sub) + i128::from(tax);
        i64::try_from(t).map_err(|_| DomainError::IntegerOverflow)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainEvent {
    QuoteCreated {
        currency_code: String,
        jurisdiction_id: String,
    },
    LineItemAdded {
        line_id: String,
        sku: String,
        description: String,
        quantity: i64,
        unit_minor: i64,
    },
    QuoteFinalized,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainCommand {
    CreateQuote {
        currency_code: String,
        jurisdiction_id: String,
    },
    AddLineItem {
        line_id: String,
        sku: String,
        description: String,
        quantity: i64,
        unit_minor: i64,
    },
    FinalizeQuote,
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
                lines: Vec::new(),
            })
        }
        DomainEvent::LineItemAdded {
            line_id,
            sku,
            description,
            quantity,
            unit_minor,
        } => {
            if !state.is_created() {
                return Err(DomainError::QuoteNotCreated);
            }
            if state.finalized {
                return Err(DomainError::QuoteAlreadyFinalized);
            }
            if state.lines.iter().any(|l| l.line_id == *line_id) {
                return Err(DomainError::DuplicateLineId);
            }
            let mut next = state.clone();
            next.lines.push(LineItemState {
                line_id: line_id.clone(),
                sku: sku.clone(),
                description: description.clone(),
                quantity: *quantity,
                unit_minor: *unit_minor,
            });
            Ok(next)
        }
        DomainEvent::QuoteFinalized => {
            if !state.is_created() {
                return Err(DomainError::QuoteNotCreated);
            }
            if state.finalized {
                return Err(DomainError::QuoteAlreadyFinalized);
            }
            let mut next = state.clone();
            next.finalized = true;
            Ok(next)
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
        DomainCommand::AddLineItem {
            line_id,
            sku,
            description,
            quantity,
            unit_minor,
        } => {
            if !state.is_created() {
                return Err(DomainError::QuoteNotCreated);
            }
            if state.finalized {
                return Err(DomainError::QuoteAlreadyFinalized);
            }
            require_non_blank("line_id", line_id)?;
            require_non_blank("sku", sku)?;
            require_non_blank("description", description)?;
            if *quantity <= 0 {
                return Err(DomainError::InvalidQuantity);
            }
            if *unit_minor < 0 {
                return Err(DomainError::InvalidUnitMinor);
            }
            if state.lines.iter().any(|l| l.line_id == *line_id) {
                return Err(DomainError::DuplicateLineId);
            }
            // Ensure totals math won't overflow before emitting the event.
            let _probe = LineItemState {
                line_id: line_id.clone(),
                sku: sku.clone(),
                description: description.clone(),
                quantity: *quantity,
                unit_minor: *unit_minor,
            }
            .line_total_minor()?;
            let mut probe_state = state.clone();
            probe_state.lines.push(LineItemState {
                line_id: line_id.clone(),
                sku: sku.clone(),
                description: description.clone(),
                quantity: *quantity,
                unit_minor: *unit_minor,
            });
            let _ = probe_state.total_minor()?;

            Ok(vec![DomainEvent::LineItemAdded {
                line_id: line_id.clone(),
                sku: sku.clone(),
                description: description.clone(),
                quantity: *quantity,
                unit_minor: *unit_minor,
            }])
        }
        DomainCommand::FinalizeQuote => {
            if !state.is_created() {
                return Err(DomainError::QuoteNotCreated);
            }
            if state.finalized {
                return Err(DomainError::QuoteAlreadyFinalized);
            }
            if state.lines.is_empty() {
                return Err(DomainError::CannotFinalizeWithoutLines);
            }
            Ok(vec![DomainEvent::QuoteFinalized])
        }
    }
}

fn require_non_blank(field: &'static str, value: &str) -> Result<(), DomainError> {
    if value.trim().is_empty() {
        return Err(DomainError::InvalidField { field });
    }
    Ok(())
}

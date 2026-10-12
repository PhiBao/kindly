use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("currency mismatch: {left} vs {right}")]
    CurrencyMismatch { left: String, right: String },

    #[error("arithmetic overflow while computing {context}")]
    Overflow { context: &'static str },

    #[error("invalid money value: {0}")]
    InvalidMoney(String),

    #[error("invalid percentage: {0} basis points")]
    InvalidPercent(i32),

    #[error("invalid quantity: {0}")]
    InvalidQuantity(String),

    #[error("invalid state transition: {from} -> {to} ({reason})")]
    InvalidTransition {
        from: String,
        to: String,
        reason: String,
    },

    #[error("negotiation round limit reached ({limit})")]
    RoundLimit { limit: u8 },

    #[error("cannot apply payment of {amount} to invoice {invoice}: {reason}")]
    Overpayment {
        invoice: String,
        amount: String,
        reason: String,
    },

    #[error("validation failed: {0}")]
    Validation(String),
}

pub type CoreResult<T> = Result<T, CoreError>;

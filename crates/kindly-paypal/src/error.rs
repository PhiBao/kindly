use thiserror::Error;

#[derive(Debug, Error)]
pub enum PayPalError {
    #[error("paypal transport error: {0}")]
    Transport(String),

    #[error("paypal api error {status} ({issue}): {message}")]
    Api {
        status: u16,
        issue: String,
        message: String,
    },

    #[error("paypal authentication failed: {0}")]
    Auth(String),

    #[error("paypal configuration error: {0}")]
    Config(String),

    #[error("paypal resource not found: {0}")]
    NotFound(String),

    #[error("webhook verification failed: {0}")]
    SignatureInvalid(String),

    #[error("unexpected paypal response shape: {0}")]
    Shape(String),
}

impl PayPalError {
    pub fn is_retryable(&self) -> bool {
        match self {
            PayPalError::Transport(_) => true,
            PayPalError::Api { status, .. } => {
                matches!(*status, 429 | 500 | 502 | 503 | 504)
            }
            _ => false,
        }
    }
}

pub type PayPalResult<T> = Result<T, PayPalError>;

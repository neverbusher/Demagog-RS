use std::fmt;

/// Failure of a user-visible operation.
///
/// Cancellation is its own variant: callers must not guess it from message
/// text, and a cancelled step never turns into a red error line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppError {
    /// The user asked to stop; the payload is the action that was interrupted.
    Cancelled(&'static str),
    /// A real failure: unreadable file, broken archive, invalid data.
    Failed(String),
}

impl AppError {
    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
    }

    pub fn cancelled(action: &'static str) -> Self {
        Self::Cancelled(action)
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled(_))
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled(action) => f.write_str(action),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for AppError {}

impl From<String> for AppError {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

impl From<&str> for AppError {
    fn from(message: &str) -> Self {
        Self::Failed(message.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;

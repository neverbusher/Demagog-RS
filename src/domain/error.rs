use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Io,
    Archive,
    Xml,
    LimitExceeded,
    UnsupportedFormat,
    InvalidData,
    Serialization,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppFailure {
    pub kind: FailureKind,
    pub message: String,
}

/// Failure of a user-visible operation.
///
/// Cancellation is its own variant: callers must not guess it from message
/// text, and a cancelled step never turns into a red error line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppError {
    /// The user asked to stop; the payload is the action that was interrupted.
    Cancelled(&'static str),
    /// A real failure: unreadable file, broken archive, invalid data.
    Failed(AppFailure),
}

impl AppError {
    fn typed_failure(kind: FailureKind, message: impl Into<String>) -> Self {
        Self::Failed(AppFailure {
            kind,
            message: message.into(),
        })
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self::typed_failure(FailureKind::Other, message)
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::typed_failure(FailureKind::Io, message)
    }

    pub fn archive(message: impl Into<String>) -> Self {
        Self::typed_failure(FailureKind::Archive, message)
    }

    pub fn xml(message: impl Into<String>) -> Self {
        Self::typed_failure(FailureKind::Xml, message)
    }

    pub fn limit_exceeded(message: impl Into<String>) -> Self {
        Self::typed_failure(FailureKind::LimitExceeded, message)
    }

    pub fn unsupported_format(message: impl Into<String>) -> Self {
        Self::typed_failure(FailureKind::UnsupportedFormat, message)
    }

    pub fn invalid_data(message: impl Into<String>) -> Self {
        Self::typed_failure(FailureKind::InvalidData, message)
    }

    pub fn serialization(message: impl Into<String>) -> Self {
        Self::typed_failure(FailureKind::Serialization, message)
    }

    pub fn cancelled(action: &'static str) -> Self {
        Self::Cancelled(action)
    }

    pub fn is_cancelled(&self) -> bool {
        self.failure_kind().is_none()
    }

    pub fn failure_kind(&self) -> Option<FailureKind> {
        match self {
            Self::Cancelled(_) => None,
            Self::Failed(failure) => Some(failure.kind),
        }
    }

    pub fn map_failure_message(self, map: impl FnOnce(String) -> String) -> Self {
        match self {
            Self::Cancelled(action) => Self::Cancelled(action),
            Self::Failed(AppFailure { kind, message }) => Self::Failed(AppFailure {
                kind,
                message: map(message),
            }),
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled(action) => f.write_str(action),
            Self::Failed(failure) => f.write_str(&failure.message),
        }
    }
}

impl std::error::Error for AppError {}

impl From<String> for AppError {
    fn from(message: String) -> Self {
        Self::failed(message)
    }
}

impl From<&str> for AppError {
    fn from(message: &str) -> Self {
        Self::failed(message)
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::AppError;
    use crate::domain::{AppFailure, FailureKind};

    #[test]
    fn constructors_preserve_failure_kind_and_exact_message() {
        const MESSAGE: &str = "  Точное сообщение об ошибке\n";
        let cases = [
            (AppError::failed(MESSAGE), FailureKind::Other),
            (AppError::io(MESSAGE), FailureKind::Io),
            (AppError::archive(MESSAGE), FailureKind::Archive),
            (AppError::xml(MESSAGE), FailureKind::Xml),
            (
                AppError::limit_exceeded(MESSAGE),
                FailureKind::LimitExceeded,
            ),
            (
                AppError::unsupported_format(MESSAGE),
                FailureKind::UnsupportedFormat,
            ),
            (AppError::invalid_data(MESSAGE), FailureKind::InvalidData),
            (AppError::serialization(MESSAGE), FailureKind::Serialization),
        ];

        for (error, expected_kind) in cases {
            assert_eq!(error.clone(), error);
            let AppError::Failed(failure) = error else {
                panic!("constructor must produce a failure");
            };
            assert_eq!(failure.kind, expected_kind);
            assert_eq!(failure.message, MESSAGE);
        }
    }

    #[test]
    fn string_conversions_produce_other_failures() {
        let cases = [
            (
                AppError::from(String::from("owned message")),
                "owned message",
            ),
            (AppError::from("borrowed message"), "borrowed message"),
        ];

        for (error, expected_message) in cases {
            let AppError::Failed(failure) = error else {
                panic!("string conversion must produce a failure");
            };
            assert_eq!(failure.kind, FailureKind::Other);
            assert_eq!(failure.message, expected_message);
        }
    }

    #[test]
    fn display_outputs_exact_failure_message_and_cancelled_action() {
        let failed = AppError::Failed(AppFailure {
            kind: FailureKind::Serialization,
            message: "  Текст ошибки\n".to_string(),
        });
        let cancelled = AppError::cancelled("Действие отменено");

        assert_eq!(failed.to_string(), "  Текст ошибки\n");
        assert_eq!(cancelled.to_string(), "Действие отменено");
    }

    #[test]
    fn failure_kind_distinguishes_cancelled_from_failed() {
        let cancelled = AppError::cancelled("stop");
        let failed = AppError::io("read failed");

        assert!(cancelled.is_cancelled());
        assert_eq!(cancelled.failure_kind(), None);
        assert!(!failed.is_cancelled());
        assert_eq!(failed.failure_kind(), Some(FailureKind::Io));
    }

    #[test]
    fn map_failure_message_preserves_kind_and_changes_only_message() {
        for kind in [
            FailureKind::Io,
            FailureKind::Archive,
            FailureKind::Xml,
            FailureKind::LimitExceeded,
            FailureKind::UnsupportedFormat,
            FailureKind::InvalidData,
            FailureKind::Serialization,
            FailureKind::Other,
        ] {
            let original = AppError::Failed(AppFailure {
                kind,
                message: "original".to_string(),
            });

            let mapped = original
                .clone()
                .map_failure_message(|message| format!("prefix: {message}"));

            let AppError::Failed(failure) = mapped else {
                panic!("mapped failure must remain a failure");
            };
            assert_eq!(failure.kind, kind);
            assert_eq!(failure.message, "prefix: original");
        }
    }

    #[test]
    fn map_failure_message_leaves_cancelled_unchanged() {
        let original = AppError::cancelled("stop loading");

        let mapped = original
            .clone()
            .map_failure_message(|_| panic!("cancelled action must not be mapped"));

        assert_eq!(mapped, original);
        assert!(mapped.is_cancelled());
    }
}

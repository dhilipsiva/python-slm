use serde::Serialize;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    Internal,
    Usage,
    Integrity,
    Environment,
    Gate,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProductError {
    pub schema: &'static str,
    pub code: String,
    pub category: ErrorCategory,
    pub message: String,
    pub remediation: String,
}

impl ProductError {
    pub fn new(
        code: impl Into<String>,
        category: ErrorCategory,
        message: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Self {
        Self {
            schema: "python-slm-error-v1",
            code: code.into(),
            category,
            message: message.into(),
            remediation: remediation.into(),
        }
    }

    pub fn usage(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(
            code,
            ErrorCategory::Usage,
            message,
            "Use python-slm --help and provide an explicit versioned configuration.",
        )
    }

    pub fn gate(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(
            code,
            ErrorCategory::Gate,
            message,
            "Complete the owning implementation phase; no legacy fallback is available.",
        )
    }

    pub fn internal(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(
            code,
            ErrorCategory::Internal,
            message,
            "Inspect the Rust implementation and retry after correction.",
        )
    }

    pub fn exit_code(&self) -> i32 {
        match self.category {
            ErrorCategory::Internal => 1,
            ErrorCategory::Usage => 2,
            ErrorCategory::Integrity => 3,
            ErrorCategory::Environment => 4,
            ErrorCategory::Gate => 5,
        }
    }
}

impl fmt::Display for ProductError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for ProductError {}

pub type Result<T> = std::result::Result<T, ProductError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_categories_are_stable() {
        let cases = [
            (ErrorCategory::Internal, 1),
            (ErrorCategory::Usage, 2),
            (ErrorCategory::Integrity, 3),
            (ErrorCategory::Environment, 4),
            (ErrorCategory::Gate, 5),
        ];
        for (category, expected) in cases {
            assert_eq!(
                ProductError::new("TEST", category, "test", "test").exit_code(),
                expected
            );
        }
    }
}

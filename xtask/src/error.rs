use serde::Serialize;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Internal,
    Usage,
    Integrity,
    Environment,
    Gate,
}

#[derive(Debug, Serialize)]
pub struct XtaskError {
    pub schema: &'static str,
    pub code: String,
    pub category: Category,
    pub message: String,
    pub remediation: String,
}

impl XtaskError {
    pub fn new(
        code: impl Into<String>,
        category: Category,
        message: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Self {
        Self {
            schema: "python-slm-xtask-error-v1",
            code: code.into(),
            category,
            message: message.into(),
            remediation: remediation.into(),
        }
    }

    pub fn integrity(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(
            code,
            Category::Integrity,
            message,
            "Restore the approved immutable bytes and retry from the repository root.",
        )
    }

    pub fn environment(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(
            code,
            Category::Environment,
            message,
            "Correct the local environment without rewriting historical evidence, then retry.",
        )
    }

    pub fn gate(
        code: &'static str,
        message: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Self {
        Self::new(code, Category::Gate, message, remediation)
    }

    pub fn exit_code(&self) -> i32 {
        match self.category {
            Category::Internal => 1,
            Category::Usage => 2,
            Category::Integrity => 3,
            Category::Environment => 4,
            Category::Gate => 5,
        }
    }
}

impl fmt::Display for XtaskError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for XtaskError {}

pub type Result<T> = std::result::Result<T, XtaskError>;

pub trait IoContext<T> {
    fn io_context(self, code: &'static str, message: impl Into<String>) -> Result<T>;
}

impl<T> IoContext<T> for std::io::Result<T> {
    fn io_context(self, code: &'static str, message: impl Into<String>) -> Result<T> {
        let message = message.into();
        self.map_err(|error| XtaskError::environment(code, format!("{message}: {error}")))
    }
}

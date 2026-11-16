//! Typed, fail-closed errors for the `editorial_plan.v2` contract.
//!
//! Go wraps these strings as
//! `overlay plan: BrainRust editorial compiler is required: ...`, so every
//! variant must say which field or section is wrong. There is deliberately no
//! "default to an empty plan" variant: a contract violation is a caller bug,
//! never a no-op.

use std::fmt;

/// One typed contract failure. `Display` is the stable, human-readable reason
/// the worker sends back in `{"ok":false,"error":"..."}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorialPlanError {
    /// The envelope carried a schema version this build does not implement.
    UnsupportedSchemaVersion { found: String, expected: String },
    /// A required section is absent. The single-call planner may not guess it.
    MissingSection(&'static str),
    /// A required field is empty or structurally invalid.
    InvalidField { field: String, detail: String },
    /// The request bytes were not valid JSON for the declared envelope.
    MalformedRequest(String),
}

impl EditorialPlanError {
    /// Builds an [`EditorialPlanError::InvalidField`] with a stable path.
    pub fn invalid(field: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::InvalidField {
            field: field.into(),
            detail: detail.into(),
        }
    }
}

impl fmt::Display for EditorialPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchemaVersion { found, expected } => write!(
                formatter,
                "editorial plan: unsupported schema_version {found:?}, want {expected:?}"
            ),
            Self::MissingSection(section) => {
                write!(
                    formatter,
                    "editorial plan: missing required section {section:?}"
                )
            }
            Self::InvalidField { field, detail } => {
                write!(formatter, "editorial plan: invalid {field}: {detail}")
            }
            Self::MalformedRequest(detail) => {
                write!(formatter, "editorial plan: malformed request: {detail}")
            }
        }
    }
}

impl std::error::Error for EditorialPlanError {}

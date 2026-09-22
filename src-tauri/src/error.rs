//! Structured errors for the whole backend.
//!
//! Every fallible public function returns [`AppResult`]. Errors carry three
//! separate things so that each audience gets what it needs:
//!
//! * a **stable machine code** (`code`) the frontend can branch on,
//! * a **user-facing message** that is safe and useful to display verbatim,
//! * an optional **diagnostic detail** (paths, OS error text) for logs.
//!
//! The type serializes to `{ "code": ..., "message": ..., "detail": ... }`,
//! which is exactly what Tauri sends to the webview when a command fails.

use serde::ser::{Serialize, SerializeStruct, Serializer};

/// Result alias used throughout the crate.
pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// A referenced resource does not exist (file, book, chapter).
    #[error("{what} not found: {id}")]
    NotFound { what: String, id: String },

    /// Input was rejected by validation before any work happened.
    #[error("{reason}")]
    InvalidInput { reason: String },

    /// A Windows-only capability was requested on another platform. The variant
    /// only exists where it can occur, so Windows builds stay free of dead code.
    #[cfg(not(windows))]
    #[error("{feature} is only available on Windows")]
    UnsupportedPlatform { feature: String },

    /// Importing a file failed (copy, size limit, duplicate, ...).
    #[error("Could not import {file}: {reason}")]
    Import { file: String, reason: String },

    /// A document could not be parsed.
    #[error("Could not read {file}: {detail}")]
    Document { file: String, detail: String },

    /// The on-disk library could not be read or written.
    #[error("Could not {operation} the library: {detail}")]
    Storage { operation: String, detail: String },

    /// Speech playback failed.
    #[error("Playback failed: {detail}")]
    Playback { detail: String },

    /// Another playback is already running; the caller must stop it first.
    #[error("Finish the current playback first")]
    Busy,

    /// Bugs, invariant violations, and anything not otherwise classified.
    #[error("Something went wrong")]
    Internal { detail: String },
}

impl AppError {
    /// Stable identifier the frontend may branch on. Never change these
    /// strings; add new variants instead.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "not_found",
            Self::InvalidInput { .. } => "invalid_input",
            #[cfg(not(windows))]
            Self::UnsupportedPlatform { .. } => "unsupported_platform",
            Self::Import { .. } => "import_failed",
            Self::Document { .. } => "document_parse_failed",
            Self::Storage { .. } => "storage_failed",
            Self::Playback { .. } => "playback_failed",
            Self::Busy => "busy",
            Self::Internal { .. } => "internal_error",
        }
    }

    /// Safe-to-display text. Must never contain a bare OS error string alone,
    /// but may include the context the user needs to act on.
    pub fn user_message(&self) -> String {
        match self {
            Self::NotFound { what, id } => format!("{what} not found: {id}"),
            Self::InvalidInput { reason } => reason.clone(),
            #[cfg(not(windows))]
            Self::UnsupportedPlatform { feature } => {
                format!("{feature} is only available on Windows")
            }
            Self::Import { file, reason } => format!("Could not import {file}: {reason}"),
            Self::Document { file, detail } => format!("Could not read {file}: {detail}"),
            Self::Storage { operation, detail } => {
                format!("Could not {operation} the library. {detail}")
            }
            Self::Playback { detail } => format!("Playback failed: {detail}"),
            Self::Busy => "Finish the current playback first".to_string(),
            Self::Internal { .. } => {
                "Something went wrong. See the log file for details.".to_string()
            }
        }
    }

    /// Raw diagnostic information for logs. `None` when the user message
    /// already carries everything there is to know.
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::Storage { detail, .. }
            | Self::Document { detail, .. }
            | Self::Playback { detail }
            | Self::Internal { detail } => Some(detail.clone()),
            _ => None,
        }
    }

    // ── Constructors ────────────────────────────────────────────────
    // Free functions/From impls would be noisier at call sites; these read
    // well and keep `impl Into<String>` conversions implicit.

    pub fn not_found(what: impl Into<String>, id: impl Into<String>) -> Self {
        Self::NotFound {
            what: what.into(),
            id: id.into(),
        }
    }

    pub fn invalid_input(reason: impl Into<String>) -> Self {
        Self::InvalidInput {
            reason: reason.into(),
        }
    }

    #[cfg(not(windows))]
    pub fn unsupported_platform(feature: impl Into<String>) -> Self {
        Self::UnsupportedPlatform {
            feature: feature.into(),
        }
    }

    pub fn import(file: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Import {
            file: file.into(),
            reason: reason.into(),
        }
    }

    pub fn document(file: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::Document {
            file: file.into(),
            detail: detail.into(),
        }
    }

    pub fn storage(operation: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::Storage {
            operation: operation.into(),
            detail: detail.into(),
        }
    }

    pub fn playback(detail: impl Into<String>) -> Self {
        Self::Playback {
            detail: detail.into(),
        }
    }

    pub fn internal(detail: impl Into<String>) -> Self {
        Self::Internal {
            detail: detail.into(),
        }
    }
}

/// Legacy bridge: code paths that have not been migrated yet still produce
/// `String` errors. Those become internal errors, which is honest — their text
/// was never designed for users.
impl From<String> for AppError {
    fn from(value: String) -> Self {
        Self::Internal { detail: value }
    }
}

impl From<&str> for AppError {
    fn from(value: &str) -> Self {
        Self::Internal {
            detail: value.to_string(),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(value: std::io::Error) -> Self {
        Self::Internal {
            detail: value.to_string(),
        }
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("AppError", 3)?;
        state.serialize_field("code", self.code())?;
        state.serialize_field("message", &self.user_message())?;
        state.serialize_field("detail", &self.detail())?;
        state.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_and_distinct() {
        let errors = [
            AppError::not_found("Book", "abc"),
            AppError::invalid_input("Only PDF and EPUB files are supported"),
            #[cfg(not(windows))]
            AppError::unsupported_platform("Text to speech"),
            AppError::import("novel.epub", "file too large"),
            AppError::document("novel.pdf", "no extractable text"),
            AppError::storage("write", "disk full"),
            AppError::playback("voice not initialized"),
            AppError::Busy,
            AppError::internal("unreachable state"),
        ];

        let mut codes: Vec<&str> = errors.iter().map(AppError::code).collect();
        let expected = [
            "not_found",
            "invalid_input",
            #[cfg(not(windows))]
            "unsupported_platform",
            "import_failed",
            "document_parse_failed",
            "storage_failed",
            "playback_failed",
            "busy",
            "internal_error",
        ];
        assert_eq!(codes, expected);

        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), expected.len(), "codes must be unique");
    }

    #[test]
    fn user_messages_never_leak_the_internal_sentinel() {
        for error in [
            AppError::invalid_input("Only PDF and EPUB files are supported"),
            AppError::Busy,
            AppError::NotFound {
                what: "Book".into(),
                id: "abc".into(),
            },
        ] {
            assert!(!error.user_message().is_empty());
            assert!(!error.user_message().contains("Result::unwrap"));
        }
    }

    #[test]
    fn internal_errors_hide_detail_from_the_user_but_keep_it_for_logs() {
        let error = AppError::internal("index out of bounds at books[7]");
        assert_eq!(
            error.user_message(),
            "Something went wrong. See the log file for details."
        );
        assert_eq!(
            error.detail().as_deref(),
            Some("index out of bounds at books[7]")
        );
    }

    #[test]
    fn serializes_to_the_shape_the_frontend_expects() {
        let value = serde_json::to_value(AppError::storage("write", "disk full"))
            .expect("serialize AppError");

        assert_eq!(value["code"], "storage_failed");
        assert_eq!(value["message"], "Could not write the library. disk full");
        assert_eq!(value["detail"], "disk full");
    }

    #[test]
    fn serde_json_null_is_reported_for_errors_without_detail() {
        let value = serde_json::to_value(AppError::Busy).expect("serialize AppError");
        assert_eq!(value["code"], "busy");
        assert!(value["detail"].is_null());
    }

    #[test]
    fn legacy_string_errors_become_internal_errors() {
        let error: AppError = "Cannot extract text".to_string().into();
        assert_eq!(error.code(), "internal_error");
        assert_eq!(error.detail().as_deref(), Some("Cannot extract text"));
    }
}

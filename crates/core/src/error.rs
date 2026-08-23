use serde::{Deserialize, Serialize};

/// Stable machine-readable error codes shared across MCP and UI surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    AccessPaused,
    WorkspaceNotFound,
    ReadDisabled,
    WriteDisabled,
    PathOutsideWorkspace,
    SensitiveFileBlocked,
    FileNotFound,
    FileExists,
    InvalidPatch,
    FileChangedSinceRead,
    ResultTooLarge,
    IoError,
    InvalidArgument,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AccessPaused => "ACCESS_PAUSED",
            Self::WorkspaceNotFound => "WORKSPACE_NOT_FOUND",
            Self::ReadDisabled => "READ_DISABLED",
            Self::WriteDisabled => "WRITE_DISABLED",
            Self::PathOutsideWorkspace => "PATH_OUTSIDE_WORKSPACE",
            Self::SensitiveFileBlocked => "SENSITIVE_FILE_BLOCKED",
            Self::FileNotFound => "FILE_NOT_FOUND",
            Self::FileExists => "FILE_EXISTS",
            Self::InvalidPatch => "INVALID_PATCH",
            Self::FileChangedSinceRead => "FILE_CHANGED_SINCE_READ",
            Self::ResultTooLarge => "RESULT_TOO_LARGE",
            Self::IoError => "IO_ERROR",
            Self::InvalidArgument => "INVALID_ARGUMENT",
        }
    }
}

/// Core error carrying a stable code, human-readable message, and optional detail.
#[derive(Debug, Clone)]
pub struct CoreError {
    pub code: ErrorCode,
    pub message: String,
    pub detail: Option<serde_json::Value>,
}

impl CoreError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: serde_json::Value) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn io(e: std::io::Error) -> Self {
        Self::new(ErrorCode::IoError, e.to_string())
    }
}

impl From<rusqlite::Error> for CoreError {
    fn from(e: rusqlite::Error) -> Self {
        Self::new(ErrorCode::IoError, e.to_string())
    }
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for CoreError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_stable_strings() {
        assert_eq!(ErrorCode::AccessPaused.as_str(), "ACCESS_PAUSED");
        assert_eq!(ErrorCode::WorkspaceNotFound.as_str(), "WORKSPACE_NOT_FOUND");
        assert_eq!(ErrorCode::ReadDisabled.as_str(), "READ_DISABLED");
        assert_eq!(ErrorCode::WriteDisabled.as_str(), "WRITE_DISABLED");
        assert_eq!(
            ErrorCode::PathOutsideWorkspace.as_str(),
            "PATH_OUTSIDE_WORKSPACE"
        );
        assert_eq!(
            ErrorCode::SensitiveFileBlocked.as_str(),
            "SENSITIVE_FILE_BLOCKED"
        );
        assert_eq!(ErrorCode::FileNotFound.as_str(), "FILE_NOT_FOUND");
        assert_eq!(ErrorCode::FileExists.as_str(), "FILE_EXISTS");
        assert_eq!(ErrorCode::InvalidPatch.as_str(), "INVALID_PATCH");
        assert_eq!(
            ErrorCode::FileChangedSinceRead.as_str(),
            "FILE_CHANGED_SINCE_READ"
        );
        assert_eq!(ErrorCode::ResultTooLarge.as_str(), "RESULT_TOO_LARGE");
        assert_eq!(ErrorCode::IoError.as_str(), "IO_ERROR");
        assert_eq!(ErrorCode::InvalidArgument.as_str(), "INVALID_ARGUMENT");
    }

    #[test]
    fn error_displays_as_code_message() {
        let e = CoreError::new(ErrorCode::FileNotFound, "missing.txt not found");
        assert_eq!(e.to_string(), "FILE_NOT_FOUND: missing.txt not found");
    }

    #[test]
    fn io_error_maps_to_io_error_code() {
        let e = CoreError::io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "denied",
        ));
        assert_eq!(e.code, ErrorCode::IoError);
    }
}

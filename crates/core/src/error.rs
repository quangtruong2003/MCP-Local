use serde::{Deserialize, Serialize};

/// Stable machine-readable error codes shared across MCP and UI surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    AccessPaused,
    WorkspaceNotFound,
    ReadDisabled,
    WriteDisabled,
    ExecuteDisabled,
    CommandNotAllowed,
    PowershellDisabled,
    InvalidWorkingDirectory,
    CommandTimeout,
    ExecutionFailed,
    PathOutsideWorkspace,
    SensitiveFileBlocked,
    FileNotFound,
    FileExists,
    InvalidPatch,
    FileChangedSinceRead,
    ResultTooLarge,
    IoError,
    InvalidArgument,
    ScreenCaptureDisabled,
    ScreenCaptureUnsupported,
    CaptureTargetNotFound,
    ScreenCaptureFailed,
    SettingsElevationRequiresUser,
    BrowserAutomationDisabled,
    BrowserNotRunning,
    BrowserStartFailed,
    BrowserProcessLost,
    BrowserTargetNotFound,
    BrowserStaleRef,
    BrowserNavigationTimeout,
    BrowserActionTimeout,
    BrowserFeatureUnsupported,
    BrowserFeatureUnsupportedInMode,
    BrowserInvalidViewport,
    BrowserUploadDenied,
    BrowserDownloadNotFound,
    BrowserDownloadDenied,
    BrowserArtifactFailed,
    BrowserProtocolError,
    BrowserWslUnavailable,
    BrowserWslRebootRequired,
    BrowserWslDistroConflict,
    BrowserRuntimeNotReady,
    BrowserRuntimeDownloadFailed,
    BrowserRuntimeIntegrityFailed,
    BrowserRuntimeVersionMismatch,
    BrowserRuntimeProvisionFailed,
    BrowserRuntimeRepairRequired,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AccessPaused => "ACCESS_PAUSED",
            Self::WorkspaceNotFound => "WORKSPACE_NOT_FOUND",
            Self::ReadDisabled => "READ_DISABLED",
            Self::WriteDisabled => "WRITE_DISABLED",
            Self::ExecuteDisabled => "EXECUTE_DISABLED",
            Self::CommandNotAllowed => "COMMAND_NOT_ALLOWED",
            Self::PowershellDisabled => "POWERSHELL_DISABLED",
            Self::InvalidWorkingDirectory => "INVALID_WORKING_DIRECTORY",
            Self::CommandTimeout => "COMMAND_TIMEOUT",
            Self::ExecutionFailed => "EXECUTION_FAILED",
            Self::PathOutsideWorkspace => "PATH_OUTSIDE_WORKSPACE",
            Self::SensitiveFileBlocked => "SENSITIVE_FILE_BLOCKED",
            Self::FileNotFound => "FILE_NOT_FOUND",
            Self::FileExists => "FILE_EXISTS",
            Self::InvalidPatch => "INVALID_PATCH",
            Self::FileChangedSinceRead => "FILE_CHANGED_SINCE_READ",
            Self::ResultTooLarge => "RESULT_TOO_LARGE",
            Self::IoError => "IO_ERROR",
            Self::InvalidArgument => "INVALID_ARGUMENT",
            Self::ScreenCaptureDisabled => "SCREEN_CAPTURE_DISABLED",
            Self::ScreenCaptureUnsupported => "SCREEN_CAPTURE_UNSUPPORTED",
            Self::CaptureTargetNotFound => "CAPTURE_TARGET_NOT_FOUND",
            Self::ScreenCaptureFailed => "SCREEN_CAPTURE_FAILED",
            Self::SettingsElevationRequiresUser => "SETTINGS_ELEVATION_REQUIRES_USER",
            Self::BrowserAutomationDisabled => "BROWSER_AUTOMATION_DISABLED",
            Self::BrowserNotRunning => "BROWSER_NOT_RUNNING",
            Self::BrowserStartFailed => "BROWSER_START_FAILED",
            Self::BrowserProcessLost => "BROWSER_PROCESS_LOST",
            Self::BrowserTargetNotFound => "BROWSER_TARGET_NOT_FOUND",
            Self::BrowserStaleRef => "BROWSER_STALE_REF",
            Self::BrowserNavigationTimeout => "BROWSER_NAVIGATION_TIMEOUT",
            Self::BrowserActionTimeout => "BROWSER_ACTION_TIMEOUT",
            Self::BrowserFeatureUnsupported => "BROWSER_FEATURE_UNSUPPORTED",
            Self::BrowserFeatureUnsupportedInMode => "BROWSER_FEATURE_UNSUPPORTED_IN_MODE",
            Self::BrowserInvalidViewport => "BROWSER_INVALID_VIEWPORT",
            Self::BrowserUploadDenied => "BROWSER_UPLOAD_DENIED",
            Self::BrowserDownloadNotFound => "BROWSER_DOWNLOAD_NOT_FOUND",
            Self::BrowserDownloadDenied => "BROWSER_DOWNLOAD_DENIED",
            Self::BrowserArtifactFailed => "BROWSER_ARTIFACT_FAILED",
            Self::BrowserProtocolError => "BROWSER_PROTOCOL_ERROR",
            Self::BrowserWslUnavailable => "BROWSER_WSL_UNAVAILABLE",
            Self::BrowserWslRebootRequired => "BROWSER_WSL_REBOOT_REQUIRED",
            Self::BrowserWslDistroConflict => "BROWSER_WSL_DISTRO_CONFLICT",
            Self::BrowserRuntimeNotReady => "BROWSER_RUNTIME_NOT_READY",
            Self::BrowserRuntimeDownloadFailed => "BROWSER_RUNTIME_DOWNLOAD_FAILED",
            Self::BrowserRuntimeIntegrityFailed => "BROWSER_RUNTIME_INTEGRITY_FAILED",
            Self::BrowserRuntimeVersionMismatch => "BROWSER_RUNTIME_VERSION_MISMATCH",
            Self::BrowserRuntimeProvisionFailed => "BROWSER_RUNTIME_PROVISION_FAILED",
            Self::BrowserRuntimeRepairRequired => "BROWSER_RUNTIME_REPAIR_REQUIRED",
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
        assert_eq!(ErrorCode::ExecuteDisabled.as_str(), "EXECUTE_DISABLED");
        assert_eq!(ErrorCode::CommandNotAllowed.as_str(), "COMMAND_NOT_ALLOWED");
        assert_eq!(
            ErrorCode::PowershellDisabled.as_str(),
            "POWERSHELL_DISABLED"
        );
        assert_eq!(
            ErrorCode::InvalidWorkingDirectory.as_str(),
            "INVALID_WORKING_DIRECTORY"
        );
        assert_eq!(ErrorCode::CommandTimeout.as_str(), "COMMAND_TIMEOUT");
        assert_eq!(ErrorCode::ExecutionFailed.as_str(), "EXECUTION_FAILED");
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
        assert_eq!(
            ErrorCode::ScreenCaptureDisabled.as_str(),
            "SCREEN_CAPTURE_DISABLED"
        );
        assert_eq!(
            ErrorCode::ScreenCaptureUnsupported.as_str(),
            "SCREEN_CAPTURE_UNSUPPORTED"
        );
        assert_eq!(
            ErrorCode::CaptureTargetNotFound.as_str(),
            "CAPTURE_TARGET_NOT_FOUND"
        );
        assert_eq!(
            ErrorCode::ScreenCaptureFailed.as_str(),
            "SCREEN_CAPTURE_FAILED"
        );
        assert_eq!(
            ErrorCode::SettingsElevationRequiresUser.as_str(),
            "SETTINGS_ELEVATION_REQUIRES_USER"
        );
        assert_eq!(
            ErrorCode::BrowserAutomationDisabled.as_str(),
            "BROWSER_AUTOMATION_DISABLED"
        );
        assert_eq!(ErrorCode::BrowserNotRunning.as_str(), "BROWSER_NOT_RUNNING");
        assert_eq!(
            ErrorCode::BrowserStartFailed.as_str(),
            "BROWSER_START_FAILED"
        );
        assert_eq!(
            ErrorCode::BrowserProcessLost.as_str(),
            "BROWSER_PROCESS_LOST"
        );
        assert_eq!(
            ErrorCode::BrowserTargetNotFound.as_str(),
            "BROWSER_TARGET_NOT_FOUND"
        );
        assert_eq!(ErrorCode::BrowserStaleRef.as_str(), "BROWSER_STALE_REF");
        assert_eq!(
            ErrorCode::BrowserNavigationTimeout.as_str(),
            "BROWSER_NAVIGATION_TIMEOUT"
        );
        assert_eq!(
            ErrorCode::BrowserActionTimeout.as_str(),
            "BROWSER_ACTION_TIMEOUT"
        );
        assert_eq!(
            ErrorCode::BrowserFeatureUnsupported.as_str(),
            "BROWSER_FEATURE_UNSUPPORTED"
        );
        assert_eq!(
            ErrorCode::BrowserFeatureUnsupportedInMode.as_str(),
            "BROWSER_FEATURE_UNSUPPORTED_IN_MODE"
        );
        assert_eq!(
            ErrorCode::BrowserInvalidViewport.as_str(),
            "BROWSER_INVALID_VIEWPORT"
        );
        assert_eq!(
            ErrorCode::BrowserUploadDenied.as_str(),
            "BROWSER_UPLOAD_DENIED"
        );
        assert_eq!(
            ErrorCode::BrowserDownloadNotFound.as_str(),
            "BROWSER_DOWNLOAD_NOT_FOUND"
        );
        assert_eq!(
            ErrorCode::BrowserDownloadDenied.as_str(),
            "BROWSER_DOWNLOAD_DENIED"
        );
        assert_eq!(
            ErrorCode::BrowserArtifactFailed.as_str(),
            "BROWSER_ARTIFACT_FAILED"
        );
        assert_eq!(
            ErrorCode::BrowserProtocolError.as_str(),
            "BROWSER_PROTOCOL_ERROR"
        );
        assert_eq!(
            ErrorCode::BrowserWslUnavailable.as_str(),
            "BROWSER_WSL_UNAVAILABLE"
        );
        assert_eq!(
            ErrorCode::BrowserWslRebootRequired.as_str(),
            "BROWSER_WSL_REBOOT_REQUIRED"
        );
        assert_eq!(
            ErrorCode::BrowserWslDistroConflict.as_str(),
            "BROWSER_WSL_DISTRO_CONFLICT"
        );
        assert_eq!(
            ErrorCode::BrowserRuntimeNotReady.as_str(),
            "BROWSER_RUNTIME_NOT_READY"
        );
        assert_eq!(
            ErrorCode::BrowserRuntimeDownloadFailed.as_str(),
            "BROWSER_RUNTIME_DOWNLOAD_FAILED"
        );
        assert_eq!(
            ErrorCode::BrowserRuntimeIntegrityFailed.as_str(),
            "BROWSER_RUNTIME_INTEGRITY_FAILED"
        );
        assert_eq!(
            ErrorCode::BrowserRuntimeVersionMismatch.as_str(),
            "BROWSER_RUNTIME_VERSION_MISMATCH"
        );
        assert_eq!(
            ErrorCode::BrowserRuntimeProvisionFailed.as_str(),
            "BROWSER_RUNTIME_PROVISION_FAILED"
        );
        assert_eq!(
            ErrorCode::BrowserRuntimeRepairRequired.as_str(),
            "BROWSER_RUNTIME_REPAIR_REQUIRED"
        );
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

    #[test]
    fn with_detail_attaches_payload_and_preserves_code() {
        let e = CoreError::new(ErrorCode::PathOutsideWorkspace, "escaped")
            .with_detail(serde_json::json!({ "requested": "../x" }));
        assert_eq!(e.code, ErrorCode::PathOutsideWorkspace);
        assert_eq!(e.detail.as_ref().unwrap()["requested"], "../x");
        // No detail by default.
        assert!(CoreError::new(ErrorCode::IoError, "plain").detail.is_none());
    }
}

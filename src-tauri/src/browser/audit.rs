use lc_core::{
    error::{CoreError, ErrorCode},
    CoreServices,
};

fn decision(error: Option<&CoreError>) -> &'static str {
    match error {
        None => "allow",
        Some(error)
            if matches!(
                error.code,
                ErrorCode::AccessPaused
                    | ErrorCode::BrowserAutomationDisabled
                    | ErrorCode::BrowserUploadDenied
                    | ErrorCode::BrowserDownloadDenied
            ) =>
        {
            "deny"
        }
        Some(_) => "error",
    }
}

/// Records exactly one fail-closed audit row for a public browser MCP call.
/// Callers must pass only already-redacted metadata; this helper deliberately
/// never accepts raw page content, form values, cookies, response bodies, or
/// arbitrary upstream tool output.
pub(crate) fn record_browser_call(
    core: &CoreServices,
    op: &str,
    error: Option<&CoreError>,
    detail: Option<serde_json::Value>,
) -> Result<(), CoreError> {
    lc_core::audit::record_simple(
        &core.db,
        "mcp",
        Some(op),
        None,
        op,
        decision(error),
        error.map(|error| error.code.as_str()),
        None,
        detail,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_controlled_browser_denials_are_classified_as_deny() {
        for code in [
            ErrorCode::AccessPaused,
            ErrorCode::BrowserAutomationDisabled,
            ErrorCode::BrowserUploadDenied,
            ErrorCode::BrowserDownloadDenied,
        ] {
            assert_eq!(decision(Some(&CoreError::new(code, "denied"))), "deny");
        }
        assert_eq!(decision(None), "allow");
        assert_eq!(
            decision(Some(&CoreError::new(ErrorCode::InvalidArgument, "bad"))),
            "error"
        );
    }
}

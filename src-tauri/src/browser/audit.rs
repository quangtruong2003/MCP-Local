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
                ErrorCode::AccessPaused | ErrorCode::BrowserAutomationDisabled
            ) =>
        {
            "deny"
        }
        Some(_) => "error",
    }
}

/// The result class of one native `browser_lightpanda_*` MCP call. Used only
/// to derive the audit decision/error-code; raw upstream output, page content,
/// cookies, env values, and upstream messages never pass through here.
pub(crate) enum NativeCallOutcome<'a> {
    /// Forwarded a native `CallToolResult` unchanged; `is_error` mirrors the
    /// upstream `isError` flag.
    Native { is_error: bool },
    /// A valid upstream JSON-RPC tool error. The numeric code is stored in the
    /// audit detail by the caller; the raw message is never audited.
    UpstreamRpc,
    /// A Local Workspaces error produced before/without a native result.
    Local(&'a CoreError),
}

/// Records exactly one fail-closed audit row for a public native browser MCP
/// call. `detail` must already be content-free (see
/// `mcp::safe_native_audit_detail`); this helper never accepts raw page
/// content, form values, cookies, env values, scripts, or upstream output.
pub(crate) fn record_native_browser_call(
    core: &CoreServices,
    op: &str,
    outcome: NativeCallOutcome,
    detail: Option<serde_json::Value>,
) -> Result<(), CoreError> {
    let (decision, error_code): (&str, Option<&str>) = match outcome {
        NativeCallOutcome::Native { is_error: false } => ("allow", None),
        NativeCallOutcome::Native { is_error: true } => ("error", Some("NATIVE_TOOL_ERROR")),
        NativeCallOutcome::UpstreamRpc => ("error", Some("UPSTREAM_ERROR")),
        NativeCallOutcome::Local(error) => (decision(Some(error)), Some(error.code.as_str())),
    };
    lc_core::audit::record_simple(
        &core.db,
        "mcp",
        Some(op),
        None,
        op,
        decision,
        error_code,
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

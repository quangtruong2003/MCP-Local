use lc_core::error::{CoreError, ErrorCode};

use super::types::{
    DEFAULT_ACTION_TIMEOUT_MS, DEFAULT_EVENT_LIMIT, DEFAULT_NAVIGATION_TIMEOUT_MS, MAX_BATCH_STEPS,
    MAX_EVENT_LIMIT, MAX_TOOL_TIMEOUT_MS, MAX_VIEWPORT_EDGE,
};

/// Native tools whose `url` argument causes browser navigation and must
/// therefore pass the Local Workspaces navigation policy before forwarding.
/// `getCookies.url` only scopes cookie output and is not navigation.
pub const NATIVE_NAVIGATION_URL_TOOLS: &[&str] = &[
    "goto",
    "markdown",
    "html",
    "links",
    "evaluate",
    "tree",
    "interactiveElements",
    "structuredData",
    "detectForms",
];

pub fn validate_navigation_url(input: &str) -> Result<url::Url, CoreError> {
    let parsed = url::Url::parse(input).map_err(|_| {
        CoreError::new(
            ErrorCode::InvalidArgument,
            "browser URL must be an absolute http(s) URL",
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(CoreError::new(
            ErrorCode::InvalidArgument,
            "browser navigation only allows http and https URLs",
        ));
    }
    Ok(parsed)
}

pub fn validate_viewport(width: u32, height: u32) -> Result<(), CoreError> {
    if (1..=MAX_VIEWPORT_EDGE).contains(&width) && (1..=MAX_VIEWPORT_EDGE).contains(&height) {
        Ok(())
    } else {
        Err(CoreError::new(
            ErrorCode::BrowserInvalidViewport,
            format!("viewport dimensions must be between 1 and {MAX_VIEWPORT_EDGE} pixels"),
        ))
    }
}

pub fn validate_timeout(requested: Option<u64>, navigation: bool) -> Result<u64, CoreError> {
    let default = if navigation {
        DEFAULT_NAVIGATION_TIMEOUT_MS
    } else {
        DEFAULT_ACTION_TIMEOUT_MS
    };
    let value = requested.unwrap_or(default);
    if value == 0 || value > MAX_TOOL_TIMEOUT_MS {
        return Err(CoreError::new(
            ErrorCode::InvalidArgument,
            format!("browser timeout must be between 1 and {MAX_TOOL_TIMEOUT_MS} ms"),
        ));
    }
    Ok(value)
}

pub fn bounded_event_limit(requested: Option<usize>) -> Result<usize, CoreError> {
    let value = requested.unwrap_or(DEFAULT_EVENT_LIMIT);
    if value == 0 || value > MAX_EVENT_LIMIT {
        return Err(CoreError::new(
            ErrorCode::InvalidArgument,
            format!("browser event limit must be between 1 and {MAX_EVENT_LIMIT}"),
        ));
    }
    Ok(value)
}

pub fn validate_batch_step_count(count: usize) -> Result<(), CoreError> {
    if count == 0 || count > MAX_BATCH_STEPS {
        return Err(CoreError::new(
            ErrorCode::InvalidArgument,
            format!("browser_run_steps requires 1 to {MAX_BATCH_STEPS} steps"),
        ));
    }
    Ok(())
}

pub fn redacted_origin(input: &str) -> Result<String, CoreError> {
    let url = validate_navigation_url(input)?;
    let host = url.host_str().ok_or_else(|| {
        CoreError::new(
            ErrorCode::InvalidArgument,
            "browser URL must include a host",
        )
    })?;
    let mut origin = format!("{}://{}", url.scheme(), host);
    if let Some(port) = url.port() {
        origin.push(':');
        origin.push_str(&port.to_string());
    }
    Ok(origin)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_core::error::ErrorCode;

    #[test]
    fn url_policy_accepts_http_https_and_localhost() {
        assert!(validate_navigation_url("https://example.com/a?q=1").is_ok());
        assert!(validate_navigation_url("http://127.0.0.1:5173/").is_ok());
        assert!(validate_navigation_url("http://localhost:3000/").is_ok());
    }

    #[test]
    fn url_policy_rejects_file_and_non_http_schemes() {
        assert_eq!(
            validate_navigation_url("file:///C:/secret.txt")
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            validate_navigation_url("javascript:alert(1)")
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn viewport_is_bounded() {
        assert!(validate_viewport(1, 1).is_ok());
        assert!(validate_viewport(8192, 8192).is_ok());
        assert_eq!(
            validate_viewport(0, 600).unwrap_err().code,
            ErrorCode::BrowserInvalidViewport
        );
        assert_eq!(
            validate_viewport(9000, 600).unwrap_err().code,
            ErrorCode::BrowserInvalidViewport
        );
    }

    #[test]
    fn timeout_policy_is_bounded() {
        assert_eq!(validate_timeout(None, false).unwrap(), 10_000);
        assert_eq!(validate_timeout(None, true).unwrap(), 60_000);
        assert_eq!(validate_timeout(Some(120_000), false).unwrap(), 120_000);
        assert_eq!(
            validate_timeout(Some(120_001), false).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn event_limits_and_batch_limits_are_bounded() {
        assert_eq!(bounded_event_limit(None).unwrap(), 100);
        assert_eq!(bounded_event_limit(Some(500)).unwrap(), 500);
        assert_eq!(
            bounded_event_limit(Some(501)).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert!(validate_batch_step_count(20).is_ok());
        assert_eq!(
            validate_batch_step_count(21).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn redacted_origin_drops_path_query_and_fragment() {
        assert_eq!(
            redacted_origin("https://example.com:8443/path?q=secret#x").unwrap(),
            "https://example.com:8443"
        );
    }
}

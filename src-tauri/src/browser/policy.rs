use lc_core::error::{CoreError, ErrorCode};

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
    fn redacted_origin_drops_path_query_and_fragment() {
        assert_eq!(
            redacted_origin("https://example.com:8443/path?q=secret#x").unwrap(),
            "https://example.com:8443"
        );
    }
}

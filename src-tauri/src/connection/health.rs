//! Public Funnel health verification: constructs the capability-scoped health
//! URL, probes it through the public tunnel with bounded retries, and reports
//! errors that never echo the capability token or its URL path.

use std::time::Duration;

/// Marker header the bridge must return on the public health endpoint.
const HEALTH_MARKER_HEADER: &str = "x-mcp-bridge-health";

/// Per-attempt timeout for the public health probe.
const REQUEST_TIMEOUT_SECS: u64 = 5;

/// Backoff schedule (ms) before each of the five probe attempts.
pub const RETRY_DELAYS_MS: [u64; 5] = [250, 500, 1000, 2000, 2000];

#[derive(Debug, thiserror::Error)]
pub enum HealthError {
    #[error("invalid Funnel hostname")]
    InvalidFqdn,
    #[error("public health request failed: {0}")]
    Request(String),
    #[error("public health endpoint returned HTTP {status}")]
    UnexpectedStatus { status: u16 },
    #[error("public health endpoint returned an invalid marker")]
    InvalidMarker,
}

/// Builds `https://{fqdn}/{capability}/health`, rejecting FQDN strings that
/// could alter the URL shape (paths, queries, fragments, whitespace, scheme
/// prefixes) and re-validating the parsed host against the input.
fn health_url(fqdn: &str, capability: &str) -> Result<url::Url, HealthError> {
    let invalid = fqdn.is_empty()
        || fqdn.contains('/')
        || fqdn.contains('?')
        || fqdn.contains('#')
        || fqdn.contains("://")
        || fqdn.starts_with("http")
        || fqdn.chars().any(char::is_whitespace);
    if invalid {
        return Err(HealthError::InvalidFqdn);
    }
    let parsed = url::Url::parse(&format!("https://{fqdn}/{capability}/health"))
        .map_err(|_| HealthError::InvalidFqdn)?;
    // Defense against parse surprises: the host must round-trip to the input.
    match parsed.host_str() {
        Some(host) if host == fqdn => Ok(parsed),
        _ => Err(HealthError::InvalidFqdn),
    }
}

/// One probe attempt. Takes a full [`url::Url`] so tests can point it at a
/// local server instead of the real tunnel.
async fn try_once(client: &reqwest::Client, url: &url::Url) -> Result<(), HealthError> {
    let response = client
        .get(url.clone())
        .send()
        .await
        .map_err(classify_request)?;
    let status = response.status();
    if status != reqwest::StatusCode::NO_CONTENT {
        return Err(HealthError::UnexpectedStatus {
            status: status.as_u16(),
        });
    }
    let marker_ok = response.headers().get(HEALTH_MARKER_HEADER)
        == Some(&reqwest::header::HeaderValue::from_static("ok"));
    if !marker_ok {
        return Err(HealthError::InvalidMarker);
    }
    Ok(())
}

/// Maps a reqwest failure to a token-free error kind string. Never embeds the
/// raw reqwest message, which references the full URL including the capability.
fn classify_request(e: reqwest::Error) -> HealthError {
    let kind = if e.is_timeout() {
        "timeout"
    } else if e.is_connect() {
        "connect"
    } else {
        "network"
    };
    HealthError::Request(format!("request to public health endpoint failed: {kind}"))
}

/// Probes the public Funnel health endpoint up to five times with the
/// [`RETRY_DELAYS_MS`] backoff schedule, requiring HTTP 204 plus the
/// `x-mcp-bridge-health: ok` marker on success.
pub async fn verify_public_health(fqdn: &str, capability: &str) -> Result<(), HealthError> {
    let url = health_url(fqdn, capability)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|_| {
            HealthError::Request("request to public health endpoint failed: client".into())
        })?;
    let mut last = HealthError::Request("no attempts made".into());
    for (attempt, delay_ms) in RETRY_DELAYS_MS.into_iter().enumerate() {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
        match try_once(&client, &url).await {
            Ok(()) => return Ok(()),
            Err(e) => last = e,
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_expected_health_url() {
        let url = health_url("local-workspace-mcp.example.ts.net", "abc").expect("valid inputs");
        assert_eq!(
            url.as_str(),
            "https://local-workspace-mcp.example.ts.net/abc/health"
        );
    }

    #[test]
    fn rejects_fqdns_that_could_alter_the_url_shape() {
        for bad in [
            "",
            "host/path",
            "https://evil.com",
            "host?q=1",
            "host#f",
            "host with space",
            "host\ttab",
        ] {
            assert!(
                health_url(bad, "abc").is_err(),
                "expected rejection: {bad:?}"
            );
        }
    }

    #[test]
    fn invalid_fqdn_error_never_contains_the_capability() {
        let capability = "super-secret-capability-token";
        let err = health_url("bad/host", capability).unwrap_err();
        assert!(!err.to_string().contains(capability));
    }

    #[test]
    fn retry_delay_schedule_matches_contract() {
        assert_eq!(RETRY_DELAYS_MS, [250, 500, 1000, 2000, 2000]);
    }

    /// Spawns a local axum server answering GET `<path>` with `status` and an
    /// optional marker header; returns its base URL.
    async fn spawn_health_server(
        status: axum::http::StatusCode,
        marker: Option<&'static str>,
    ) -> url::Url {
        let handler = move || async move {
            let mut headers = axum::http::HeaderMap::new();
            if let Some(value) = marker {
                headers.insert(HEALTH_MARKER_HEADER, value.parse().unwrap());
            }
            (status, headers)
        };
        let app = axum::Router::new().route("/abc/health", axum::routing::get(handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        url::Url::parse(&format!("http://{addr}/abc/health")).unwrap()
    }

    #[tokio::test]
    async fn accepts_204_with_marker_header() {
        let url = spawn_health_server(axum::http::StatusCode::NO_CONTENT, Some("ok")).await;
        let client = reqwest::Client::new();
        assert!(try_once(&client, &url).await.is_ok());
    }

    #[tokio::test]
    async fn treats_404_as_unexpected_status_not_success() {
        let url = spawn_health_server(axum::http::StatusCode::NOT_FOUND, None).await;
        let client = reqwest::Client::new();
        match try_once(&client, &url).await.unwrap_err() {
            HealthError::UnexpectedStatus { status } => assert_eq!(status, 404),
            other => panic!("expected UnexpectedStatus, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_204_missing_marker_header() {
        let url = spawn_health_server(axum::http::StatusCode::NO_CONTENT, None).await;
        let client = reqwest::Client::new();
        assert!(matches!(
            try_once(&client, &url).await,
            Err(HealthError::InvalidMarker)
        ));
    }
}

//! Public Funnel health verification: constructs the capability-scoped health
//! URL, probes it through the public tunnel with bounded retries, and reports
//! errors that never echo the capability token or its URL path.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

/// Marker header the bridge must return on the public health endpoint.
const HEALTH_MARKER_HEADER: &str = "x-mcp-bridge-health";

/// Per-attempt timeout for the public health probe.
const REQUEST_TIMEOUT_SECS: u64 = 5;

/// Backoff schedule (ms) before each of the five probe attempts.
pub const RETRY_DELAYS_MS: [u64; 5] = [250, 500, 1000, 2000, 2000];

/// DNS-over-HTTPS JSON resolvers tried in order when the system resolver
/// yields no usable IPv4 address for the funnel host.
const DOH_ENDPOINTS: [&str; 2] = [
    "https://dns.google/resolve",
    "https://cloudflare-dns.com/dns-query",
];

/// Per-attempt timeout for a DoH lookup.
const DOH_TIMEOUT_SECS: u64 = 3;

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

/// True only for globally routable IPv4 addresses (`Ipv4Addr::is_global` is
/// still unstable): rejects this-network, private, loopback, CGNAT/shared,
/// link-local, documentation, benchmarking, multicast, and reserved ranges so
/// a hostile or broken DNS answer can never aim the probe at internal
/// services.
fn is_global_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !matches!(
        o,
        [0, _, _, _]
            | [10, _, _, _]
            | [127, _, _, _]
            | [100, 64..=127, _, _]
            | [169, 254, _, _]
            | [172, 16..=31, _, _]
            | [192, 0, 0, _]
            | [192, 0, 2, _]
            | [192, 168, _, _]
            | [198, 18..=19, _, _]
            | [198, 51, 100, _]
            | [203, 0, 113, _]
            | [224..=239, _, _, _]
            | [240..=255, _, _, _]
    )
}

/// Extracts globally routable IPv4 addresses from a DoH JSON response (the
/// RFC 8484-style JSON served by Google/Cloudflare). Non-A answers, malformed
/// data, and non-global addresses (loopback, private, CGNAT, reserved) are
/// dropped so a hostile or broken answer can never redirect the probe.
fn parse_doh_ipv4(body: &str) -> Vec<Ipv4Addr> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(answers) = value.get("Answer").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    answers
        .iter()
        .filter(|a| a.get("type").and_then(serde_json::Value::as_u64) == Some(1))
        .filter_map(|a| a.get("data").and_then(serde_json::Value::as_str))
        .filter_map(|s| s.parse::<Ipv4Addr>().ok())
        .filter(|ip| is_global_v4(*ip))
        .collect()
}

/// One DoH A-record lookup against `endpoint` (e.g. `https://dns.google/resolve`).
async fn doh_lookup_v4(client: &reqwest::Client, endpoint: &str, fqdn: &str) -> Vec<Ipv4Addr> {
    // `fqdn` is pre-validated by `health_url` (no whitespace, slashes, or
    // query metacharacters), so interpolating it into the query is safe.
    let url = format!("{endpoint}?name={fqdn}&type=A");
    let Ok(response) = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/dns-json")
        .send()
        .await
    else {
        return Vec::new();
    };
    let Ok(body) = response.text().await else {
        return Vec::new();
    };
    parse_doh_ipv4(&body)
}

/// Resolves probe targets for `fqdn`: global IPv4 addresses from the system
/// resolver first, then DoH fallback. Empty means "no pin" — the caller falls
/// back to the client's own resolution. Pinning exists because some ISP
/// resolvers answer AAAA-only for `*.ts.net`, which strands the probe on an
/// IPv6 path the machine may not have.
async fn resolve_probe_addrs(fqdn: &str) -> Vec<SocketAddr> {
    if let Ok(addrs) = tokio::net::lookup_host((fqdn, 443)).await {
        let v4: Vec<SocketAddr> = addrs
            .filter(|a| match a {
                SocketAddr::V4(v4) => is_global_v4(*v4.ip()),
                SocketAddr::V6(_) => false,
            })
            .collect();
        if !v4.is_empty() {
            return v4;
        }
    }
    let Ok(doh_client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(DOH_TIMEOUT_SECS))
        .build()
    else {
        return Vec::new();
    };
    for endpoint in DOH_ENDPOINTS {
        let found = doh_lookup_v4(&doh_client, endpoint, fqdn).await;
        if !found.is_empty() {
            return found
                .into_iter()
                .map(|ip| SocketAddr::from((ip, 443)))
                .collect();
        }
    }
    Vec::new()
}

/// Probes the public Funnel health endpoint up to five times with the
/// [`RETRY_DELAYS_MS`] backoff schedule, requiring HTTP 204 plus the
/// `x-mcp-bridge-health: ok` marker on success.
pub async fn verify_public_health(fqdn: &str, capability: &str) -> Result<(), HealthError> {
    let url = health_url(fqdn, capability)?;
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS));
    let addrs = resolve_probe_addrs(fqdn).await;
    if !addrs.is_empty() {
        builder = builder.resolve_to_addrs(fqdn, &addrs);
    }
    let client = builder
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

    #[test]
    fn doh_parse_keeps_only_global_a_records() {
        let body = r#"{
            "Status": 0,
            "Answer": [
                {"name": "x.ts.net", "type": 1, "data": "103.84.155.153"},
                {"name": "x.ts.net", "type": 1, "data": "192.168.1.1"},
                {"name": "x.ts.net", "type": 1, "data": "127.0.0.1"},
                {"name": "x.ts.net", "type": 1, "data": "100.122.128.89"},
                {"name": "x.ts.net", "type": 28, "data": "2403:2500:400:20::25a"},
                {"name": "x.ts.net", "type": 1, "data": "not-an-ip"}
            ]
        }"#;
        let ips = parse_doh_ipv4(body);
        assert_eq!(ips, vec!["103.84.155.153".parse::<Ipv4Addr>().unwrap()]);
    }

    #[test]
    fn doh_parse_tolerates_garbage_and_missing_answers() {
        assert!(parse_doh_ipv4("not json").is_empty());
        assert!(parse_doh_ipv4(r#"{"Status": 3}"#).is_empty());
        assert!(parse_doh_ipv4(r#"{"Answer": "nope"}"#).is_empty());
    }

    #[test]
    fn global_v4_filter_rejects_internal_and_reserved_ranges() {
        for global in ["103.84.155.153", "8.8.8.8", "1.1.1.1", "203.0.114.1"] {
            assert!(
                is_global_v4(global.parse().unwrap()),
                "should be global: {global}"
            );
        }
        for internal in [
            "0.0.0.0",
            "10.0.0.1",
            "127.0.0.1",
            "100.122.128.89", // Tailscale CGNAT range
            "169.254.1.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "198.51.100.7",
            "203.0.113.9",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
        ] {
            assert!(
                !is_global_v4(internal.parse().unwrap()),
                "should NOT be global: {internal}"
            );
        }
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

//! Public Funnel health verification: constructs the capability-scoped health
//! URL, probes it through the public tunnel with bounded retries, and reports
//! errors that never echo the capability token or its URL path.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

/// Marker header the bridge must return on the public health endpoint.
const HEALTH_MARKER_HEADER: &str = "x-mcp-bridge-health";

/// Per-attempt timeout for the public health probe. Funnel health is a tiny
/// 204 response, so a multi-second connect stall should fail fast and let the
/// next attempt / IPv4 fallback take over.
const REQUEST_TIMEOUT_SECS: u64 = 2;

/// Delays (ms) between five total probe attempts. The first probe is always
/// immediate; these four waits total only 1.5s instead of the old 5.5s.
pub const RETRY_DELAYS_MS: [u64; 4] = [100, 200, 400, 800];

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

/// Runs the first public probe immediately while IPv4 fallback resolution
/// happens in the background. A healthy endpoint therefore never waits on
/// system DNS/DoH pre-resolution; only a failed direct probe waits for the
/// fallback result before the remaining retries.
async fn first_probe_with_background_resolution(
    client: &reqwest::Client,
    url: &url::Url,
    resolver: tokio::task::JoinHandle<Vec<SocketAddr>>,
) -> (Result<(), HealthError>, Option<Vec<SocketAddr>>) {
    let result = try_once(client, url).await;
    match &result {
        // A successful probe obviously needs no fallback. Likewise, an HTTP
        // response (even 404) or a bad marker proves DNS/TLS/connectivity are
        // already working; only the public route is not ready yet. Waiting on
        // alternate DNS in those cases can only make startup slower.
        Ok(()) | Err(HealthError::UnexpectedStatus { .. }) | Err(HealthError::InvalidMarker) => {
            resolver.abort();
            (result, None)
        }
        Err(HealthError::Request(_)) => {
            let addrs = resolver.await.unwrap_or_default();
            (result, Some(addrs))
        }
        Err(HealthError::InvalidFqdn) => {
            resolver.abort();
            (result, None)
        }
    }
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

/// Races two IPv4 resolver futures and returns the first non-empty answer. If
/// the first completed resolver has no usable addresses, wait only for the
/// other one. This keeps a slow/unreachable DoH provider off the fallback
/// critical path without sacrificing the second-provider safety net.
async fn first_nonempty_ipv4<A, B>(a: A, b: B) -> Vec<Ipv4Addr>
where
    A: std::future::Future<Output = Vec<Ipv4Addr>>,
    B: std::future::Future<Output = Vec<Ipv4Addr>>,
{
    tokio::pin!(a);
    tokio::pin!(b);
    tokio::select! {
        first = &mut a => {
            if first.is_empty() { b.await } else { first }
        }
        first = &mut b => {
            if first.is_empty() { a.await } else { first }
        }
    }
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
    let found = first_nonempty_ipv4(
        doh_lookup_v4(&doh_client, DOH_ENDPOINTS[0], fqdn),
        doh_lookup_v4(&doh_client, DOH_ENDPOINTS[1], fqdn),
    )
    .await;
    found
        .into_iter()
        .map(|ip| SocketAddr::from((ip, 443)))
        .collect()
}

/// Probes the public Funnel health endpoint up to five times with the
/// [`RETRY_DELAYS_MS`] backoff schedule, requiring HTTP 204 plus the
/// `x-mcp-bridge-health: ok` marker on success.
pub async fn verify_public_health(fqdn: &str, capability: &str) -> Result<(), HealthError> {
    let url = health_url(fqdn, capability)?;
    let direct_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|_| {
            HealthError::Request("request to public health endpoint failed: client".into())
        })?;

    // DNS fallback used to block the happy path before the first request. Run
    // it concurrently instead: if ordinary HTTPS works, Online can be reached
    // immediately; if it fails (notably AAAA-only/broken IPv6 networks), the
    // already-running resolver supplies global IPv4 addresses for retries.
    let fqdn_owned = fqdn.to_string();
    let resolver = tokio::spawn(async move { resolve_probe_addrs(&fqdn_owned).await });
    let (first, resolved) =
        first_probe_with_background_resolution(&direct_client, &url, resolver).await;
    let mut last = match first {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };

    let retry_client = if let Some(addrs) = resolved.filter(|addrs| !addrs.is_empty()) {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .resolve_to_addrs(fqdn, &addrs)
            .build()
            .map_err(|_| {
                HealthError::Request("request to public health endpoint failed: client".into())
            })?
    } else {
        direct_client
    };

    for delay_ms in RETRY_DELAYS_MS {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        match try_once(&retry_client, &url).await {
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
    fn retry_delay_schedule_is_fast_and_bounded() {
        assert_eq!(RETRY_DELAYS_MS, [100, 200, 400, 800]);
        assert_eq!(RETRY_DELAYS_MS.iter().sum::<u64>(), 1_500);
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
    async fn ready_endpoint_is_not_blocked_by_slow_fallback_resolution() {
        let url = spawn_health_server(axum::http::StatusCode::NO_CONTENT, Some("ok")).await;
        let client = reqwest::Client::new();
        let resolver = tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Vec::<SocketAddr>::new()
        });

        let result = tokio::time::timeout(
            Duration::from_millis(500),
            first_probe_with_background_resolution(&client, &url, resolver),
        )
        .await
        .expect("ready health probe must not wait for fallback DNS");

        assert!(result.0.is_ok());
        assert!(result.1.is_none());
    }

    #[tokio::test]
    async fn reachable_but_not_ready_endpoint_does_not_wait_for_dns_fallback() {
        let url = spawn_health_server(axum::http::StatusCode::NOT_FOUND, None).await;
        let client = reqwest::Client::new();
        let resolver = tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Vec::<SocketAddr>::new()
        });

        let result = tokio::time::timeout(
            Duration::from_millis(500),
            first_probe_with_background_resolution(&client, &url, resolver),
        )
        .await
        .expect("reachable endpoint must not wait for fallback DNS");

        assert!(matches!(
            result.0,
            Err(HealthError::UnexpectedStatus { status: 404 })
        ));
        assert!(result.1.is_none());
    }

    #[tokio::test]
    async fn doh_fallback_returns_first_nonempty_resolver_without_waiting_for_slow_peer() {
        let fast_ip: Ipv4Addr = "103.84.155.153".parse().unwrap();
        let slow = async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Vec::<Ipv4Addr>::new()
        };
        let fast = async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            vec![fast_ip]
        };

        let got = tokio::time::timeout(Duration::from_millis(500), first_nonempty_ipv4(slow, fast))
            .await
            .expect("fast DoH resolver should win immediately");
        assert_eq!(got, vec![fast_ip]);
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

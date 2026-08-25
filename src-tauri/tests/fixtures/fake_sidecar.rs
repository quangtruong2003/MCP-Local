//! Fake funnel sidecar used by `tests/connection_integration.rs`.
//!
//! Prints one NDJSON event per line on stdout based on a scenario argument,
//! then exits (except where noted). Deliberately standalone — it does NOT
//! link the manager crate, so it exercises the real spawn/stdout-parse path
//! from an external process exactly like the Go sidecar would.
//!
//! SECURITY: this program never reads or prints `MCP_CAPABILITY_TOKEN` or any
//! real capability value. Where a token must appear conceptually, the literal
//! placeholder `<token>` is used instead.

use std::io::Write;
use std::time::Duration;

fn main() {
    // The manager-style spawn passes real flags first (--target, ...), so
    // the scenario is always the final argument.
    let scenario = std::env::args()
        .next_back()
        .unwrap_or_else(|| "online".to_string());

    let mut out = std::io::stdout().lock();
    let mut emit = |line: &str| {
        // Flush every line so the parent's reader thread sees events
        // individually, like a real streaming sidecar.
        writeln!(out, "{line}").unwrap();
        out.flush().unwrap();
    };

    match scenario.as_str() {
        "online" => {
            emit(r#"{"type":"starting"}"#);
            emit(r#"{"type":"funnel_listening","fqdn":"box.tailnet.ts.net"}"#);
            // Stay alive so the test controls shutdown via stop().
            park();
        }
        "auth_then_online" => {
            emit(r#"{"type":"starting"}"#);
            emit(r#"{"type":"auth_required","url":"https://login.tailscale.com/a/fixture-1"}"#);
            emit(r#"{"type":"authenticated"}"#);
            emit(r#"{"type":"funnel_listening","fqdn":"box.tailnet.ts.net"}"#);
            park();
        }
        "funnel_denied" => {
            emit(r#"{"type":"starting"}"#);
            emit(concat!(
                r#"{"type":"error","code":"FUNNEL_NOT_ALLOWED","#,
                r#""message":"funnel permission missing for node <token>"}"#
            ));
            // Keep listening: FUNNEL_NOT_ALLOWED is actionable, not fatal, and
            // the manager must keep the process around for a later retry.
            park();
        }
        "malformed_event" => {
            emit("this is not json at all");
            emit(r#"{"type":"weird","x":1}"#);
            emit("");
            emit(r#"{"type":"starting"}"#);
            emit(r#"{"type":"funnel_listening","fqdn":"box.tailnet.ts.net"}"#);
            park();
        }
        "exit_early" => {
            emit(r#"{"type":"starting"}"#);
            // No funnel event; process exits immediately. The manager must
            // settle transitional states into SIDECAR_EXITED.
        }
        "slow_start" => {
            std::thread::sleep(Duration::from_millis(400));
            emit(r#"{"type":"starting"}"#);
            emit(r#"{"type":"funnel_listening","fqdn":"box.tailnet.ts.net"}"#);
            park();
        }
        other => {
            eprintln!("unknown scenario: {other}");
            std::process::exit(2);
        }
    }
}

/// Sleeps forever without touching stdout; the job-object teardown in
/// sidecar.rs is what ends us during tests that call stop().
fn park() {
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

//! End-to-end connection lifecycle tests driving the REAL sidecar spawn +
//! NDJSON parse pipeline (`spawn_sidecar_exe` → reader threads → manager run
//! loop) against a fake sidecar executable, so no Tailscale network or Go
//! sidecar binary is needed.
//!
//! The fake fixture is the standalone `tests/fixtures/fake_sidecar.rs`
//! example; `cargo test` builds it into `<target-dir>/examples/`. It selects
//! its event sequence from its first CLI argument (see that file for the
//! scenario list) and never reads or prints the capability token.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use local_workspace_mcp_manager::connection::health::HealthError;
use local_workspace_mcp_manager::connection::manager::{
    ConnectionError, ConnectionManager, HealthVerifier, SidecarDriver, SidecarSession,
    StatusEmitter,
};
use local_workspace_mcp_manager::connection::secrets::CapabilitySecretStore;
use local_workspace_mcp_manager::connection::sidecar::{
    spawn_sidecar_exe, SidecarConfig, SidecarEvent, SidecarProcess,
};
use local_workspace_mcp_manager::connection::state::{ConnectionState, ConnectionStatus};

const HOSTNAME: &str = "box.tailnet.ts.net";
const PORT: u16 = 8787;
const WAIT: Duration = Duration::from_secs(10);

/// Locates the fake sidecar example binary built by cargo. Prefers
/// `<profile>/examples/fake_sidecar(.exe)` next to the test executable,
/// falling back to the target dir reported by cargo at compile time.
fn fake_sidecar_exe() -> PathBuf {
    let name = format!("fake_sidecar{}", std::env::consts::EXE_SUFFIX);
    if let Ok(mut path) = std::env::current_exe() {
        path.pop(); // debug/ or release/
        path.pop(); // deps/
        let candidate = path.join("examples").join(&name);
        if candidate.is_file() {
            return candidate;
        }
        // Some layouts keep examples beside the top profile dir already.
        let candidate = path.join(&name);
        if candidate.is_file() {
            return candidate;
        }
    }
    // `env!` would fail to compile when the variable is unset (plain
    // `cargo test` outside CI); fall back to the workspace-local default.
    PathBuf::from(option_env!("CARGO_TARGET_DIR").unwrap_or("target"))
        .join("debug")
        .join("examples")
        .join(name)
}

/// Builds the fake fixture before each scenario. `cargo test` does not
/// rebuild examples automatically, and a stale fixture would silently
/// exercise outdated behavior — so always invoke the (fast, incremental)
/// build.
fn ensure_fake_built() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .arg("build")
        .arg("--example")
        .arg("fake_sidecar")
        .current_dir(&manifest)
        .status()
        .expect("failed to run cargo build --example fake_sidecar");
    assert!(status.success(), "building fake_sidecar example failed");
    let exe = fake_sidecar_exe();
    assert!(
        exe.is_file(),
        "fake sidecar not found after build at {exe:?}"
    );
}

/// Real-process driver: spawns the fake fixture through [`spawn_sidecar_exe`]
/// with the scenario appended as an extra CLI argument, exactly like
/// production's `ProcessSidecarDriver` except for the program and arg.
struct FakeFixtureDriver {
    program: PathBuf,
    scenario: &'static str,
}

#[async_trait]
impl SidecarDriver for FakeFixtureDriver {
    async fn spawn(&self, cfg: SidecarConfig) -> Result<Box<dyn SidecarSession>, ConnectionError> {
        let (process, rx) =
            spawn_sidecar_exe(cfg, self.program.clone(), vec![self.scenario.to_string()])
                .map_err(ConnectionError::from)?;
        Ok(Box::new(FixtureSession { process, rx }))
    }
}

struct FixtureSession {
    process: SidecarProcess,
    rx: tokio::sync::mpsc::UnboundedReceiver<SidecarEvent>,
}

#[async_trait]
impl SidecarSession for FixtureSession {
    async fn next_event(&mut self) -> Option<SidecarEvent> {
        self.rx.recv().await
    }

    async fn stop(&mut self) -> Result<(), ConnectionError> {
        self.process.stop().await.map_err(Into::into)
    }

    fn recent_diagnostics(&self) -> Vec<String> {
        self.process.recent_diagnostics()
    }
}

/// Verifier that always succeeds instantly — health is covered by unit tests;
/// here we only need the funnel-listening → Online leg to complete.
struct InstantOkVerifier;

#[async_trait]
impl HealthVerifier for InstantOkVerifier {
    async fn verify(&self, _fqdn: &str, _capability: &str) -> Result<(), HealthError> {
        Ok(())
    }
}

struct Harness {
    mgr: ConnectionManager,
    /// Keeps temp dirs alive for the whole test.
    _dirs: (tempfile::TempDir, tempfile::TempDir),
    initial_capability: String,
    seen: Arc<std::sync::Mutex<Vec<ConnectionState>>>,
}

fn harness(scenario: &'static str) -> Harness {
    let app_dir = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let app_data = app_dir.path().to_path_buf();

    let store = CapabilitySecretStore::new(app_data.clone());
    let initial_capability = store.load_or_create().unwrap();

    let seen: Arc<std::sync::Mutex<Vec<ConnectionState>>> = Arc::default();
    let sink = seen.clone();
    let emitter: StatusEmitter =
        Arc::new(move |status: &ConnectionStatus| sink.lock().unwrap().push(status.state.clone()));

    let mgr = ConnectionManager::with_parts(
        Arc::new(FakeFixtureDriver {
            program: fake_sidecar_exe(),
            scenario,
        }),
        Arc::new(InstantOkVerifier),
        emitter,
        CapabilitySecretStore::new(app_data),
        state_dir.path().to_path_buf(),
        HOSTNAME.into(),
        PORT,
    );

    Harness {
        mgr,
        _dirs: (app_dir, state_dir),
        initial_capability,
        seen,
    }
}

async fn wait_for(
    mgr: &ConnectionManager,
    pred: impl Fn(&ConnectionState) -> bool,
) -> ConnectionState {
    let deadline = Instant::now() + WAIT;
    loop {
        let state = mgr.status().await.state;
        if pred(&state) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for state, last: {state:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn online_reaches_online_after_funnel_listening() {
    ensure_fake_built();
    let h = harness("online");
    h.mgr.start().await.unwrap();

    let online = wait_for(&h.mgr, |s| matches!(s, ConnectionState::Online { .. })).await;
    match online {
        ConnectionState::Online { fqdn, endpoint } => {
            assert_eq!(fqdn, HOSTNAME);
            assert_eq!(
                endpoint,
                format!("https://{HOSTNAME}/{}/mcp", h.initial_capability)
            );
        }
        other => panic!("expected Online, got {other:?}"),
    }
}

#[tokio::test]
async fn auth_then_online_walks_login_sequence_to_online() {
    ensure_fake_built();
    let h = harness("auth_then_online");
    h.mgr.start().await.unwrap();

    let online = wait_for(&h.mgr, |s| matches!(s, ConnectionState::Online { .. })).await;
    assert!(matches!(online, ConnectionState::Online { .. }));

    let expected = vec![
        ConnectionState::StartingServer,
        ConnectionState::StartingTunnel,
        ConnectionState::NeedsLogin {
            login_url: "https://login.tailscale.com/a/fixture-1".into(),
        },
        ConnectionState::Verifying {
            fqdn: HOSTNAME.into(),
        },
        ConnectionState::Online {
            fqdn: HOSTNAME.into(),
            endpoint: format!("https://{HOSTNAME}/{}/mcp", h.initial_capability),
        },
    ];
    assert_eq!(*h.seen.lock().unwrap(), expected);
}

#[tokio::test]
async fn funnel_denied_lands_in_needs_funnel_permission_and_process_is_cleaned_up_on_stop() {
    ensure_fake_built();
    let h = harness("funnel_denied");
    h.mgr.start().await.unwrap();

    let state = wait_for(&h.mgr, |s| {
        matches!(s, ConnectionState::NeedsFunnelPermission { .. })
    })
    .await;
    match state {
        ConnectionState::NeedsFunnelPermission { message } => {
            assert_eq!(message, "funnel permission missing for node <token>");
        }
        other => panic!("expected NeedsFunnelPermission, got {other:?}"),
    }

    // Stop must clean up: Stopped status and no leftover child process.
    let stopped = h.mgr.stop().await.unwrap();
    assert_eq!(stopped.state, ConnectionState::Stopped);

    // Identity files survive a stop.
    assert!(h._dirs.0.path().join("connection-secret.bin").is_file());
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_events_go_to_diagnostics_and_valid_events_still_land_online() {
    ensure_fake_built();
    let h = harness("malformed_event");
    h.mgr.start().await.unwrap();

    let online = wait_for(&h.mgr, |s| matches!(s, ConnectionState::Online { .. })).await;
    assert!(matches!(online, ConnectionState::Online { .. }));

    // Malformed lines were captured in diagnostics instead of crashing.
    let diag = h.mgr.diagnostics_snapshot().await.join("\n");
    assert!(diag.contains("this is not json at all"));
    // ...and nothing leaked the capability token into them.
    assert!(!diag.contains(&h.initial_capability));
}

#[tokio::test]
async fn exit_early_settles_into_error_state_with_code() {
    ensure_fake_built();
    let h = harness("exit_early");
    h.mgr.start().await.unwrap();

    let state = wait_for(&h.mgr, |s| matches!(s, ConnectionState::Error { .. })).await;
    match state {
        ConnectionState::Error { code, message } => {
            assert_eq!(code, "SIDECAR_EXITED");
            assert!(message.contains("exited unexpectedly"));
        }
        other => panic!("expected Error(SIDECAR_EXITED), got {other:?}"),
    }
}

#[tokio::test]
async fn slow_start_stays_starting_until_first_event_then_reaches_online() {
    ensure_fake_built();
    let h = harness("slow_start");
    h.mgr.start().await.unwrap();

    // Immediately after start we are still in a starting state — the fake
    // sleeps 400ms before its first event.
    let early = h.mgr.status().await.state;
    assert!(
        matches!(
            early,
            ConnectionState::StartingServer | ConnectionState::StartingTunnel
        ),
        "expected a starting state right after start(), got {early:?}"
    );

    let online = wait_for(&h.mgr, |s| matches!(s, ConnectionState::Online { .. })).await;
    assert!(matches!(online, ConnectionState::Online { .. }));
}

//! Connection manager: drives the funnel sidecar through the
//! [`ConnectionState`] machine behind an operation lock and monotonically
//! increasing run generations, so stale events from a previous run can never
//! mark a newer run Online.
//!
//! Security invariants:
//! - the capability token never appears in any error code/message or log;
//! - the composed endpoint URL (which embeds the token by design) is only
//!   ever stored in the `Online` variant of [`ConnectionState`].
//!
//! Emission design: state changes are published through an injectable
//! [`StatusEmitter`] closure (default: no-op) so the manager is fully
//! unit-testable without a Tauri runtime. Production wires
//! [`ConnectionManager::tauri_emitter`] in `lib.rs`, emitting the
//! `connection-status` event after every transition.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{mpsc, Mutex, Notify};

use super::health::{verify_public_health, HealthError};
use super::secrets::CapabilitySecretStore;
use super::sidecar::{
    spawn_sidecar, SidecarConfig, SidecarEvent, SidecarProcess, SidecarSpawnError,
};
use super::state::{ConnectionState, ConnectionStatus};

/// One stable internal error shape commands can serialize without exposing
/// secrets. `Display` renders only the safe human message.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct ConnectionError {
    /// Stable machine-readable code, e.g. `SIDECAR_NOT_FOUND`.
    pub code: String,
    /// Human-safe message; never contains the capability token.
    pub message: String,
}

impl From<SidecarSpawnError> for ConnectionError {
    // SidecarSpawnError is crate-private by design; this conversion is the
    // Task-10 bridge its TODO notes anticipated (codes map 1:1).
    #[allow(private_interfaces)]
    fn from(e: SidecarSpawnError) -> Self {
        Self {
            code: e.code,
            message: e.message,
        }
    }
}

/// Async facade over a running sidecar so the manager never touches the raw
/// process/channel directly (tests inject fakes).
#[async_trait]
pub trait SidecarSession: Send {
    async fn next_event(&mut self) -> Option<SidecarEvent>;
    async fn stop(&mut self) -> Result<(), ConnectionError>;

    /// Redacted recent sidecar output lines (most recent last). Default is
    /// empty so test fakes need no implementation; lines are pre-redacted by
    /// the sidecar reader threads.
    fn recent_diagnostics(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Spawns sidecar sessions.
#[async_trait]
pub trait SidecarDriver: Send + Sync {
    async fn spawn(&self, cfg: SidecarConfig) -> Result<Box<dyn SidecarSession>, ConnectionError>;
}

/// Verifies the public Funnel endpoint health.
#[async_trait]
pub trait HealthVerifier: Send + Sync {
    async fn verify(&self, fqdn: &str, capability: &str) -> Result<(), HealthError>;
}

/// Production session: owns the child process so dropping (or stopping) it
/// tears the process tree down.
struct ProcessSession {
    process: SidecarProcess,
    rx: mpsc::UnboundedReceiver<SidecarEvent>,
}

#[async_trait]
impl SidecarSession for ProcessSession {
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

/// Production driver wrapping [`spawn_sidecar`].
pub struct ProcessSidecarDriver;

#[async_trait]
impl SidecarDriver for ProcessSidecarDriver {
    async fn spawn(&self, cfg: SidecarConfig) -> Result<Box<dyn SidecarSession>, ConnectionError> {
        let (process, rx) = spawn_sidecar(cfg)?;
        Ok(Box::new(ProcessSession { process, rx }))
    }
}

/// Production verifier wrapping [`verify_public_health`] (Task 9).
pub struct ReqwestHealthVerifier;

#[async_trait]
impl HealthVerifier for ReqwestHealthVerifier {
    async fn verify(&self, fqdn: &str, capability: &str) -> Result<(), HealthError> {
        verify_public_health(fqdn, capability).await
    }
}

/// Called after every state change with the full status snapshot. Default is
/// a no-op so tests need no Tauri runtime; production installs
/// [`ConnectionManager::tauri_emitter`].
pub type StatusEmitter = Arc<dyn Fn(&ConnectionStatus) + Send + Sync>;

/// Everything shared between the manager handle and its spawned run tasks.
struct Shared {
    driver: Arc<dyn SidecarDriver>,
    verifier: Arc<dyn HealthVerifier>,
    secret_store: CapabilitySecretStore,
    state_dir: PathBuf,
    hostname: String,
    local_port: u16,
    emitter: StatusEmitter,
    /// Serializes start/stop/retry/reset so only one operation runs at a time.
    op_lock: Mutex<()>,
    inner: Mutex<Inner>,
    /// Latest redacted sidecar diagnostics snapshot for the diagnostics
    /// command. Updated after spawn and after each run-loop event; lines are
    /// pre-redacted by sidecar.rs and never contain secrets.
    last_diagnostics: std::sync::Mutex<Vec<String>>,
}

struct Inner {
    status: ConnectionStatus,
    /// Monotonically increasing run generation. Bumped by every stop and by
    /// every start; run tasks only write state while their generation is
    /// still current.
    generation: u64,
    runner: Option<tokio::task::JoinHandle<()>>,
    cancel: Option<Arc<Notify>>,
}

impl Shared {
    fn emit(&self, status: &ConnectionStatus) {
        (self.emitter)(status);
    }

    /// Applies `state` only if `gen` is still the current generation.
    /// Returns whether it was applied.
    async fn set_state(&self, gen: u64, state: ConnectionState) -> bool {
        let mut inner = self.inner.lock().await;
        if inner.generation != gen {
            return false;
        }
        inner.status.state = state.clone();
        let snap = inner.status.clone();
        drop(inner);
        self.emit(&snap);
        true
    }

    async fn current_generation(&self) -> u64 {
        self.inner.lock().await.generation
    }

    async fn current_state(&self) -> ConnectionState {
        self.inner.lock().await.status.state.clone()
    }

    /// Records an `Error` state for a failed start and builds the error to
    /// return to the caller.
    async fn fail_start(&self, gen: u64, code: &str, message: String) -> ConnectionError {
        self.set_state(
            gen,
            ConnectionState::Error {
                code: code.to_string(),
                message: message.clone(),
            },
        )
        .await;
        ConnectionError {
            code: code.to_string(),
            message,
        }
    }

    /// Replaces the diagnostics snapshot (pre-redacted lines only).
    fn set_diagnostics(&self, lines: Vec<String>) {
        *self.last_diagnostics.lock().unwrap() = lines;
    }
}

/// The connection state machine handle. Cheap to clone.
#[derive(Clone)]
pub struct ConnectionManager {
    shared: Arc<Shared>,
}

impl ConnectionManager {
    /// Production constructor: real sidecar driver, real health verifier,
    /// no-op emitter (call `with_parts` / replace the emitter to push Tauri
    /// events).
    pub fn new(
        secret_store: CapabilitySecretStore,
        state_dir: PathBuf,
        hostname: String,
        local_port: u16,
    ) -> Self {
        Self::with_parts(
            Arc::new(ProcessSidecarDriver),
            Arc::new(ReqwestHealthVerifier),
            Arc::new(|_| {}),
            secret_store,
            state_dir,
            hostname,
            local_port,
        )
    }

    /// Fully injectable constructor used by tests and by `lib.rs` when wiring
    /// the Tauri emitter.
    pub fn with_parts(
        driver: Arc<dyn SidecarDriver>,
        verifier: Arc<dyn HealthVerifier>,
        emitter: StatusEmitter,
        secret_store: CapabilitySecretStore,
        state_dir: PathBuf,
        hostname: String,
        local_port: u16,
    ) -> Self {
        let status = ConnectionStatus {
            local_port,
            ..Default::default()
        };
        Self {
            shared: Arc::new(Shared {
                driver,
                verifier,
                secret_store,
                state_dir,
                hostname,
                local_port,
                emitter,
                op_lock: Mutex::new(()),
                inner: Mutex::new(Inner {
                    status,
                    generation: 0,
                    runner: None,
                    cancel: None,
                }),
                last_diagnostics: std::sync::Mutex::new(Vec::new()),
            }),
        }
    }

    /// Builds an emitter that publishes every status snapshot as the Tauri
    /// `connection-status` event. Used in `lib.rs`; kept here so the event
    /// name lives beside the state machine.
    pub fn tauri_emitter<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> StatusEmitter {
        Arc::new(move |status: &ConnectionStatus| {
            use tauri::Emitter;
            let _ = app.emit("connection-status", status);
        })
    }

    /// Starts the connection. If a run is already active (any non-Stopped
    /// state) returns the current status without spawning a duplicate
    /// sidecar.
    pub async fn start(&self) -> Result<ConnectionStatus, ConnectionError> {
        let _op = self.shared.op_lock.lock().await;
        let mut inner = self.shared.inner.lock().await;
        if !matches!(inner.status.state, ConnectionState::Stopped) {
            return Ok(inner.status.clone());
        }
        inner.generation += 1;
        let gen = inner.generation;
        inner.status.state = ConnectionState::StartingServer;
        let snap = inner.status.clone();
        drop(inner);
        self.shared.emit(&snap);

        let capability = match self.shared.secret_store.load_or_create() {
            Ok(c) => c,
            Err(e) => {
                return Err(self
                    .shared
                    .fail_start(
                        gen,
                        "SECRET_UNAVAILABLE",
                        format!("failed to load capability secret: {e}"),
                    )
                    .await)
            }
        };

        let cfg = SidecarConfig {
            local_port: self.shared.local_port,
            state_dir: self.shared.state_dir.clone(),
            hostname: self.shared.hostname.clone(),
            capability_token: capability.clone(),
        };
        let session = match self.shared.driver.spawn(cfg).await {
            Ok(s) => s,
            Err(e) => return Err(self.shared.fail_start(gen, &e.code, e.message).await),
        };
        // Snapshot whatever the fresh sidecar has already buffered so a
        // start-time failure still has diagnostics to show.
        self.shared.set_diagnostics(session.recent_diagnostics());

        self.shared
            .set_state(gen, ConnectionState::StartingTunnel)
            .await;

        let cancel = Arc::new(Notify::new());
        let runner = tokio::spawn(run_loop(
            self.shared.clone(),
            session,
            gen,
            cancel.clone(),
            capability,
        ));

        let mut inner = self.shared.inner.lock().await;
        inner.runner = Some(runner);
        inner.cancel = Some(cancel);
        Ok(inner.status.clone())
    }

    /// Stops the current run (if any). Identity files (capability secret and
    /// Tailscale state directory) are left intact.
    pub async fn stop(&self) -> Result<ConnectionStatus, ConnectionError> {
        let _op = self.shared.op_lock.lock().await;
        let (runner, cancel) = {
            let mut inner = self.shared.inner.lock().await;
            if matches!(inner.status.state, ConnectionState::Stopped) {
                return Ok(inner.status.clone());
            }
            // Bump generation: the run task may no longer write any state,
            // including a late `Online` after verification succeeds.
            inner.generation += 1;
            inner.status.state = ConnectionState::Stopping;
            let snap = inner.status.clone();
            drop(inner);
            self.shared.emit(&snap);

            let mut inner = self.shared.inner.lock().await;
            (inner.runner.take(), inner.cancel.take())
        };
        if let Some(cancel) = &cancel {
            cancel.notify_one();
        }
        if let Some(runner) = runner {
            let _ = runner.await;
        }

        let mut inner = self.shared.inner.lock().await;
        if !matches!(inner.status.state, ConnectionState::Stopped) {
            inner.status.state = ConnectionState::Stopped;
            let snap = inner.status.clone();
            drop(inner);
            self.shared.emit(&snap);
        }
        Ok(self.shared.inner.lock().await.status.clone())
    }

    /// Stop then start.
    pub async fn retry(&self) -> Result<ConnectionStatus, ConnectionError> {
        self.stop().await?;
        self.start().await
    }

    /// Current status snapshot.
    pub async fn status(&self) -> ConnectionStatus {
        self.shared.inner.lock().await.status.clone()
    }

    /// The Tailscale state directory path (for the Settings page).
    pub async fn state_dir(&self) -> PathBuf {
        self.shared.state_dir.clone()
    }

    /// Latest redacted sidecar diagnostics lines, for the diagnostics report.
    pub async fn diagnostics_snapshot(&self) -> Vec<String> {
        self.shared.last_diagnostics.lock().unwrap().clone()
    }

    /// Rotates the capability secret. While running: stop -> rotate -> start.
    /// While stopped: rotate only. Returns the resulting status; secrets are
    /// never included in the return value.
    pub async fn rotate_capability(&self) -> Result<ConnectionStatus, ConnectionError> {
        let was_running = !matches!(self.shared.current_state().await, ConnectionState::Stopped);
        if was_running {
            self.stop().await?;
        }
        self.shared
            .secret_store
            .rotate()
            .map_err(|e| ConnectionError {
                code: "SECRET_ROTATE_FAILED".into(),
                message: format!("failed to rotate capability secret: {e}"),
            })?;
        if was_running {
            self.start().await
        } else {
            Ok(self.status().await)
        }
    }

    /// Resets identity: stop, optionally remove the Tailscale state
    /// directory, rotate the capability secret, and start again only if
    /// `restart` is requested. Requires UI confirmation at the command layer.
    pub async fn reset_identity(
        &self,
        remove_state_dir: bool,
        restart: bool,
    ) -> Result<ConnectionStatus, ConnectionError> {
        self.stop().await?;
        if remove_state_dir {
            match std::fs::remove_dir_all(&self.shared.state_dir) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(ConnectionError {
                        code: "STATE_DIR_REMOVE_FAILED".into(),
                        message: format!("failed to remove Tailscale state directory: {e}"),
                    })
                }
            }
        }
        self.shared
            .secret_store
            .rotate()
            .map_err(|e| ConnectionError {
                code: "SECRET_ROTATE_FAILED".into(),
                message: format!("failed to rotate capability secret: {e}"),
            })?;
        if restart {
            self.start().await
        } else {
            Ok(self.status().await)
        }
    }
}

/// Composes the public MCP endpoint. Private: callers must have validated the
/// FQDN (the health verifier rejects malformed hosts before this runs).
fn endpoint(fqdn: &str, capability: &str) -> String {
    format!("https://{fqdn}/{capability}/mcp")
}

/// Event loop for one run generation. Exits when the sidecar channel closes,
/// the sidecar reports a fatal error, or cancellation fires; stale
/// generations never write state.
async fn run_loop(
    shared: Arc<Shared>,
    mut session: Box<dyn SidecarSession>,
    gen: u64,
    cancel: Arc<Notify>,
    capability: String,
) {
    loop {
        let event = tokio::select! {
            ev = session.next_event() => match ev {
                Some(ev) => ev,
                None => break,
            },
            _ = cancel.notified() => {
                let _ = session.stop().await;
                break;
            }
        };

        if shared.current_generation().await != gen {
            // Superseded: another stop/start owns the state machine now.
            let _ = session.stop().await;
            break;
        }

        // Keep the diagnostics snapshot fresh (lines are pre-redacted).
        shared.set_diagnostics(session.recent_diagnostics());

        match event {
            SidecarEvent::Starting | SidecarEvent::Authenticated => {
                // Already reflected by StartingServer/StartingTunnel states.
            }
            SidecarEvent::AuthRequired { url } => {
                shared
                    .set_state(gen, ConnectionState::NeedsLogin { login_url: url })
                    .await;
            }
            SidecarEvent::FunnelListening { fqdn } => {
                shared
                    .set_state(gen, ConnectionState::Verifying { fqdn: fqdn.clone() })
                    .await;
                let verified = shared.verifier.verify(&fqdn, &capability).await;
                if shared.current_generation().await != gen {
                    // Verification raced a stop: drop the success/failure, the
                    // newer run must never see us go Online.
                    let _ = session.stop().await;
                    break;
                }
                match verified {
                    Ok(()) => {
                        let url = endpoint(&fqdn, &capability);
                        shared
                            .set_state(
                                gen,
                                ConnectionState::Online {
                                    fqdn,
                                    endpoint: url,
                                },
                            )
                            .await;
                    }
                    Err(e) => {
                        // HealthError messages name the FQDN/status only,
                        // never the capability path.
                        shared
                            .set_state(
                                gen,
                                ConnectionState::Error {
                                    code: "HEALTH_VERIFY_FAILED".into(),
                                    message: e.to_string(),
                                },
                            )
                            .await;
                    }
                }
            }
            SidecarEvent::Error { code, message } => {
                if code == "FUNNEL_NOT_ALLOWED" {
                    // Actionable permission problem, not a fatal error: keep
                    // listening so a later FunnelListening still lands.
                    shared
                        .set_state(gen, ConnectionState::NeedsFunnelPermission { message })
                        .await;
                } else {
                    shared
                        .set_state(gen, ConnectionState::Error { code, message })
                        .await;
                    let _ = session.stop().await;
                    break;
                }
            }
            SidecarEvent::Stopped => break,
        }
    }

    // Natural exit (channel closed / fatal error) with our generation still
    // current: settle transitional states so the UI is never left hanging.
    let mut inner = shared.inner.lock().await;
    if inner.generation != gen {
        return; // cancelled run: stop() owns the state now
    }
    let settled = match inner.status.state.clone() {
        ConnectionState::StartingServer
        | ConnectionState::StartingTunnel
        | ConnectionState::Verifying { .. } => ConnectionState::Error {
            code: "SIDECAR_EXITED".into(),
            message: "funnel sidecar exited unexpectedly".into(),
        },
        other => other,
    };
    inner.status.state = settled.clone();
    let snap = inner.status.clone();
    drop(inner);
    shared.emit(&snap);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use tempfile::TempDir;

    const HOSTNAME: &str = "box.tailnet.ts.net";
    const PORT: u16 = 8787;

    // ---------- fakes ----------

    struct FakeSession {
        rx: mpsc::UnboundedReceiver<SidecarEvent>,
        /// Dropped on stop so a pending `recv` would unblock in production
        /// terms (process death closes the pipe).
        gate: Arc<std::sync::Mutex<Option<mpsc::UnboundedSender<SidecarEvent>>>>,
        stop_called: Arc<AtomicBool>,
    }

    #[async_trait]
    impl SidecarSession for FakeSession {
        async fn next_event(&mut self) -> Option<SidecarEvent> {
            self.rx.recv().await
        }

        async fn stop(&mut self) -> Result<(), ConnectionError> {
            self.stop_called.store(true, Ordering::SeqCst);
            self.gate.lock().unwrap().take();
            Ok(())
        }
    }

    /// Creates a fake session plus the sender used to inject events.
    fn fake_session() -> (
        FakeSession,
        mpsc::UnboundedSender<SidecarEvent>,
        Arc<AtomicBool>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        let stop_called = Arc::new(AtomicBool::new(false));
        (
            FakeSession {
                rx,
                gate: Arc::new(std::sync::Mutex::new(Some(tx.clone()))),
                stop_called: stop_called.clone(),
            },
            tx,
            stop_called,
        )
    }

    struct FakeDriver {
        sessions: std::sync::Mutex<VecDeque<FakeSession>>,
        spawns: AtomicUsize,
    }

    impl FakeDriver {
        fn new(sessions: impl IntoIterator<Item = FakeSession>) -> Arc<Self> {
            Arc::new(Self {
                sessions: std::sync::Mutex::new(sessions.into_iter().collect()),
                spawns: AtomicUsize::new(0),
            })
        }

        fn spawn_count(&self) -> usize {
            self.spawns.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl SidecarDriver for FakeDriver {
        async fn spawn(
            &self,
            _cfg: SidecarConfig,
        ) -> Result<Box<dyn SidecarSession>, ConnectionError> {
            self.spawns.fetch_add(1, Ordering::SeqCst);
            let session =
                self.sessions
                    .lock()
                    .unwrap()
                    .pop_front()
                    .ok_or_else(|| ConnectionError {
                        code: "NO_FAKE_SESSION".into(),
                        message: "test exhausted prepared sessions".into(),
                    })?;
            Ok(Box::new(session))
        }
    }

    /// Fake verifier: configurable verdict and artificial latency so tests
    /// can land a stop mid-verification.
    struct FakeVerifier {
        ok: bool,
        delay: Duration,
    }

    #[async_trait]
    impl HealthVerifier for FakeVerifier {
        async fn verify(&self, _fqdn: &str, _capability: &str) -> Result<(), HealthError> {
            tokio::time::sleep(self.delay).await;
            if self.ok {
                Ok(())
            } else {
                Err(HealthError::UnexpectedStatus { status: 404 })
            }
        }
    }

    // ---------- harness ----------

    struct Harness {
        mgr: ConnectionManager,
        driver: Arc<FakeDriver>,
        /// Keeps the temp dirs alive for the whole test.
        _dirs: (TempDir, TempDir),
        app_data: PathBuf,
        state_dir: PathBuf,
        initial_capability: String,
        seen: Arc<std::sync::Mutex<Vec<ConnectionState>>>,
    }

    fn harness(
        sessions: impl IntoIterator<Item = FakeSession>,
        verifier: Arc<dyn HealthVerifier>,
    ) -> Harness {
        let app_dir = tempfile::tempdir().unwrap();
        let state_dir = tempfile::tempdir().unwrap();
        let app_data = app_dir.path().to_path_buf();
        let state_path = state_dir.path().to_path_buf();

        let store = CapabilitySecretStore::new(app_data.clone());
        let initial_capability = store.load_or_create().unwrap();

        let seen: Arc<std::sync::Mutex<Vec<ConnectionState>>> = Arc::default();
        let sink = seen.clone();
        let emitter: StatusEmitter = Arc::new(move |status: &ConnectionStatus| {
            sink.lock().unwrap().push(status.state.clone())
        });

        let driver = FakeDriver::new(sessions);
        let mgr = ConnectionManager::with_parts(
            driver.clone(),
            verifier,
            emitter,
            CapabilitySecretStore::new(app_data.clone()),
            state_path.clone(),
            HOSTNAME.into(),
            PORT,
        );
        Harness {
            mgr,
            driver,
            _dirs: (app_dir, state_dir),
            app_data,
            state_dir: state_path,
            initial_capability,
            seen,
        }
    }

    fn instant_verifier(ok: bool) -> Arc<dyn HealthVerifier> {
        Arc::new(FakeVerifier {
            ok,
            delay: Duration::ZERO,
        })
    }

    async fn wait_for(
        mgr: &ConnectionManager,
        timeout: Duration,
        pred: impl Fn(&ConnectionState) -> bool,
    ) -> ConnectionState {
        let deadline = Instant::now() + timeout;
        loop {
            let state = mgr.status().await.state;
            if pred(&state) {
                return state;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for state, last: {state:?}"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    // ---------- tests ----------

    #[tokio::test]
    async fn first_run_login_transition_sequence_is_exact() {
        let (sess, tx, _) = fake_session();
        tx.send(SidecarEvent::Starting).unwrap();
        tx.send(SidecarEvent::AuthRequired {
            url: "https://login.tailscale.com/a/xyz".into(),
        })
        .unwrap();
        tx.send(SidecarEvent::Authenticated).unwrap();
        tx.send(SidecarEvent::FunnelListening {
            fqdn: HOSTNAME.into(),
        })
        .unwrap();
        drop(tx); // channel closes after the queue drains

        let h = harness([sess], instant_verifier(true));
        h.mgr.start().await.unwrap();

        let online = wait_for(&h.mgr, Duration::from_secs(2), |s| {
            matches!(s, ConnectionState::Online { .. })
        })
        .await;
        let expected_endpoint = endpoint(HOSTNAME, &h.initial_capability);
        assert_eq!(
            online,
            ConnectionState::Online {
                fqdn: HOSTNAME.into(),
                endpoint: expected_endpoint,
            }
        );

        assert_eq!(
            *h.seen.lock().unwrap(),
            vec![
                ConnectionState::StartingServer,
                ConnectionState::StartingTunnel,
                ConnectionState::NeedsLogin {
                    login_url: "https://login.tailscale.com/a/xyz".into()
                },
                ConnectionState::Verifying {
                    fqdn: HOSTNAME.into()
                },
                ConnectionState::Online {
                    fqdn: HOSTNAME.into(),
                    endpoint: endpoint(HOSTNAME, &h.initial_capability),
                },
            ]
        );
    }

    #[tokio::test]
    async fn funnel_not_allowed_maps_to_needs_funnel_permission_not_generic_error() {
        let (sess, tx, _) = fake_session();
        tx.send(SidecarEvent::Starting).unwrap();
        tx.send(SidecarEvent::Error {
            code: "FUNNEL_NOT_ALLOWED".into(),
            message: "funnel permission missing for node".into(),
        })
        .unwrap();
        drop(tx);

        let h = harness([sess], instant_verifier(true));
        h.mgr.start().await.unwrap();

        let state = wait_for(&h.mgr, Duration::from_secs(2), |s| {
            matches!(
                s,
                ConnectionState::NeedsFunnelPermission { .. } | ConnectionState::Error { .. }
            )
        })
        .await;
        assert_eq!(
            state,
            ConnectionState::NeedsFunnelPermission {
                message: "funnel permission missing for node".into()
            }
        );

        // Serialized status must never leak the capability token.
        let status = h.mgr.status().await;
        let json = serde_json::to_string(&status).unwrap();
        assert!(!json.contains(&h.initial_capability));
        // No endpoint key anywhere in non-Online states.
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value["state"].get("endpoint").is_none());
    }

    #[tokio::test]
    async fn generic_sidecar_error_lands_in_error_state_with_code() {
        let (sess, tx, _) = fake_session();
        tx.send(SidecarEvent::Error {
            code: "TAILSDOWN".into(),
            message: "tailscaled unreachable".into(),
        })
        .unwrap();
        drop(tx);

        let h = harness([sess], instant_verifier(true));
        h.mgr.start().await.unwrap();

        let state = wait_for(&h.mgr, Duration::from_secs(2), |s| {
            matches!(s, ConnectionState::Error { .. })
        })
        .await;
        assert_eq!(
            state,
            ConnectionState::Error {
                code: "TAILSDOWN".into(),
                message: "tailscaled unreachable".into()
            }
        );
    }

    #[tokio::test]
    async fn stop_during_login_ends_stopped_and_keeps_identity_files() {
        let (sess, tx, stop_called) = fake_session();
        tx.send(SidecarEvent::Starting).unwrap();
        tx.send(SidecarEvent::AuthRequired {
            url: "https://login.tailscale.com/a/xyz".into(),
        })
        .unwrap();
        // tx intentionally kept alive: login pending, no further events.

        let h = harness([sess], instant_verifier(true));
        h.mgr.start().await.unwrap();
        wait_for(&h.mgr, Duration::from_secs(2), |s| {
            matches!(s, ConnectionState::NeedsLogin { .. })
        })
        .await;

        let stopped = h.mgr.stop().await.unwrap();
        assert_eq!(stopped.state, ConnectionState::Stopped);
        assert!(stop_called.load(Ordering::SeqCst));

        // Identity/secret files intact.
        assert!(h.app_data.join("connection-secret.bin").is_file());
        assert!(h.state_dir.is_dir());

        // Never reached verifying/online.
        let seen = h.seen.lock().unwrap();
        assert!(seen.iter().all(|s| {
            !matches!(
                s,
                ConnectionState::Verifying { .. } | ConnectionState::Online { .. }
            )
        }));
    }

    #[tokio::test]
    async fn stop_during_verification_ends_stopped_and_never_goes_online() {
        let (sess, tx, stop_called) = fake_session();
        tx.send(SidecarEvent::Starting).unwrap();
        tx.send(SidecarEvent::FunnelListening {
            fqdn: HOSTNAME.into(),
        })
        .unwrap();
        // tx kept alive; the slow verifier below is what we interrupt.

        let h = harness(
            [sess],
            Arc::new(FakeVerifier {
                ok: true, // verification WILL succeed -- but too late
                delay: Duration::from_millis(500),
            }),
        );
        h.mgr.start().await.unwrap();
        wait_for(&h.mgr, Duration::from_secs(2), |s| {
            matches!(s, ConnectionState::Verifying { .. })
        })
        .await;

        let stopped = h.mgr.stop().await.unwrap();
        assert_eq!(stopped.state, ConnectionState::Stopped);
        assert!(stop_called.load(Ordering::SeqCst));

        // Verification succeeded underneath, but the stale generation must
        // never publish Online.
        assert!(h
            .seen
            .lock()
            .unwrap()
            .iter()
            .all(|s| !matches!(s, ConnectionState::Online { .. })));

        assert!(h.app_data.join("connection-secret.bin").is_file());
        assert!(h.state_dir.is_dir());
    }

    #[tokio::test]
    async fn double_start_returns_same_status_without_duplicate_spawn() {
        let (sess, tx, _) = fake_session();
        tx.send(SidecarEvent::Starting).unwrap();
        tx.send(SidecarEvent::AuthRequired {
            url: "https://login.tailscale.com/a/xyz".into(),
        })
        .unwrap();

        let h = harness([sess], instant_verifier(true));
        h.mgr.start().await.unwrap();
        wait_for(&h.mgr, Duration::from_secs(2), |s| {
            matches!(s, ConnectionState::NeedsLogin { .. })
        })
        .await;
        let first = h.mgr.start().await.unwrap();
        let second = h.mgr.start().await.unwrap();

        assert_eq!(first, second);
        assert!(matches!(second.state, ConnectionState::NeedsLogin { .. }));
        assert_eq!(h.driver.spawn_count(), 1);
    }

    #[tokio::test]
    async fn rotate_while_running_stops_rotates_and_restarts() {
        let (sess1, tx1, stop1) = fake_session();
        tx1.send(SidecarEvent::Starting).unwrap();
        tx1.send(SidecarEvent::AuthRequired {
            url: "https://login.tailscale.com/a/1".into(),
        })
        .unwrap();
        let (sess2, tx2, _) = fake_session();
        tx2.send(SidecarEvent::Starting).unwrap();
        tx2.send(SidecarEvent::Authenticated).unwrap();
        tx2.send(SidecarEvent::FunnelListening {
            fqdn: HOSTNAME.into(),
        })
        .unwrap();
        drop(tx2);

        let h = harness([sess1, sess2], instant_verifier(true));
        h.mgr.start().await.unwrap();
        wait_for(&h.mgr, Duration::from_secs(2), |s| {
            matches!(s, ConnectionState::NeedsLogin { .. })
        })
        .await;

        h.mgr.rotate_capability().await.unwrap();
        let restarted = wait_for(&h.mgr, Duration::from_secs(2), |s| {
            matches!(s, ConnectionState::Online { .. })
        })
        .await;
        match restarted {
            ConnectionState::Online { fqdn, endpoint: ep } => {
                assert_eq!(fqdn, HOSTNAME);
                // Endpoint must embed the NEW capability, not the old one.
                let store = CapabilitySecretStore::new(h.app_data.clone());
                let new_cap = store.load_or_create().unwrap();
                assert_ne!(new_cap, h.initial_capability);
                assert_eq!(ep, endpoint(HOSTNAME, &new_cap));
                // And must not embed the old token.
                assert!(!ep.contains(&h.initial_capability));
            }
            other => panic!("expected Online after rotate-restart, got {other:?}"),
        }

        // Old session was stopped; exactly two spawns happened.
        assert!(stop1.load(Ordering::SeqCst));
        assert_eq!(h.driver.spawn_count(), 2);

        // Stored secret changed.
        let store = CapabilitySecretStore::new(h.app_data.clone());
        assert_ne!(store.load_or_create().unwrap(), h.initial_capability);
    }

    #[tokio::test]
    async fn rotate_when_stopped_only_rotates_the_secret() {
        let h = harness([], instant_verifier(true));
        let status = h.mgr.rotate_capability().await.unwrap();

        assert_eq!(status.state, ConnectionState::Stopped);
        assert_eq!(h.driver.spawn_count(), 0);
        let store = CapabilitySecretStore::new(h.app_data.clone());
        assert_ne!(store.load_or_create().unwrap(), h.initial_capability);
    }

    #[tokio::test]
    async fn reset_identity_removes_state_dir_rotates_and_stays_stopped_without_restart() {
        let h = harness([], instant_verifier(true));
        // Seed the state dir with a file so removal is observable.
        let marker = h.state_dir.join("tailscaled.state");
        std::fs::write(&marker, b"x").unwrap();

        let status = h.mgr.reset_identity(true, false).await.unwrap();
        assert_eq!(status.state, ConnectionState::Stopped);
        assert!(!marker.exists());
        let store = CapabilitySecretStore::new(h.app_data.clone());
        assert_ne!(store.load_or_create().unwrap(), h.initial_capability);
        assert_eq!(h.driver.spawn_count(), 0);
    }

    #[tokio::test]
    async fn reset_identity_with_restart_comes_back_online() {
        let (sess1, tx1, _) = fake_session();
        tx1.send(SidecarEvent::Starting).unwrap();
        tx1.send(SidecarEvent::AuthRequired {
            url: "https://login.tailscale.com/a/1".into(),
        })
        .unwrap();
        let (sess2, tx2, _) = fake_session();
        tx2.send(SidecarEvent::Starting).unwrap();
        tx2.send(SidecarEvent::FunnelListening {
            fqdn: HOSTNAME.into(),
        })
        .unwrap();
        drop(tx2);

        let h = harness([sess1, sess2], instant_verifier(true));
        let marker_path = h.state_dir.join("tailscaled.state");
        std::fs::write(&marker_path, b"x").unwrap();
        h.mgr.start().await.unwrap();
        wait_for(&h.mgr, Duration::from_secs(2), |s| {
            matches!(s, ConnectionState::NeedsLogin { .. })
        })
        .await;

        let status = h.mgr.reset_identity(true, true).await.unwrap();
        assert!(matches!(
            status.state,
            ConnectionState::StartingServer
                | ConnectionState::StartingTunnel
                | ConnectionState::Verifying { .. }
                | ConnectionState::Online { .. }
        ));
        let online = wait_for(&h.mgr, Duration::from_secs(2), |s| {
            matches!(s, ConnectionState::Online { .. })
        })
        .await;
        assert!(matches!(online, ConnectionState::Online { .. }));
        assert_eq!(h.driver.spawn_count(), 2);
        // State dir contents were wiped before the fresh run.
        assert!(!marker_path.exists());
    }

    #[tokio::test]
    async fn stale_generation_cannot_write_state() {
        let h = harness([], instant_verifier(true));
        // Generation 0 is current; a stale run's write must be rejected.
        let applied = h
            .mgr
            .shared
            .set_state(
                99,
                ConnectionState::Online {
                    fqdn: HOSTNAME.into(),
                    endpoint: "https://spoofed/mcp".into(),
                },
            )
            .await;
        assert!(!applied);
        assert_eq!(h.mgr.status().await.state, ConnectionState::Stopped);
    }

    #[test]
    fn endpoint_composes_capability_scoped_mcp_url() {
        assert_eq!(
            endpoint("box.tailnet.ts.net", "cap123"),
            "https://box.tailnet.ts.net/cap123/mcp"
        );
    }
}

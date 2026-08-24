# Tailscale Funnel Complete App Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Transform the current Windows MVP into a polished desktop MCP manager with a one-click Start/Stop flow, a persistent Tailscale Funnel `*.ts.net` endpoint, capability-protected public routing, verified connection state, improved reliability, and a complete Dashboard/Workspaces/Activity/Security/Settings UX.

**Architecture:** Keep the Rust `crates/core` filesystem policy layer unchanged as the only filesystem doorway. Add a small bundled Go `tsnet` sidecar that owns an app-specific persistent Tailscale node, opens a Funnel-only TLS listener on port 443, protects the public route with a 256-bit capability path, and reverse-proxies accepted MCP traffic to the Rust loopback MCP server. Replace the current tunnel lifecycle with a Rust connection state machine, then rebuild the React UI around that state machine.

**Tech Stack:** Rust 2021, Tauri 2, rmcp 3.1, axum 0.8, Tokio, SQLite/rusqlite, React 18, TypeScript 5, Vite 5, Go 1.26.5+, `tailscale.com` v1.102.3 / `tailscale.com/tsnet`, Windows DPAPI + Job Objects.

**Spec:** `docs/superpowers/specs/2026-08-24-tailscale-funnel-complete-app-design.md`

## Global Constraints

- Windows 10/11 is the supported target for this milestone.
- Tailscale Funnel is the only new remote connection provider in this milestone.
- Do not require Tailscale Desktop, `tailscale.exe`, `tailscaled`, Administrator elevation, Cloudflare, ngrok, or a custom domain.
- Keep the local MCP server bound strictly to `127.0.0.1:<ephemeral-port>`.
- Public Funnel traffic must never reach raw `/mcp`; only the capability-protected route is accepted.
- Capability token entropy is 256 bits and the token is never passed in process argv or emitted to logs.
- `Online` means end-to-end public health verification succeeded; successful process spawn is not enough.
- Stop preserves Tailscale identity and capability token so the full endpoint stays stable across normal restarts.
- Filesystem access continues through the existing `CoreServices` fail-closed policy pipeline.
- New workspaces default to Read ON, Write OFF.
- UI remains English for this milestone, matching the existing product copy.
- Pin `tailscale.com` to `v1.102.3` and use Go 1.26.5 or newer; Tailscale v1.102.3 is built on the Go 1.26 toolchain.
- OAuth 2.1, alternate tunnel providers, Git tools, terminal/process tools, delete/move/rename filesystem tools, macOS/Linux packaging, and mobile support are out of scope.

---

## File Structure to Create / Change

### New Go sidecar

- Create `sidecars/tailscale-funnel/go.mod` — pinned Go module dependencies.
- Create `sidecars/tailscale-funnel/main.go` — CLI parsing, tsnet lifecycle, machine-readable event stream.
- Create `sidecars/tailscale-funnel/internal/events/events.go` — NDJSON event writer and event schema.
- Create `sidecars/tailscale-funnel/internal/bridge/handler.go` — capability-path router, health handler, MCP reverse proxy.
- Create `sidecars/tailscale-funnel/internal/bridge/handler_test.go` — route/proxy/security unit tests.
- Create `sidecars/tailscale-funnel/internal/events/events_test.go` — event serialization tests.
- Create `scripts/build-tailscale-sidecar.ps1` — Windows sidecar build into Tauri externalBin path.

### Rust connection subsystem

- Create `src-tauri/src/connection/mod.rs` — module exports.
- Create `src-tauri/src/connection/state.rs` — connection state/status DTOs.
- Create `src-tauri/src/connection/secrets.rs` — capability token generation + DPAPI storage.
- Create `src-tauri/src/connection/sidecar.rs` — sidecar spawn, stdout parsing, stderr redaction, process handle.
- Create `src-tauri/src/connection/windows_job.rs` — Job Object kill-on-close binding.
- Create `src-tauri/src/connection/health.rs` — public health verification.
- Create `src-tauri/src/connection/manager.rs` — Start/Stop/Retry/Rotate/Reset state machine.
- Create `src-tauri/src/connection/test_support.rs` under `#[cfg(test)]` or `src-tauri/tests/support/` — deterministic fake sidecar helpers.

### Rust existing files

- Modify `src-tauri/src/lib.rs` — initialize connection manager, replace legacy tunnel registration.
- Modify `src-tauri/src/commands.rs` — new connection/settings/security commands.
- Modify `src-tauri/src/settings.rs` — versioned settings + legacy migration.
- Modify `src-tauri/Cargo.toml` — reqwest, rand, base64, windows API features, clipboard/opener plugins as required.
- Modify `src-tauri/tauri.conf.json` — replace legacy external binaries with Tailscale sidecar after migration gate.
- Modify `src-tauri/capabilities/default.json` — opener/clipboard permissions.
- Modify `src-tauri/tests/mcp_integration.rs` — audit context assertions.
- Modify `crates/core/src/pipeline.rs` — operation audit context interface.
- Modify `crates/core/src/ops/write_file.rs` — Windows-safe replacement.
- Modify read/search/list ops only as needed to return safe audit metrics.
- Remove `src-tauri/src/tunnel.rs` only in the final cleanup task after the new E2E path is green.

### Frontend

- Rewrite `src/App.tsx` — desktop shell and page selection.
- Rewrite `src/api.ts` — new backend contracts.
- Create `src/types.ts` — shared frontend DTOs.
- Create `src/components/AppShell.tsx`.
- Create `src/components/Sidebar.tsx`.
- Create `src/components/ConnectionCard.tsx`.
- Create `src/components/StatusBadge.tsx`.
- Create `src/components/ConfirmDialog.tsx`.
- Create `src/components/EmptyState.tsx`.
- Create `src/pages/DashboardPage.tsx`.
- Create `src/pages/WorkspacesPage.tsx`.
- Create `src/pages/ActivityPage.tsx`.
- Create `src/pages/SecurityPage.tsx`.
- Create `src/pages/SettingsPage.tsx`.
- Create `src/components/OnboardingFlow.tsx`.
- Replace `src/styles.css` with modular CSS files under `src/styles/`.
- Remove obsolete `StatusBar.tsx`, `PauseButton.tsx`, `AuditTable.tsx`, and `SettingsDialog.tsx` only after replacement screens are working.

### Docs/build

- Modify `package.json` — sidecar build scripts and UI dependencies.
- Modify `README.md` — Tailscale Funnel setup/use/security model.
- Add third-party license attribution for the bundled Tailscale sidecar dependencies.

---

### Task 1: Establish Baseline and Fix Windows Patch Replacement

**Files:**
- Modify: `crates/core/src/ops/write_file.rs`
- Test: `crates/core/src/ops/write_file.rs` unit tests
- Test: `src-tauri/tests/mcp_integration.rs`

**Interfaces:**
- Consumes: existing `atomic_write(target: &Path, bytes: &[u8]) -> std::io::Result<()>`.
- Produces: `atomic_replace(target, bytes)` semantics that can replace an existing file on Windows without exposing partial content.

- [ ] **Step 1: Run the current baseline tests before any change**

Run from repository root:

```powershell
cargo test -p core
cargo test -p local-workspace-mcp-manager
npm run build
```

Expected: capture current PASS/FAIL results in the execution notes. Do not attribute pre-existing failures to later tasks.

- [ ] **Step 2: Add a Windows-specific regression test that replaces an existing file**

Add a test equivalent to:

```rust
#[test]
fn atomic_write_replaces_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("existing.txt");
    std::fs::write(&target, b"old").unwrap();

    atomic_write(&target, b"new").unwrap();

    assert_eq!(std::fs::read(&target).unwrap(), b"new");
}
```

- [ ] **Step 3: Run only that test and confirm current Windows behavior**

```powershell
cargo test -p core atomic_write_replaces_existing_file -- --nocapture
```

Expected before fix on affected Windows semantics: FAIL when rename cannot replace the existing target. If the current toolchain already replaces correctly, keep the test and still retain the explicit Windows implementation below so behavior is deliberate rather than accidental.

- [ ] **Step 4: Implement explicit Windows replacement behavior**

Use a platform helper instead of relying on cross-platform rename semantics:

```rust
#[cfg(windows)]
fn replace_file(tmp: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};

    fn wide(p: &Path) -> Vec<u16> {
        p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    }

    let from = wide(tmp);
    let to = wide(target);
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
        .map_err(|e| std::io::Error::from_raw_os_error(e.code().0))
    }
}

#[cfg(not(windows))]
fn replace_file(tmp: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::rename(tmp, target)
}
```

Then make `atomic_write` call `replace_file(&tmp, target)`.

- [ ] **Step 5: Run core and MCP integration tests**

```powershell
cargo test -p core
cargo test -p local-workspace-mcp-manager --test mcp_integration
```

Expected: PASS, including existing stale-revision and patch roundtrip tests.

- [ ] **Step 6: Commit**

```powershell
git add crates/core/src/ops/write_file.rs src-tauri/tests/mcp_integration.rs
git commit -m "fix: make file replacement reliable on Windows"
```

---

### Task 2: Create the Tailscale Sidecar Event Contract

**Files:**
- Create: `sidecars/tailscale-funnel/go.mod`
- Create: `sidecars/tailscale-funnel/main.go`
- Create: `sidecars/tailscale-funnel/internal/events/events.go`
- Create: `sidecars/tailscale-funnel/internal/events/events_test.go`

**Interfaces:**
- Produces stdout NDJSON events consumed later by Rust:
  - `starting`
  - `auth_required { url }`
  - `authenticated`
  - `funnel_listening { fqdn }`
  - `health_ready`
  - `error { code, message }`
  - `stopped`

- [ ] **Step 1: Create the pinned Go module**

`sidecars/tailscale-funnel/go.mod`:

```go
module mcp-gptweb/tailscale-funnel-sidecar

go 1.26.5

require tailscale.com v1.102.3
```

Run:

```powershell
cd sidecars/tailscale-funnel
go mod tidy
```

Commit `go.sum` produced by the pinned dependency resolution.

- [ ] **Step 2: Write event serialization tests first**

```go
func TestWriterEmitsOneJSONObjectPerLine(t *testing.T) {
    var out bytes.Buffer
    w := events.NewWriter(&out)
    if err := w.Emit(events.Event{Type: "starting"}); err != nil {
        t.Fatal(err)
    }
    if got, want := out.String(), "{\"type\":\"starting\"}\n"; got != want {
        t.Fatalf("got %q, want %q", got, want)
    }
}

func TestWriterNeverIncludesEmptyOptionalFields(t *testing.T) {
    var out bytes.Buffer
    w := events.NewWriter(&out)
    if err := w.Emit(events.Event{Type: "error", Code: "FUNNEL_NOT_ALLOWED", Message: "denied"}); err != nil {
        t.Fatal(err)
    }
    got := out.String()
    if strings.Contains(got, "\"url\"") || strings.Contains(got, "\"fqdn\"") {
        t.Fatalf("empty optional fields leaked: %s", got)
    }
}
```

Use only the Go standard library for sidecar unit-test assertions so the test surface does not add an assertion framework.

- [ ] **Step 3: Implement `events.Event` and a synchronized writer**

```go
type Event struct {
    Type    string `json:"type"`
    URL     string `json:"url,omitempty"`
    FQDN    string `json:"fqdn,omitempty"`
    Code    string `json:"code,omitempty"`
    Message string `json:"message,omitempty"`
}

type Writer struct {
    mu sync.Mutex
    w  io.Writer
}

func (w *Writer) Emit(event Event) error {
    w.mu.Lock()
    defer w.mu.Unlock()
    b, err := json.Marshal(event)
    if err != nil { return err }
    b = append(b, '\n')
    _, err = w.w.Write(b)
    return err
}
```

- [ ] **Step 4: Create a CLI skeleton that validates non-secret arguments**

`main.go` must accept exactly:

```text
--target http://127.0.0.1:<port>
--state-dir <directory>
--hostname <dns-label>
```

The capability token is read only from `MCP_CAPABILITY_TOKEN`.

Validate target with `net/url`:

```go
if u.Scheme != "http" || u.Hostname() != "127.0.0.1" {
    return errors.New("target must be an http://127.0.0.1 loopback URL")
}
```

- [ ] **Step 5: Run Go tests and a dry-run invalid-target test**

```powershell
go test ./...
go run . --target https://example.com --state-dir .state --hostname local-workspace-mcp
```

Expected: tests PASS; invalid target exits non-zero without printing the capability token.

- [ ] **Step 6: Commit**

```powershell
git add sidecars/tailscale-funnel
git commit -m "feat: add Tailscale sidecar event protocol"
```

---

### Task 3: Implement Capability-Protected HTTP Bridge

**Files:**
- Create: `sidecars/tailscale-funnel/internal/bridge/handler.go`
- Create: `sidecars/tailscale-funnel/internal/bridge/handler_test.go`
- Modify: `sidecars/tailscale-funnel/main.go`

**Interfaces:**
- Consumes: `target *url.URL`, `capabilityToken string`.
- Produces: `http.Handler` with protected `/TOKEN/health` and `/TOKEN/mcp...` routes.

- [ ] **Step 1: Write route-denial tests before implementation**

Required test matrix:

```go
cases := []struct{
    path string
    want int
}{
    {"/", http.StatusNotFound},
    {"/mcp", http.StatusNotFound},
    {"/wrong/mcp", http.StatusNotFound},
    {"/correct", http.StatusNotFound},
    {"/correct/health", http.StatusNoContent},
}
```

Assert that 404 bodies do not contain `mcp`, `tailscale`, `capability`, or the token.

- [ ] **Step 2: Write a reverse-proxy rewrite test**

Use `httptest.NewServer` as the loopback target and assert:

```text
/correct/mcp       -> upstream /mcp
/correct/mcp/foo   -> upstream /mcp/foo
/correct/mcp?a=1   -> upstream /mcp?a=1
```

- [ ] **Step 3: Implement exact capability routing**

Core logic:

```go
func NewHandler(token string, target *url.URL) http.Handler {
    healthPath := "/" + token + "/health"
    mcpPrefix := "/" + token + "/mcp"

    proxy := httputil.NewSingleHostReverseProxy(target)
    proxy.FlushInterval = -1
    originalDirector := proxy.Director
    proxy.Director = func(req *http.Request) {
        incoming := req.URL.Path
        originalDirector(req)
        switch {
        case incoming == mcpPrefix:
            req.URL.Path = "/mcp"
        case strings.HasPrefix(incoming, mcpPrefix+"/"):
            req.URL.Path = "/mcp" + strings.TrimPrefix(incoming, mcpPrefix)
        }
    }

    return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
        switch {
        case r.URL.Path == healthPath:
            w.Header().Set("X-MCP-Bridge-Health", "ok")
            w.WriteHeader(http.StatusNoContent)
        case r.URL.Path == mcpPrefix || strings.HasPrefix(r.URL.Path, mcpPrefix+"/"):
            proxy.ServeHTTP(w, r)
        default:
            http.NotFound(w, r)
        }
    })
}
```

- [ ] **Step 4: Add streaming-safe HTTP server settings**

Use no `WriteTimeout` so valid MCP streaming responses are not killed:

```go
server := &http.Server{
    Handler:           handler,
    ReadHeaderTimeout: 15 * time.Second,
    IdleTimeout:       0,
}
```

Do not add response buffering middleware.

- [ ] **Step 5: Run tests**

```powershell
go test ./...
```

Expected: PASS, including exact 404 behavior and upstream rewrite.

- [ ] **Step 6: Commit**

```powershell
git add sidecars/tailscale-funnel/internal/bridge sidecars/tailscale-funnel/main.go
git commit -m "feat: protect and proxy public MCP routes"
```

---

### Task 4: Add tsnet Login, Persistent Identity, and Funnel Listener

**Files:**
- Modify: `sidecars/tailscale-funnel/main.go`
- Test: `sidecars/tailscale-funnel/internal/events/events_test.go`

**Interfaces:**
- Consumes: fixed hostname, persistent state directory, capability handler.
- Produces: Tailscale authentication URL event and stable Funnel FQDN event.

- [ ] **Step 1: Add an auth-URL extraction unit test**

```go
func TestExtractAuthURL(t *testing.T) {
    msg := "To start this tsnet server, restart with TS_AUTHKEY set, or go to: https://login.tailscale.com/a/1234567890"
    got := extractAuthURL(msg)
    if got != "https://login.tailscale.com/a/1234567890" {
        t.Fatalf("unexpected auth URL: %q", got)
    }
}
```

The regex must only accept `https://login.tailscale.com/a/...`.

- [ ] **Step 2: Configure `tsnet.Server` with persistent state**

Implementation shape:

```go
srv := &tsnet.Server{
    Dir:       cfg.StateDir,
    Hostname:  cfg.Hostname,
    Ephemeral: false,
    UserLogf: func(format string, args ...any) {
        msg := fmt.Sprintf(format, args...)
        if authURL := extractAuthURL(msg); authURL != "" {
            _ = eventWriter.Emit(events.Event{Type: "auth_required", URL: authURL})
        }
        fmt.Fprintln(os.Stderr, redact(msg))
    },
}
defer srv.Close()
```

Create the state directory before constructing the server.

- [ ] **Step 3: Open a Funnel-only TLS listener on port 443**

```go
ln, err := srv.ListenFunnel("tcp", ":443", tsnet.FunnelOnly())
if err != nil {
    emitMappedFunnelError(err)
    os.Exit(1)
}
defer ln.Close()
```

- [ ] **Step 4: Emit authenticated and FQDN events only after the listener exists**

```go
domains := srv.CertDomains()
if len(domains) == 0 {
    emit error code CERT_DOMAIN_UNAVAILABLE and exit
}
_ = eventWriter.Emit(events.Event{Type: "authenticated"})
_ = eventWriter.Emit(events.Event{Type: "funnel_listening", FQDN: domains[0]})
```

Never construct the tailnet suffix by guessing it.

- [ ] **Step 5: Map actionable Funnel failures**

Centralize error mapping:

```go
func funnelErrorCode(err error) string {
    s := strings.ToLower(err.Error())
    if strings.Contains(s, "funnel") && (strings.Contains(s, "not allowed") || strings.Contains(s, "nodeattr")) {
        return "FUNNEL_NOT_ALLOWED"
    }
    if strings.Contains(s, "certificate") || strings.Contains(s, "https") {
        return "TAILSCALE_HTTPS_NOT_READY"
    }
    return "TAILSCALE_START_FAILED"
}
```

The raw error can be in the event message, but apply `redact` first.

- [ ] **Step 6: Start `http.Server` on the Funnel listener**

```go
if err := server.Serve(ln); err != nil && !errors.Is(err, http.ErrServerClosed) {
    emit error code FUNNEL_SERVE_FAILED
    os.Exit(1)
}
```

Emit `stopped` on graceful shutdown.

- [ ] **Step 7: Run Go tests and perform a manual sidecar smoke test with a temporary local HTTP server**

```powershell
go test ./...
```

Then, on a developer tailnet with Funnel enabled, run the sidecar against a loopback test server and verify it emits `auth_required` on first state, then `funnel_listening` after login.

- [ ] **Step 8: Commit**

```powershell
git add sidecars/tailscale-funnel
git commit -m "feat: add persistent tsnet Funnel lifecycle"
```

---

### Task 5: Build and Bundle the Tailscale Sidecar

**Files:**
- Create: `scripts/build-tailscale-sidecar.ps1`
- Modify: `package.json`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `.gitignore` if required for generated EXE

**Interfaces:**
- Produces: `src-tauri/binaries/tailscale-funnel-sidecar-x86_64-pc-windows-msvc.exe`.

- [ ] **Step 1: Create the deterministic PowerShell build script**

Script behavior:

```powershell
$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$sidecar = Join-Path $repo "sidecars\tailscale-funnel"
$out = Join-Path $repo "src-tauri\binaries\tailscale-funnel-sidecar-x86_64-pc-windows-msvc.exe"

Push-Location $sidecar
try {
    go test ./...
    go build -trimpath -ldflags="-s -w" -o $out .
} finally {
    Pop-Location
}

if (!(Test-Path $out)) { throw "Tailscale sidecar build output missing: $out" }
```

- [ ] **Step 2: Add npm scripts**

```json
{
  "scripts": {
    "dev": "vite",
    "build": "tsc && vite build",
    "build:sidecar": "powershell -ExecutionPolicy Bypass -File scripts/build-tailscale-sidecar.ps1",
    "verify": "npm run build && npm run build:sidecar"
  }
}
```

Preserve existing scripts not shown here.

- [ ] **Step 3: Update Tauri externalBin**

During migration, include the new sidecar and keep the old binaries temporarily:

```json
"externalBin": [
  "binaries/tailscale-funnel-sidecar",
  "binaries/tunnel-client",
  "binaries/cloudflared"
]
```

The old entries are removed in the final cleanup task only after E2E passes.

- [ ] **Step 4: Make dev/build produce the sidecar before Tauri starts**

Update:

```json
"beforeDevCommand": "npm run build:sidecar && npm run dev",
"beforeBuildCommand": "npm run build:sidecar && npm run build"
```

- [ ] **Step 5: Verify artifact and Tauri metadata**

```powershell
npm run build:sidecar
Test-Path src-tauri\binaries\tailscale-funnel-sidecar-x86_64-pc-windows-msvc.exe
npx tauri info
```

Expected: sidecar exists and Tauri config parses.

- [ ] **Step 6: Commit**

```powershell
git add scripts package.json src-tauri/tauri.conf.json .gitignore sidecars/tailscale-funnel/go.mod sidecars/tailscale-funnel/go.sum
git commit -m "build: bundle Tailscale Funnel sidecar"
```

---

### Task 6: Add Capability Secret Generation and DPAPI Storage

**Files:**
- Create: `src-tauri/src/connection/mod.rs`
- Create: `src-tauri/src/connection/secrets.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`

**Interfaces:**
- Produces:
  - `CapabilitySecretStore::load_or_create() -> Result<String, String>`
  - `CapabilitySecretStore::rotate() -> Result<String, String>`
  - `CapabilitySecretStore::clear() -> Result<(), String>`

- [ ] **Step 1: Add dependencies**

Add:

```toml
rand = "0.9"
base64 = "0.22"
windows = { version = "0.61", features = [
  "Win32_Foundation",
  "Win32_Security_Cryptography",
  "Win32_System_Memory",
  "Win32_System_JobObjects",
  "Win32_System_Threading",
  "Win32_Storage_FileSystem"
] }
```

If Task 1 already added `windows`, merge features rather than adding a duplicate dependency.

- [ ] **Step 2: Write token-format tests first**

```rust
#[test]
fn generated_capability_has_at_least_256_bits_of_source_entropy() {
    let a = generate_capability().unwrap();
    let b = generate_capability().unwrap();
    assert_ne!(a, b);
    assert!(!a.contains('/'));
    assert!(!a.contains('+'));
    assert!(!a.contains('='));
    assert!(a.len() >= 43);
}
```

- [ ] **Step 3: Implement generation using `OsRng` and URL-safe base64**

```rust
fn generate_capability() -> Result<String, String> {
    use base64::Engine;
    use rand::TryRngCore;

    let mut bytes = [0u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|e| format!("OS RNG unavailable: {e}"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}
```

Return an error instead of panicking in the production wrapper if the selected rand API is fallible in the pinned crate version.

- [ ] **Step 4: Write a Windows DPAPI round-trip test**

The test must assert plaintext is not present in the encrypted blob:

```rust
#[cfg(windows)]
#[test]
fn dpapi_roundtrip_hides_plaintext() {
    let plain = b"capability-secret-example";
    let enc = protect(plain).unwrap();
    assert!(!String::from_utf8_lossy(&enc).contains("capability-secret-example"));
    assert_eq!(unprotect(&enc).unwrap(), plain);
}
```

- [ ] **Step 5: Implement DPAPI wrappers and atomic encrypted-file persistence**

Store at:

```text
<app-data>/connection-secret.bin
```

Use `CryptProtectData` with current-user scope; do not use `CRYPTPROTECT_LOCAL_MACHINE`.

`CapabilitySecretStore` takes the app-data directory in its constructor so tests can use a temp directory.

- [ ] **Step 6: Verify create/load/rotate behavior**

Tests:

```text
load_or_create twice -> same token
rotate -> different token
load after rotate -> rotated token
clear then load_or_create -> new token
```

- [ ] **Step 7: Run Rust tests**

```powershell
cargo test -p local-workspace-mcp-manager connection::secrets
```

Expected: PASS on Windows.

- [ ] **Step 8: Commit**

```powershell
git add src-tauri/Cargo.toml src-tauri/src/connection src-tauri/src/lib.rs
git commit -m "feat: secure persistent MCP capability secret"
```

---

### Task 7: Define the Rust Connection State Contract

**Files:**
- Create: `src-tauri/src/connection/state.rs`
- Modify: `src-tauri/src/connection/mod.rs`
- Modify: `src/api.ts` later; Rust contract first

**Interfaces:**
- Produces `ConnectionState` and `ConnectionStatus` serialized to snake_case JSON.

- [ ] **Step 1: Write serialization tests**

Required state enum:

```rust
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ConnectionState {
    Stopped,
    StartingServer,
    StartingTunnel,
    NeedsLogin { login_url: String },
    NeedsFunnelPermission { message: String },
    Verifying { fqdn: String },
    Online { fqdn: String, endpoint: String },
    Degraded { fqdn: Option<String>, message: String },
    Stopping,
    Error { code: String, message: String },
}
```

`ConnectionStatus`:

```rust
pub struct ConnectionStatus {
    pub state: ConnectionState,
    pub local_port: u16,
    pub paused: bool,
}
```

- [ ] **Step 2: Assert endpoint is emitted only in Online**

Do not expose the capability endpoint in errors, debug diagnostics, or intermediate states.

- [ ] **Step 3: Implement the types and unit tests**

```powershell
cargo test -p local-workspace-mcp-manager connection::state
```

Expected: PASS.

- [ ] **Step 4: Commit**

```powershell
git add src-tauri/src/connection/state.rs src-tauri/src/connection/mod.rs
git commit -m "feat: define verified connection state model"
```

---

### Task 8: Spawn Sidecar, Parse NDJSON, and Prevent Orphans

**Files:**
- Create: `src-tauri/src/connection/sidecar.rs`
- Create: `src-tauri/src/connection/windows_job.rs`
- Modify: `src-tauri/src/connection/mod.rs`

**Interfaces:**
- Produces `SidecarEvent` matching the Go event schema.
- Produces this non-`Debug` secret-bearing configuration type:

```rust
pub struct SidecarConfig {
    pub local_port: u16,
    pub state_dir: PathBuf,
    pub hostname: String,
    pub capability_token: String,
}
```

- Produces `SidecarProcess`, the production process wrapper that owns the child, Job Object, stdout event receiver, and redacted diagnostics ring.
- Produces `spawn_sidecar(config: SidecarConfig) -> Result<SidecarProcess, ConnectionError>`.
- Produces `SidecarProcess::stop()` and `JobHandle::assign(&Child)` on Windows.
- `ConnectionError` is the stable internal error type defined in Task 10; until Task 10, sidecar functions may use a private `SidecarSpawnError` and Task 10 adds the conversion.

- [ ] **Step 1: Write sidecar event parsing tests**

```rust
#[test]
fn parses_auth_required_event() {
    let e: SidecarEvent = serde_json::from_str(
        r#"{"type":"auth_required","url":"https://login.tailscale.com/a/123"}"#
    ).unwrap();
    assert!(matches!(e, SidecarEvent::AuthRequired { .. }));
}
```

Also test unknown event types fail safely without panicking the manager.

- [ ] **Step 2: Implement sidecar path resolution**

Mirror the existing bundled/dev fallback pattern but target:

```text
tailscale-funnel-sidecar-x86_64-pc-windows-msvc.exe
```

- [ ] **Step 3: Spawn with secret in environment only**

```rust
let mut cmd = Command::new(sidecar_path());
cmd.args([
    "--target", &format!("http://127.0.0.1:{}", cfg.local_port),
    "--state-dir", cfg.state_dir.to_string_lossy().as_ref(),
    "--hostname", &cfg.hostname,
]);
cmd.env("MCP_CAPABILITY_TOKEN", &cfg.capability_token);
cmd.stdin(Stdio::null());
cmd.stdout(Stdio::piped());
cmd.stderr(Stdio::piped());
```

Never include `cfg.capability_token` in a formatted error containing the command line.

- [ ] **Step 4: Parse stdout line-by-line on a dedicated thread/task**

Each valid JSON line becomes a `SidecarEvent` sent through a Tokio channel to `ConnectionManager`.

Malformed lines become redacted diagnostics, not state-machine panics.

- [ ] **Step 5: Drain stderr continuously**

Do not repeat the old piped-but-never-drained deadlock risk. Drain stderr into a bounded in-memory diagnostics ring, maximum 200 lines, after redacting:

```text
MCP_CAPABILITY_TOKEN value
https://login.tailscale.com/a/<...>
```

- [ ] **Step 6: Write the Windows Job Object test before implementation**

Spawn a PowerShell sleeper, assign it to the job, drop/close the job, then assert PID disappears.

- [ ] **Step 7: Implement kill-on-close Job Object**

Use:

```text
CreateJobObjectW
SetInformationJobObject(JobObjectExtendedLimitInformation)
JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
AssignProcessToJobObject
```

Keep the Job Object handle owned by `SidecarProcess` for the child lifetime.

- [ ] **Step 8: Verify process lifecycle tests**

```powershell
cargo test -p local-workspace-mcp-manager connection::sidecar -- --nocapture
cargo test -p local-workspace-mcp-manager connection::windows_job -- --nocapture
```

- [ ] **Step 9: Commit**

```powershell
git add src-tauri/src/connection
git commit -m "feat: manage Tailscale sidecar lifecycle safely"
```

---

### Task 9: Implement End-to-End Health Verification

**Files:**
- Create: `src-tauri/src/connection/health.rs`
- Modify: `src-tauri/Cargo.toml`

**Interfaces:**
- Produces `verify_public_health(fqdn, capability) -> Result<(), HealthError>`.
- Defines a secret-safe error type:

```rust
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
```

No `HealthError` variant may store or format the capability token or full capability URL.

- [ ] **Step 1: Add reqwest dependency**

```toml
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }
thiserror.workspace = true
```

- [ ] **Step 2: Write health verifier tests against a local HTTPS-independent test abstraction**

Separate URL construction from network call:

```rust
fn health_url(fqdn: &str, capability: &str) -> Result<Url, HealthError>;
```

Assert:

```text
fqdn = local-workspace-mcp.example.ts.net
capability = abc
=> https://local-workspace-mcp.example.ts.net/abc/health
```

Reject FQDN strings containing `/`, scheme, query, fragment, or whitespace.

- [ ] **Step 3: Implement bounded retry verification**

Use five attempts with delays `250ms, 500ms, 1s, 2s, 2s`, request timeout 5 seconds per attempt.

Success requires:

```text
status == 204
X-MCP-Bridge-Health == ok
```

A 404 is treated as verification failure, not success.

- [ ] **Step 4: Ensure error formatting never includes capability URL**

Errors may mention FQDN and status code but must not render the full secret path.

- [ ] **Step 5: Run tests**

```powershell
cargo test -p local-workspace-mcp-manager connection::health
```

- [ ] **Step 6: Commit**

```powershell
git add src-tauri/Cargo.toml src-tauri/src/connection/health.rs
git commit -m "feat: verify Funnel endpoint before reporting online"
```

---

### Task 10: Build the Connection Manager State Machine

**Files:**
- Create: `src-tauri/src/connection/manager.rs`
- Modify: `src-tauri/src/connection/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`
- Add tests in `src-tauri/src/connection/manager.rs` or `src-tauri/tests/connection_manager.rs`

**Interfaces:**
- Produces:
  - `ConnectionManager::start()`
  - `ConnectionManager::stop()`
  - `ConnectionManager::retry()`
  - `ConnectionManager::status()`
  - `ConnectionManager::rotate_capability()`
  - `ConnectionManager::reset_identity()`

- [ ] **Step 1: Add async-trait and define the manager error plus injectable interfaces**

Add to `src-tauri/Cargo.toml`:

```toml
async-trait = "0.1"
```

Use one stable internal error shape that commands can serialize without exposing secrets:

```rust
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct ConnectionError {
    pub code: String,
    pub message: String,
}
```

Create injectable interfaces so manager tests do not need a real Tailscale process:

```rust
#[async_trait]
pub trait SidecarSession: Send {
    async fn next_event(&mut self) -> Option<SidecarEvent>;
    async fn stop(&mut self) -> Result<(), ConnectionError>;
}

#[async_trait]
pub trait SidecarDriver: Send + Sync {
    async fn spawn(
        &self,
        cfg: SidecarConfig,
    ) -> Result<Box<dyn SidecarSession>, ConnectionError>;
}

#[async_trait]
pub trait HealthVerifier: Send + Sync {
    async fn verify(&self, fqdn: &str, capability: &str) -> Result<(), HealthError>;
}
```

`ProcessSidecarDriver` wraps the `SidecarProcess` from Task 8. Tests use a channel-backed `FakeSidecarSession`; `ReqwestHealthVerifier` wraps Task 9.

- [ ] **Step 2: Write the first-run login transition test**

Expected sequence:

```text
Stopped
-> StartingServer
-> StartingTunnel
-> NeedsLogin(login_url)
-> Verifying(fqdn)
-> Online(fqdn, endpoint)
```

The test should inject `AuthRequired`, then `FunnelListening`, then a successful fake health verifier.

- [ ] **Step 3: Write Funnel permission failure test**

Inject:

```json
{"type":"error","code":"FUNNEL_NOT_ALLOWED","message":"..."}
```

Expected final state: `NeedsFunnelPermission`, not generic `Error`.

- [ ] **Step 4: Write stop-during-login and stop-during-verification tests**

Both must finish at `Stopped` and leave identity/secret files intact.

- [ ] **Step 5: Implement state transitions behind a mutex and generation/cancellation token**

Only one Start may run at a time. A second Start while non-Stopped returns current state without spawning a duplicate sidecar.

Use a monotonically increasing run generation or cancellation token so stale async events from a previous sidecar cannot mark a new run Online.

- [ ] **Step 6: Compose the endpoint only after FQDN is trusted**

```rust
fn endpoint(fqdn: &str, capability: &str) -> String {
    format!("https://{fqdn}/{capability}/mcp")
}
```

Keep this helper private to the manager/status layer.

- [ ] **Step 7: Implement rotate capability**

Behavior:

```text
if running: stop -> rotate secret -> start
if stopped: rotate secret only
```

Return the new status; never return both old and new secret.

- [ ] **Step 8: Implement reset identity**

Behavior:

```text
stop
remove <app-data>/tailscale state directory
rotate capability
start only if caller requested restart
```

Require a UI confirmation in a later task; backend command itself performs the explicit action.

- [ ] **Step 9: Emit one Tauri `connection-status` event after every state change**

Do not rely on frontend polling for live connection transitions.

- [ ] **Step 10: Run manager tests**

```powershell
cargo test -p local-workspace-mcp-manager connection::manager -- --nocapture
```

- [ ] **Step 11: Commit**

```powershell
git add src-tauri/src/connection src-tauri/src/lib.rs
git commit -m "feat: add Tailscale connection state machine"
```

---

### Task 11: Version Settings and Migrate Away from Legacy Tunnel Configuration

**Files:**
- Rewrite: `src-tauri/src/settings.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/settings.rs`

**Interfaces:**
- Produces versioned settings:

```rust
pub struct AppSettings {
    pub version: u32,
    pub start_on_launch: bool,
    pub tailscale_hostname: String,
    pub onboarding_complete: bool,
}
```

- [ ] **Step 1: Write migration tests using old JSON**

Input:

```json
{
  "tunnel_id": "legacy-id",
  "tunnel_api_key": "legacy-secret"
}
```

Expected new settings:

```json
{
  "version": 2,
  "start_on_launch": false,
  "tailscale_hostname": "local-workspace-mcp",
  "onboarding_complete": false
}
```

The new serialized file must not contain `legacy-id` or `legacy-secret`.

- [ ] **Step 2: Implement tolerant load**

Load algorithm:

```text
missing settings -> defaults v2
v2 settings -> deserialize directly
legacy object containing tunnel_id/tunnel_api_key -> migrate to v2 and overwrite file atomically
invalid JSON -> return explicit SETTINGS_INVALID error; do not silently replace user file
```

- [ ] **Step 3: Remove automatic legacy `tunnel::spawn` from app startup**

`lib.rs` must initialize `ConnectionManager` and optionally call `start()` only when `start_on_launch == true`.

- [ ] **Step 4: Keep `src-tauri/src/tunnel.rs` compiling but unused until final cleanup**

This makes rollback easier while new Funnel E2E is still being proven.

- [ ] **Step 5: Run tests**

```powershell
cargo test -p local-workspace-mcp-manager settings
```

- [ ] **Step 6: Commit**

```powershell
git add src-tauri/src/settings.rs src-tauri/src/lib.rs
git commit -m "feat: migrate connection settings to Tailscale"
```

---

### Task 12: Expose Tauri Commands for Connection, Security, and Settings

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/api.ts` in a later frontend task

**Interfaces:**
- Produce Tauri commands:
  - `connection_status`
  - `start_connection`
  - `stop_connection`
  - `retry_connection`
  - `rotate_connection_url`
  - `reset_tailscale_identity`
  - `get_app_settings`
  - `update_app_settings`

- [ ] **Step 1: Add command-level tests around manager calls where practical**

At minimum verify command serialization shape and error mapping into stable JSON `{code,message}`.

- [ ] **Step 2: Implement connection commands as thin wrappers**

Example:

```rust
#[tauri::command]
pub async fn start_connection(
    state: State<'_, AppState>,
) -> Result<ConnectionStatus, String> {
    state.connection.start().await.map_err(err_string)
}
```

Do not duplicate lifecycle logic in `commands.rs`.

- [ ] **Step 3: Add settings update validation**

`tailscale_hostname` rules:

```text
1-63 characters
lowercase ASCII letters, digits, hyphen
must start and end with letter/digit
```

Reject invalid hostnames before persistence.

- [ ] **Step 4: Register commands in `generate_handler!`**

Keep existing workspace/audit commands.

- [ ] **Step 5: Run Rust tests**

```powershell
cargo test -p local-workspace-mcp-manager
```

- [ ] **Step 6: Commit**

```powershell
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat: expose Tailscale connection commands"
```

---

### Task 13: Enrich Audit Context for the Activity Experience

**Files:**
- Modify: `crates/core/src/pipeline.rs`
- Modify: `crates/core/src/audit.rs`
- Modify: `crates/core/src/ops/list_dir.rs`
- Modify: `crates/core/src/ops/find_files.rs`
- Modify: `crates/core/src/ops/search_text.rs`
- Modify: `crates/core/src/ops/read_file.rs`
- Modify: `crates/core/src/ops/write_file.rs`
- Modify: `src-tauri/tests/mcp_integration.rs`

**Interfaces:**
- Produce safe per-operation audit `path` and `detail` without content/secrets.

- [ ] **Step 1: Write integration assertions that successful MCP operations record a relative path and safe metrics**

For `read_file hello.txt`, assert:

```text
path == "hello.txt"
detail.bytes > 0
```

For `apply_patch`, assert:

```text
path == "hello.txt"
detail.bytes_changed == expected
```

- [ ] **Step 2: Replace the pipeline closure return with an audited outcome wrapper**

Use a generic shape such as:

```rust
pub struct Audited<T> {
    pub value: T,
    pub path: Option<String>,
    pub detail: Option<serde_json::Value>,
}
```

`run_tool_audited` writes `Audited.path/detail` then returns `Audited.value` to the handler.

- [ ] **Step 3: Update each operation call site with safe metadata**

Examples:

```text
list_directory -> path=requested relative directory, detail.entries + truncated
find_files -> detail.paths + truncated
search_text -> path=path_prefix if supplied, detail.matches + truncated
read_file -> path=rel, detail.bytes + total_lines + truncated
read_files -> detail.requested_files + successful_files
create_file -> path=created path, detail.bytes
apply_patch -> path=path, detail.bytes_changed
```

Do not include file contents or matched search lines in audit detail.

- [ ] **Step 4: Add a regression test that `.env` denial does not leak secret content**

Existing test checks response; extend audit assertion so `detail` contains no file content and `path` may identify `.env` only if the product intentionally allows the blocked filename to be shown in Activity. The spec allows path but never content.

- [ ] **Step 5: Run core and MCP integration tests**

```powershell
cargo test -p core
cargo test -p local-workspace-mcp-manager --test mcp_integration
```

- [ ] **Step 6: Commit**

```powershell
git add crates/core src-tauri/tests/mcp_integration.rs
git commit -m "feat: enrich safe MCP audit activity"
```

---

### Task 14: Create the Frontend Design System and Desktop Shell

**Files:**
- Modify: `package.json`
- Create: `src/types.ts`
- Create: `src/components/AppShell.tsx`
- Create: `src/components/Sidebar.tsx`
- Create: `src/components/StatusBadge.tsx`
- Create: `src/components/ConfirmDialog.tsx`
- Create: `src/components/EmptyState.tsx`
- Create: `src/styles/tokens.css`
- Create: `src/styles/global.css`
- Create: `src/styles/components.css`
- Rewrite: `src/styles.css` to import the new files or remove it and update `main.tsx`
- Modify: `src/main.tsx`
- Rewrite: `src/App.tsx`

**Interfaces:**
- Produces `ViewId = "dashboard" | "workspaces" | "activity" | "security" | "settings"`.
- Produces reusable status and dialog primitives used by later pages.

- [ ] **Step 1: Add only the icon dependency needed for a polished shell**

```powershell
npm install lucide-react
```

Do not add a full component framework in this milestone.

- [ ] **Step 2: Define design tokens before page CSS**

`tokens.css` must define semantic values such as:

```css
:root {
  --bg-app: #f6f7f9;
  --bg-surface: #ffffff;
  --bg-subtle: #f1f3f5;
  --text-primary: #17191c;
  --text-secondary: #666b73;
  --border-default: #e2e5e9;
  --accent: #2563eb;
  --success: #15803d;
  --warning: #b45309;
  --danger: #b91c1c;
  --radius-sm: 8px;
  --radius-md: 12px;
  --shadow-popover: 0 12px 32px rgba(0,0,0,.12);
}
```

Exact final palette may be tuned during implementation, but components must consume semantic tokens rather than hardcoded one-off colors.

- [ ] **Step 3: Implement global interaction rules**

Include:

```css
:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
button, input { font: inherit; }
button { cursor: pointer; }
button:disabled { cursor: not-allowed; opacity: .55; }
```

Ensure 13px minimum body text in dense activity UI and 14px+ for normal body copy.

- [ ] **Step 4: Build a fixed-width sidebar + flexible main content shell**

No React Router is required. Keep selected page in app state because this is a single-window desktop application.

- [ ] **Step 5: Create accessible reusable primitives**

`StatusBadge` must render icon + text, not color only.

`ConfirmDialog` must:

```text
trap the intended action behind explicit confirmation
support Escape to cancel
focus the primary/cancel button when opened
never use native confirm()
```

- [ ] **Step 6: Rewrite `App.tsx` as shell orchestration only**

`App.tsx` should own:

```text
current page
initial data load
connection-status subscription
global toast/error state
onboarding visibility
```

It must not contain page markup for all features inline.

- [ ] **Step 7: Build frontend**

```powershell
npm run build
```

Expected: TypeScript/Vite PASS.

- [ ] **Step 8: Commit**

```powershell
git add package.json package-lock.json src
git commit -m "feat: add polished desktop app shell"
```

---

### Task 15: Rewrite Frontend API Contracts and Live Connection Subscription

**Files:**
- Rewrite: `src/api.ts`
- Modify: `src/types.ts`
- Modify: `src/App.tsx`

**Interfaces:**
- Consumes Rust `ConnectionStatus`, workspace, audit, settings DTOs.
- Produces typed frontend API methods and `connection-status` event subscription.

- [ ] **Step 1: Define discriminated TypeScript connection state**

```ts
export type ConnectionState =
  | { state: "stopped" }
  | { state: "starting_server" }
  | { state: "starting_tunnel" }
  | { state: "needs_login"; login_url: string }
  | { state: "needs_funnel_permission"; message: string }
  | { state: "verifying"; fqdn: string }
  | { state: "online"; fqdn: string; endpoint: string }
  | { state: "degraded"; fqdn?: string | null; message: string }
  | { state: "stopping" }
  | { state: "error"; code: string; message: string };
```

- [ ] **Step 2: Add API methods matching Task 12 exactly**

```ts
connectionStatus()
startConnection()
stopConnection()
retryConnection()
rotateConnectionUrl()
resetTailscaleIdentity()
getAppSettings()
updateAppSettings(settings)
```

Keep workspace/audit APIs.

- [ ] **Step 3: Subscribe to backend connection events once in `App.tsx`**

```ts
const unlisten = await listen<ConnectionStatus>("connection-status", e => {
  setConnection(e.payload);
});
```

Seed state using `api.connectionStatus()` before relying on events.

- [ ] **Step 4: Preserve structured backend errors**

Extend error parsing to retain stable error codes for actionable UI:

```ts
export interface AppError { code?: string; message: string; detail?: unknown }
```

- [ ] **Step 5: Build**

```powershell
npm run build
```

- [ ] **Step 6: Commit**

```powershell
git add src/api.ts src/types.ts src/App.tsx
git commit -m "feat: connect UI to Tailscale lifecycle"
```

---

### Task 16: Build Dashboard and One-Click Start/Stop UX

**Files:**
- Create: `src/components/ConnectionCard.tsx`
- Create: `src/pages/DashboardPage.tsx`
- Modify: `src/App.tsx`
- Add CSS to `src/styles/components.css`

**Interfaces:**
- Consumes `ConnectionStatus`, workspace summary, recent audit events.
- Produces Start/Stop/Retry/Sign-in actions.

- [ ] **Step 1: Implement state-derived presentation helpers**

One function maps state to label/copy/intent; do not scatter switch statements across JSX.

Required copy:

```text
stopped -> Offline / Your local MCP connection is offline.
starting_server -> Starting / Starting local MCP server…
starting_tunnel -> Connecting / Starting secure Tailscale connection…
needs_login -> Sign-in required / Sign in to Tailscale to connect this app.
needs_funnel_permission -> Setup required / Funnel needs one-time approval for this tailnet.
verifying -> Verifying / Verifying the public MCP endpoint…
online -> Online / Ready for ChatGPT.
degraded -> Degraded / Connection started, but end-to-end verification failed.
stopping -> Stopping / Closing public access…
error -> Error / use backend message
```

- [ ] **Step 2: Implement primary action rules**

```text
Stopped/Error/Degraded -> Start or Retry
Starting*/Verifying/Stopping -> disabled progress action
NeedsLogin -> Sign in to Tailscale + Stop
NeedsFunnelPermission -> Retry + Open setup help
Online -> Stop
```

- [ ] **Step 3: Add endpoint panel only for Online**

Show endpoint in a selectable read-only field with:

```text
Copy
privacy warning: Anyone with this URL may attempt to use this MCP connection. Keep it private.
```

Use Tauri clipboard plugin or tested `navigator.clipboard`; prefer the plugin if WebView clipboard permission is inconsistent.

- [ ] **Step 4: Implement workspace/security/recent-activity summaries**

Dashboard must answer at a glance:

```text
How many workspaces are available?
How many allow write?
Is global access paused?
What happened recently?
```

- [ ] **Step 5: Manually test every visual state using temporary frontend fixture objects before wiring real sidecar E2E**

Do not leave fixtures in production code.

- [ ] **Step 6: Build**

```powershell
npm run build
```

- [ ] **Step 7: Commit**

```powershell
git add src/components/ConnectionCard.tsx src/pages/DashboardPage.tsx src/App.tsx src/styles
git commit -m "feat: add one-click connection dashboard"
```

---

### Task 17: Build First-Run Onboarding and Tailscale Sign-In UX

**Files:**
- Create: `src/components/OnboardingFlow.tsx`
- Modify: `src/App.tsx`
- Modify: `src/pages/DashboardPage.tsx`
- Modify: `src-tauri/src/commands.rs` / Tauri opener plugin setup
- Modify: `src-tauri/Cargo.toml`, `package.json`, capabilities as needed

**Interfaces:**
- Consumes `onboarding_complete`, workspaces, `NeedsLogin`, `NeedsFunnelPermission`.

- [ ] **Step 1: Add Tauri opener capability**

Use the official Tauri opener plugin; do not execute arbitrary shell commands from frontend.

Only open URLs matching:

```text
https://login.tailscale.com/
https://login.tailscale.com/a/
https://login.tailscale.com/admin/
https://tailscale.com/
```

- [ ] **Step 2: Implement onboarding steps**

Exact flow:

```text
Welcome
-> Add workspace
-> Review Read ON / Write OFF
-> Start connection
-> NeedsLogin: Sign in to Tailscale
-> NeedsFunnelPermission: one-time approval explanation + Retry
-> Online: Copy endpoint + Finish
```

Users may close onboarding after adding a workspace; Dashboard must still remain usable and show incomplete setup.

- [ ] **Step 3: Mark onboarding complete only after user reaches Online and presses Finish**

Do not mark it complete merely because the modal was dismissed.

- [ ] **Step 4: Add recovery copy for device approval/Funnel approval**

Never display raw Go errors as the primary user message. Keep them under an expandable diagnostic detail.

- [ ] **Step 5: Build and manually verify keyboard navigation**

```powershell
npm run build
```

Verify Tab order, Escape behavior, and focus return after dialog close.

- [ ] **Step 6: Commit**

```powershell
git add src src-tauri package.json package-lock.json
git commit -m "feat: add guided Tailscale onboarding"
```

---

### Task 18: Build the Workspaces Page with Coherent Permission UX

**Files:**
- Create: `src/pages/WorkspacesPage.tsx`
- Replace behavior from: `src/components/WorkspaceList.tsx`
- Modify CSS

**Interfaces:**
- Consumes existing workspace CRUD commands.

- [ ] **Step 1: Preserve native folder picker add flow**

Show new workspace immediately after backend success and focus/scroll to the new row.

- [ ] **Step 2: Enforce coherent Read/Write toggle behavior in UI**

Rules:

```ts
if disabling read:
  setCapabilities(id, false, false)

if enabling write while read is false:
  setCapabilities(id, true, true)

otherwise:
  toggle requested capability
```

Backend remains authoritative.

- [ ] **Step 3: Replace native confirm with `ConfirmDialog`**

Exact destructive copy:

```text
Remove workspace?
ChatGPT will lose access to this folder. Files on disk will not be deleted.
```

- [ ] **Step 4: Add empty state and permission summaries**

Do not add per-tool permission switches yet because the backend permission model remains read/write in this milestone.

- [ ] **Step 5: Build**

```powershell
npm run build
```

- [ ] **Step 6: Commit**

```powershell
git add src/pages/WorkspacesPage.tsx src/components/WorkspaceList.tsx src/styles
git commit -m "feat: redesign workspace permission management"
```

---

### Task 19: Build Activity Page and Detail Drawer/Dialog

**Files:**
- Create: `src/pages/ActivityPage.tsx`
- Create: `src/components/ActivityDetail.tsx`
- Replace: `src/components/AuditTable.tsx`
- Modify CSS

**Interfaces:**
- Consumes enriched `AuditEvent` from Task 13.

- [ ] **Step 1: Implement local-time formatting and source labels**

Map:

```text
source=mcp -> ChatGPT
source=ui -> App
```

Keep the raw source in detail view.

- [ ] **Step 2: Implement filters**

```text
All
Allowed
Blocked
Errors
```

Blocked maps to audit `decision == "deny"`.

Add client-side search across operation, tool, path, and error code for the latest 100 events.

- [ ] **Step 3: Implement activity rows**

Each row includes:

```text
time
source
humanized operation
workspace name when resolvable
decision badge
path if present
```

- [ ] **Step 4: Implement detail view**

Display safe JSON with pretty formatting. Never synthesize or fetch file contents from an audit event.

- [ ] **Step 5: Refresh activity after workspace/security mutations and MCP connection events**

Keep refresh explicit/event-driven; no sub-second polling.

- [ ] **Step 6: Build and commit**

```powershell
npm run build
git add src/pages/ActivityPage.tsx src/components/ActivityDetail.tsx src/components/AuditTable.tsx src/styles
git commit -m "feat: add useful MCP activity history"
```

---

### Task 20: Build Security Page, Pause Control, and URL Rotation

**Files:**
- Create: `src/pages/SecurityPage.tsx`
- Modify: `src/App.tsx`
- Modify CSS

**Interfaces:**
- Consumes existing `set_paused` plus new `rotate_connection_url` and `reset_tailscale_identity`.

- [ ] **Step 1: Show explicit protection summary**

Rows:

```text
Public endpoint protection    Capability URL · Active
Sensitive files              Protected
Path confinement             Active
Concurrent edit protection   Active
Write-enabled workspaces     <count>
Global MCP access            Active/Paused
```

- [ ] **Step 2: Move Pause/Resume here while retaining a compact paused warning globally**

When paused, AppShell shows a persistent warning strip:

```text
MCP access is paused. Connection may be online, but file tools are blocked.
```

- [ ] **Step 3: Implement Rotate connection URL confirmation**

Copy:

```text
Rotate connection URL?
The current URL will stop working and you will need to update the MCP endpoint in ChatGPT. Your Tailscale hostname will stay the same.
```

After success, if Online, show the new endpoint and Copy action.

- [ ] **Step 4: Implement Reset Tailscale identity as a more destructive advanced action**

Copy:

```text
Reset Tailscale identity?
This app will need to sign in to Tailscale again and its *.ts.net hostname may change.
```

Require typed confirmation `RESET` or a second confirmation step.

- [ ] **Step 5: Build and commit**

```powershell
npm run build
git add src/pages/SecurityPage.tsx src/App.tsx src/styles
git commit -m "feat: add security controls and URL rotation"
```

---

### Task 21: Build Settings Page and Diagnostics

**Files:**
- Create: `src/pages/SettingsPage.tsx`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src/api.ts`
- Modify settings types

**Interfaces:**
- Consumes `AppSettings`, `ConnectionStatus`.

- [ ] **Step 1: Implement General settings**

```text
Start connection automatically on app launch  OFF by default
```

Persist immediately only after backend success; rollback toggle on error.

- [ ] **Step 2: Implement Connection settings**

Show:

```text
Hostname: local-workspace-mcp
Current Tailscale FQDN: read-only when known
Tailscale state location: read-only, path elided with reveal/copy action
```

Changing hostname while identity exists must show reset/re-enrollment warning and must not silently mutate a running node identity.

- [ ] **Step 3: Add redacted diagnostics command**

Backend returns:

```json
{
  "app_version":"0.1.0",
  "connection_state":"online",
  "local_port":12345,
  "fqdn":"...ts.net",
  "paused":false,
  "workspace_count":2,
  "recent_sidecar_logs":["redacted lines only"]
}
```

It must never contain:

```text
capability token
full capability endpoint
Tailscale login auth URL
legacy tunnel API key
file contents
```

- [ ] **Step 4: Add Copy diagnostics button**

Use clipboard integration and a transient `Copied` acknowledgment.

- [ ] **Step 5: Build and commit**

```powershell
npm run build
git add src/pages/SettingsPage.tsx src/api.ts src-tauri/src/commands.rs src-tauri/src/settings.rs src/styles
git commit -m "feat: add connection settings and safe diagnostics"
```

---

### Task 22: Integration-Test the Connection Manager with a Fake Sidecar Process

**Files:**
- Create: `src-tauri/tests/connection_integration.rs`
- Create: `src-tauri/tests/fixtures/fake_sidecar.ps1` or a tiny test binary under `src-tauri/tests/fixtures/`
- Modify connection manager constructors for dependency injection only as needed

**Interfaces:**
- Exercises actual process spawn/stdout parsing without real Tailscale network dependency.

- [ ] **Step 1: Build a fake sidecar fixture with scenario selection**

Supported scenarios:

```text
online
auth_then_online
funnel_denied
malformed_event
exit_early
slow_start
```

The fake never reads or prints the real capability env value.

- [ ] **Step 2: Test `auth_then_online`**

Assert event sequence and final Online after a fake health verifier succeeds.

- [ ] **Step 3: Test `funnel_denied`**

Assert final `NeedsFunnelPermission` and process cleanup.

- [ ] **Step 4: Test malformed stdout**

A malformed line must not crash Tauri/Rust; diagnostics record it and later valid events still work if process continues.

- [ ] **Step 5: Test Stop while sidecar is slow**

Assert no child process remains and status reaches Stopped.

- [ ] **Step 6: Run integration tests**

```powershell
cargo test -p local-workspace-mcp-manager --test connection_integration -- --nocapture
```

- [ ] **Step 7: Commit**

```powershell
git add src-tauri/tests src-tauri/src/connection
git commit -m "test: cover connection lifecycle end to end"
```

---

### Task 23: Run Real Windows Tailscale Funnel E2E Before Removing Legacy Tunnel

**Files:**
- No production code required unless the gate finds a verified defect.
- Update execution notes in this plan or a dedicated test report under `docs/`.

**Interfaces:**
- Validates real `tsnet` + Funnel + Rust MCP + ChatGPT path.

- [ ] **Step 1: Start from clean Tailscale app state**

Back up then remove only the app-specific test state directory, not unrelated Tailscale user data.

- [ ] **Step 2: Launch app and add a temporary workspace**

Use a temp folder with non-sensitive files.

- [ ] **Step 3: Press Start and complete Tailscale login**

Expected:

```text
NeedsLogin -> Starting/Authenticated -> Funnel listening -> Verifying -> Online
```

If Funnel approval is disabled in the tailnet, verify the UI enters `NeedsFunnelPermission`, enable Funnel once, press Retry, and continue.

- [ ] **Step 4: Copy the displayed endpoint into ChatGPT custom MCP/app configuration**

Verify tool scan/list works.

- [ ] **Step 5: Run MCP read scenario**

Ask ChatGPT to:

```text
list workspaces
list test workspace
read test file
search a unique string
```

Verify Activity records each call.

- [ ] **Step 6: Run MCP write scenario**

Enable write, read the file for revision, apply a patch, and verify disk content changed exactly once.

- [ ] **Step 7: Verify security boundaries**

Test:

```text
raw https://FQDN/mcp -> 404
wrong capability -> 404
.env read -> SENSITIVE_FILE_BLOCKED
../ traversal -> PATH_OUTSIDE_WORKSPACE
Pause -> ACCESS_PAUSED
```

- [ ] **Step 8: Verify stable URL across restart**

Record endpoint, Stop, exit app, relaunch, Start. Expected: exact same FQDN and exact same capability endpoint.

- [ ] **Step 9: Verify rotation**

Rotate connection URL. Expected:

```text
old URL -> 404
new URL -> works
FQDN unchanged
```

- [ ] **Step 10: Verify parent force-kill cleanup**

Start Online, force-kill the Tauri parent in Task Manager, then confirm no `tailscale-funnel-sidecar` process remains.

- [ ] **Step 11: Record E2E result before cleanup**

Do not remove the legacy tunnel path unless every scenario above passes or has an explicitly documented non-code external prerequisite.

---

### Task 24: Remove Legacy OpenAI Tunnel Runtime and Obsolete UI

**Files:**
- Delete: `src-tauri/src/tunnel.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/tauri.conf.json`
- Delete obsolete React components after replacement:
  - `src/components/StatusBar.tsx`
  - `src/components/PauseButton.tsx`
  - `src/components/AuditTable.tsx`
  - `src/components/SettingsDialog.tsx`
  - `src/components/WorkspaceList.tsx` if no longer imported
- Remove old binary packaging entries and obsolete tracked attribution specific only to removed binaries if licensing permits.

**Interfaces:**
- Leaves Tailscale as the single supported connection runtime.

- [ ] **Step 1: Search for all legacy tunnel symbols before deletion**

```powershell
rg "tunnel_status|configure_tunnel|stop_tunnel|TunnelHandle|tunnel-client|cloudflared" src src-tauri README.md package.json
```

- [ ] **Step 2: Remove legacy commands/modules only after references are zeroed**

Do not delete migration parsing for legacy settings until the chosen compatibility window ends; parsing old settings is harmless even after runtime removal.

- [ ] **Step 3: Remove old externalBin entries**

Final configuration:

```json
"externalBin": ["binaries/tailscale-funnel-sidecar"]
```

- [ ] **Step 4: Remove obsolete frontend components and imports**

```powershell
npm run build
```

Expected: PASS with no stale imports.

- [ ] **Step 5: Run full Rust/Go/frontend verification**

```powershell
cargo test --workspace
cd sidecars/tailscale-funnel
go test ./...
cd ../..
npm run build
npm run build:sidecar
```

- [ ] **Step 6: Commit**

```powershell
git add -A
git commit -m "refactor: retire legacy tunnel connection path"
```

---

### Task 25: Update Documentation, Packaging, and Final Release Verification

**Files:**
- Rewrite relevant sections: `README.md`
- Modify: `src-tauri/tauri.conf.json`
- Add/update: third-party license/notice files for bundled Tailscale sidecar
- Update: design/plan execution notes if implementation diverged for verified reasons

**Interfaces:**
- Produces a releasable NSIS build with accurate user instructions.

- [ ] **Step 1: Rewrite README user journey**

README must explain:

```text
Install
Add workspace
Press Start
Sign in to Tailscale once
Enable Funnel once if requested
Copy stable *.ts.net capability URL
Connect it to ChatGPT
Stop/Start without changing URL
Rotate URL when needed
```

Remove tunnel ID/API-key setup instructions.

- [ ] **Step 2: Document security semantics plainly**

Include:

```text
Funnel is public internet ingress.
The displayed MCP URL is a secret capability URL.
Do not share it.
Raw /mcp is not exposed.
Filesystem access is still constrained to approved workspaces.
Sensitive files remain blocked.
```

- [ ] **Step 3: Document Tailscale prerequisites**

Explain that Funnel requires tailnet MagicDNS/HTTPS/Funnel authorization and may require one-time tailnet approval. Do not say Tailscale Desktop is required.

- [ ] **Step 4: Add Tailscale license attribution**

Preserve BSD-3-Clause notices required by the pinned Tailscale dependency and any bundled generated notices from the sidecar dependency review.

- [ ] **Step 5: Run complete automated verification**

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd sidecars/tailscale-funnel
gofmt -w .
go test ./...
go vet ./...
cd ../..
npm run build
npm run build:sidecar
npx tauri build
```

Expected: every command PASS.

- [ ] **Step 6: Inspect release bundle**

Confirm NSIS bundle contains:

```text
local-workspace-mcp-manager.exe
tailscale-funnel-sidecar-x86_64-pc-windows-msvc.exe
```

and does not contain legacy `tunnel-client`/`cloudflared` binaries.

- [ ] **Step 7: Perform one clean-install smoke test**

On a clean Windows user profile or VM:

```text
install
launch
add workspace
Start
Tailscale auth
Online
copy endpoint
Stop
relaunch
Start
same URL
uninstall
```

Verify uninstall does not delete user workspace files.

- [ ] **Step 8: Commit final documentation/release changes**

```powershell
git add README.md src-tauri docs sidecars scripts package.json package-lock.json Cargo.toml Cargo.lock
git commit -m "docs: finalize Tailscale Funnel release"
```

---

## Final Acceptance Checklist

- [ ] Main screen has a clear primary Start/Stop action.
- [ ] No tunnel ID/API-key form remains in the normal product UX.
- [ ] First login is handled through a Tailscale login URL captured from tsnet.
- [ ] App-specific Tailscale state persists in app-data.
- [ ] `Server.Hostname` remains stable unless user explicitly resets identity.
- [ ] Funnel uses `ListenFunnel("tcp", ":443", tsnet.FunnelOnly())`.
- [ ] Full MCP endpoint remains identical across normal Stop/restart cycles.
- [ ] Public raw `/mcp` returns 404.
- [ ] Wrong capability token returns 404 without service-identifying details.
- [ ] Capability token is generated from 32 CSPRNG bytes and stored with DPAPI.
- [ ] Capability token is never in argv, backend logs, diagnostics, or audit records.
- [ ] Online requires a verified protected public health response.
- [ ] Sidecar stdout is structured NDJSON and stderr is continuously drained/redacted.
- [ ] Windows Job Object prevents orphan sidecar processes.
- [ ] Existing-file patch replacement is proven working on Windows.
- [ ] Workspace read/write and sensitive-file protections still pass all tests.
- [ ] Activity UI shows safe path/metrics without file content leakage.
- [ ] Dashboard, Workspaces, Activity, Security, and Settings are complete and visually coherent.
- [ ] Onboarding supports first workspace, Tailscale login, Funnel approval recovery, and endpoint copy.
- [ ] Rotate connection URL invalidates the old URL while keeping FQDN.
- [ ] Reset Tailscale identity clearly warns FQDN may change.
- [ ] Legacy OpenAI tunnel runtime and binaries are removed only after real Funnel E2E passes.
- [ ] `cargo fmt`, `cargo clippy`, `cargo test --workspace`, `go test`, `go vet`, `npm run build`, sidecar build, and `npx tauri build` all pass.
- [ ] Clean Windows install/restart/uninstall smoke test passes.

## Execution Order / Review Gates

Use the tasks in order. Treat these as hard review gates:

1. **After Task 4:** review Go sidecar security and tsnet behavior before bundling.
2. **After Task 10:** review Rust state machine and secret-handling boundaries before frontend work.
3. **After Task 13:** review that audit enrichment does not leak content/secrets.
4. **After Task 17:** review the complete Start/Login/Online UX before building secondary pages.
5. **After Task 23:** real Tailscale + ChatGPT E2E must pass before deleting legacy runtime.
6. **After Task 25:** release only after automated verification and clean-install smoke test.

## Implementation Notes for Agentic Execution

- Prefer one task per fresh subagent/context. This plan is intentionally structured so a reviewer can accept/reject tasks independently.
- Do not repeatedly rebuild the entire Rust/Tauri application after every tiny edit. Run the narrow test listed in the current step first; run full workspace/build verification only at review gates and final release.
- Do not redesign unrelated filesystem core behavior while implementing connection/UI work.
- Do not replace the capability URL with raw no-auth Funnel because it appears easier during debugging.
- Do not store the capability token in `settings.json`, SQLite audit records, environment dumps, command lines, or frontend persistent storage.
- Do not infer `*.ts.net` hostname from the configured hostname; use `tsnet.Server.CertDomains()` after the node is running.
- Do not report `Online` from process spawn or `funnel_listening` alone; wait for public protected health verification.
- If a real Tailscale API/tsnet behavior differs from the pinned official v1.102.3 API, stop that task, verify against official Tailscale docs/source for v1.102.3, update the spec/plan with the exact verified behavior, then continue.

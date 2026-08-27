//! Deterministic localhost web app fixture for the real-browser integration
//! tests (`tests/browser_runtime_integration.rs`).
//!
//! The fixture is a tiny axum server that serves one interactive page plus a
//! handful of API/download routes. Every byte it serves is static so DOM,
//! console, network, screenshot, upload and download assertions stay stable
//! across runs.
//!
//! Routes:
//! ```text
//! /             -> interactive page (input, select, buttons, dialog trigger,
//!                  rerender/stale-ref trigger, responsive marker, upload input,
//!                  download link, console output)
//! /api/ping     -> JSON 200; the page logs "api-ping-complete" when it resolves
//! /api/fail     -> JSON 500
//! /download.txt -> deterministic text attachment
//! /api/count    -> POST increments a server-side action counter; GET reads it.
//!                  The counter lets tests prove a side-effecting click was never
//!                  replayed after a simulated browser crash.
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{Html, IntoResponse, Json, Response},
    routing::get,
    Router,
};

/// Deterministic bytes served by `/download.txt`.
pub const DOWNLOAD_BODY: &[u8] = b"deterministic-download-body\n";

/// Shared fixture state. The action counter is bumped by `POST /api/count`
/// (fired from the page's Count button) so a test can verify, out-of-band of
/// the browser, exactly how many times a side-effecting click executed.
#[derive(Clone, Default)]
pub struct FixtureState {
    pub count: Arc<AtomicUsize>,
}

/// A running fixture server. The action counter is read out-of-band over HTTP
/// (`GET /api/count`) rather than through a shared handle, so tests can verify
/// side effects even while the browser is dead.
pub struct Fixture {
    pub port: u16,
}

impl Fixture {
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

const INDEX_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Browser Test App</title>
<style>
  body { font-family: sans-serif; margin: 16px; }
  #responsive-marker { padding: 8px; border: 1px solid #000; }
  @media (max-width: 479px) { #responsive-marker { background: #ffcccc; } }
  @media (min-width: 480px) and (max-width: 1023px) { #responsive-marker { background: #ccffcc; } }
  @media (min-width: 1024px) { #responsive-marker { background: #ccccff; } }
</style>
</head>
<body>
  <h1>Browser Test App</h1>
  <div id="responsive-marker" data-mode="unknown">responsive:unknown</div>
  <div id="viewport-info">viewport:0x0</div>

  <label for="name-input">Name</label>
  <input id="name-input" type="text">

  <label for="color-select">Color</label>
  <select id="color-select">
    <option value="red">Red</option>
    <option value="green">Green</option>
    <option value="blue">Blue</option>
  </select>

  <button id="submit-btn">Submit</button>
  <div id="result" aria-live="polite">result:none</div>

  <button id="dialog-btn">OpenDialog</button>
  <div id="dialog-result">dialog:none</div>

  <button id="count-btn">Count</button>
  <div id="count-value">count:0</div>

  <button id="rerender-btn">Rerender</button>
  <div id="rerender-target"><button id="inner-btn">Inner</button></div>

  <label for="upload-input">Upload</label>
  <input id="upload-input" type="file">
  <div id="upload-result">upload:none</div>

  <a id="download-link" href="/download.txt" download>DownloadFile</a>

<script>
(function () {
  // Known console output used by test assertions.
  console.warn("known-console-warning");
  console.error("known-console-error");

  function updateResponsive() {
    var w = window.innerWidth, h = window.innerHeight;
    var mode = w < 480 ? "mobile" : (w < 1024 ? "tablet" : "desktop");
    var marker = document.getElementById("responsive-marker");
    marker.setAttribute("data-mode", mode);
    marker.textContent = "responsive:" + mode;
    document.getElementById("viewport-info").textContent = "viewport:" + w + "x" + h;
  }
  updateResponsive();
  window.addEventListener("resize", updateResponsive);

  // Success and failure network requests recorded by the page.
  fetch("/api/ping").then(function (r) { return r.json(); }).then(function () {
    console.log("api-ping-complete");
  }).catch(function () {});
  fetch("/api/fail").catch(function () {});

  document.getElementById("submit-btn").addEventListener("click", function () {
    var name = document.getElementById("name-input").value;
    var color = document.getElementById("color-select").value;
    document.getElementById("result").textContent = "result:name=" + name + ",color=" + color;
  });

  document.getElementById("dialog-btn").addEventListener("click", function () {
    var ok = window.confirm("dialog-prompt");
    document.getElementById("dialog-result").textContent = "dialog:" + (ok ? "accepted" : "dismissed");
  });

  document.getElementById("count-btn").addEventListener("click", function () {
    fetch("/api/count", { method: "POST" }).then(function (r) { return r.json(); }).then(function (d) {
      document.getElementById("count-value").textContent = "count:" + d.count;
    });
  });

  // Rerender/stale-ref trigger: replaces the subtree so previously resolved
  // element references become stale.
  document.getElementById("rerender-btn").addEventListener("click", function () {
    document.getElementById("rerender-target").innerHTML =
      '<button id="inner-btn">InnerReplaced</button>';
  });

  document.getElementById("upload-input").addEventListener("change", function (e) {
    var f = e.target.files && e.target.files[0];
    document.getElementById("upload-result").textContent = "upload:" + (f ? f.name : "none");
  });
})();
</script>
</body>
</html>
"#;

/// Binds the fixture to an OS-assigned localhost port and spawns it on the
/// current tokio runtime. The server lives until the runtime is dropped.
pub async fn spawn() -> Fixture {
    let state = FixtureState::default();
    let app = Router::new()
        .route("/", get(index))
        .route("/api/ping", get(ping))
        .route("/api/fail", get(fail))
        .route("/download.txt", get(download))
        .route("/api/count", get(count_get).post(count_post))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture listener");
    let port = listener.local_addr().expect("fixture addr").port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("fixture serve");
    });
    Fixture { port }
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn ping() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true }))
}

async fn fail() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": "intentional-failure" })),
    )
        .into_response()
}

async fn download() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"download.txt\"",
            ),
        ],
        DOWNLOAD_BODY,
    )
        .into_response()
}

async fn count_post(State(state): State<FixtureState>) -> Json<serde_json::Value> {
    let n = state.count.fetch_add(1, Ordering::SeqCst) + 1;
    Json(serde_json::json!({ "count": n }))
}

async fn count_get(State(state): State<FixtureState>) -> Json<serde_json::Value> {
    let n = state.count.load(Ordering::SeqCst);
    Json(serde_json::json!({ "count": n }))
}

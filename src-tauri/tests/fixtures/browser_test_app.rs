//! Deterministic localhost web app fixture for the real-browser integration
//! tests (`tests/browser_runtime_integration.rs`).
//!
//! The fixture is a tiny axum server that serves one interactive page plus a
//! small set of supporting routes. Every byte it serves is static (apart from
//! the action counter) so tree/console/cookie/extract assertions stay stable
//! across runs. All elements carry fixed ids/classes so selector-based and
//! backendNodeId-based replay tests are deterministic.
//!
//! Routes:
//! ```text
//! /             -> interactive page (headings/links, text input, select,
//!                  checkbox, radios, buttons, hover target, keyboard target,
//!                  scrollable page, delayed element + delayed script state,
//!                  console output, JSON-LD/OpenGraph/Twitter/canonical
//!                  metadata, extract-friendly elements)
//! /plain        -> minimal second page (link navigation target)
//! /cookie       -> sets `fixture-cookie=chocolate` via Set-Cookie
//! /api/ping     -> JSON 200; the page logs "api-ping-complete" when it resolves
//! /api/count    -> POST increments a server-side action counter; GET reads it.
//!                  The counter lets tests prove a side-effecting click was never
//!                  replayed after a simulated server crash.
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::{
    extract::State,
    http::header,
    response::{Html, IntoResponse, Json},
    routing::get,
    Router,
};

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

const INDEX_HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Browser Test App</title>
<link rel="canonical" href="https://fixture.example/">
<meta property="og:title" content="Fixture OpenGraph Title">
<meta property="og:type" content="website">
<meta name="twitter:card" content="summary">
<meta name="twitter:title" content="Fixture Twitter Title">
<script type="application/ld+json">
{"@context":"https://schema.org","@type":"WebPage","name":"Fixture JsonLd Page"}
</script>
</head>
<body>
  <h1 id="main-heading">Browser Test App</h1>

  <nav id="link-section">
    <h2>Links</h2>
    <a id="plain-link" href="/plain">PlainPage</a>
    <a id="section-link" href="#forms-section">FormsAnchor</a>
  </nav>

  <section id="forms-section">
    <h2>Forms</h2>
    <form id="main-form">
      <label for="name-input">Name</label>
      <input id="name-input" type="text">

      <label for="color-select">Color</label>
      <select id="color-select">
        <option value="red">Red</option>
        <option value="green">Green</option>
        <option value="blue">Blue</option>
      </select>

      <label for="agree-checkbox">Agree</label>
      <input id="agree-checkbox" type="checkbox">

      <span>Size</span>
      <label for="size-small">Small</label>
      <input type="radio" name="size" id="size-small" value="small">
      <label for="size-large">Large</label>
      <input type="radio" name="size" id="size-large" value="large">

      <button id="submit-btn" type="button">Submit</button>
      <div id="result" aria-live="polite">result:none</div>

      <button id="count-btn" type="button">Count</button>
      <div id="count-value">count:0</div>
    </form>
  </section>

  <section id="interaction-section">
    <h2>Interaction</h2>
    <div id="hover-target">hover:idle</div>
    <label for="keyboard-target">Keyboard</label>
    <input id="keyboard-target" type="text">
    <div id="keyboard-log">keys:</div>
    <div id="extract-target" data-sku="SKU-42">ExtractItem</div>
  </section>

  <div id="scroll-spacer" style="height:3000px">scroll-spacer</div>
  <div id="scroll-bottom">scroll-bottom</div>

<script>
(function () {
  // Known console output used by test assertions.
  console.log("known-console-log");
  console.warn("known-console-warning");
  console.error("known-console-error");

  // Successful network request recorded by the page.
  fetch("/api/ping").then(function (r) { return r.json(); }).then(function () {
    console.log("api-ping-complete");
  }).catch(function () {});

  document.getElementById("hover-target").addEventListener("mouseover", function () {
    document.getElementById("hover-target").textContent = "hover:hovered";
  });
  document.getElementById("hover-target").addEventListener("mouseenter", function () {
    document.getElementById("hover-target").textContent = "hover:hovered";
  });

  document.getElementById("keyboard-target").addEventListener("keydown", function (e) {
    var log = document.getElementById("keyboard-log");
    log.textContent = log.textContent + e.key;
  });

  document.getElementById("submit-btn").addEventListener("click", function () {
    var name = document.getElementById("name-input").value;
    var color = document.getElementById("color-select").value;
    document.getElementById("result").textContent = "result:name=" + name + ",color=" + color;
  });

  document.getElementById("count-btn").addEventListener("click", function () {
    fetch("/api/count", { method: "POST" }).then(function (r) { return r.json(); }).then(function (d) {
      document.getElementById("count-value").textContent = "count:" + d.count;
    });
  });

  // Makes the scroll-bottom marker an interactive element so tests can
  // resolve its backendNodeId through interactiveElements.
  document.getElementById("scroll-bottom").addEventListener("click", function () {});

  // Dynamic delayed element for waitForSelector.
  setTimeout(function () {
    var el = document.createElement("div");
    el.id = "delayed-element";
    el.textContent = "delayed-ready";
    document.body.appendChild(el);
  }, 300);

  // Dynamic delayed script state for waitForScript.
  setTimeout(function () {
    window.__delayedState = "ready";
  }, 500);
})();
</script>
</body>
</html>
"##;

const PLAIN_HTML: &str = r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Plain Page</title></head>
<body>
  <h1 id="plain-heading">Plain Page</h1>
  <a id="home-link" href="/">Home</a>
</body>
</html>
"#;

const COOKIE_HTML: &str = r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Cookie Page</title></head>
<body><h1 id="cookie-heading">Cookie Page</h1></body>
</html>
"#;

/// Binds the fixture to an OS-assigned loopback port and spawns it on the
/// current tokio runtime. The server lives until the runtime is dropped.
///
/// The real-runtime suite reaches this loopback server from inside the WSL2
/// VM through the suite's own node TCP proxy (Windows Firewall blocks raw
/// inbound connections to the test binary but allows node.exe).
pub async fn spawn() -> Fixture {
    let state = FixtureState::default();
    let app = Router::new()
        .route("/", get(index))
        .route("/plain", get(plain))
        .route("/cookie", get(cookie))
        .route("/api/ping", get(ping))
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

async fn plain() -> Html<&'static str> {
    Html(PLAIN_HTML)
}

async fn cookie() -> impl IntoResponse {
    (
        [(header::SET_COOKIE, "fixture-cookie=chocolate; Path=/")],
        Html(COOKIE_HTML),
    )
}

async fn ping() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true }))
}

async fn count_post(State(state): State<FixtureState>) -> Json<serde_json::Value> {
    let n = state.count.fetch_add(1, Ordering::SeqCst) + 1;
    Json(serde_json::json!({ "count": n }))
}

async fn count_get(State(state): State<FixtureState>) -> Json<serde_json::Value> {
    let n = state.count.load(Ordering::SeqCst);
    Json(serde_json::json!({ "count": n }))
}

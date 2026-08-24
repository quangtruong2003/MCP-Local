package bridge

import (
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
)

const testToken = "correct-token"

func newTestTarget(t *testing.T, backend http.HandlerFunc) *url.URL {
	t.Helper()
	ts := httptest.NewServer(backend)
	t.Cleanup(ts.Close)
	u, err := url.Parse(ts.URL)
	if err != nil {
		t.Fatal(err)
	}
	return u
}

func TestRouteDenialMatrix(t *testing.T) {
	target := newTestTarget(t, func(w http.ResponseWriter, r *http.Request) {
		t.Errorf("unrouted request %q reached the upstream", r.URL.Path)
	})
	h := NewHandler(testToken, target)

	cases := []struct {
		path string
		want int
	}{
		{"/", http.StatusNotFound},
		{"/mcp", http.StatusNotFound},
		{"/wrong/mcp", http.StatusNotFound},
		{"/" + testToken, http.StatusNotFound},
		{"/" + testToken + "/other", http.StatusNotFound},
		{"/" + strings.ToUpper(testToken) + "/health", http.StatusNotFound},
		{"/" + testToken + "/health", http.StatusNoContent},
	}

	for _, c := range cases {
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, "https://funnel.example"+c.path, nil))
		if rec.Code != c.want {
			t.Errorf("path %q: got status %d, want %d", c.path, rec.Code, c.want)
			continue
		}
		body := strings.ToLower(rec.Body.String())
		for _, banned := range []string{"mcp", "tailscale", "capability", testToken} {
			if strings.Contains(body, banned) {
				t.Errorf("path %q: 404 body leaks %q: %s", c.path, banned, rec.Body.String())
			}
		}
	}
}

func TestHealthMarkerHeader(t *testing.T) {
	target := newTestTarget(t, func(w http.ResponseWriter, r *http.Request) {})
	h := NewHandler(testToken, target)

	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, "https://funnel.example/"+testToken+"/health", nil))
	if got := rec.Header().Get("X-MCP-Bridge-Health"); got != "ok" {
		t.Fatalf("health marker header: got %q, want %q", got, "ok")
	}
}

func TestReverseProxyRewritesPaths(t *testing.T) {
	var seenPath, seenQuery string
	target := newTestTarget(t, func(w http.ResponseWriter, r *http.Request) {
		b, _ := io.ReadAll(r.Body)
		seenPath = r.URL.Path
		seenQuery = r.URL.RawQuery
		w.Write(b)
	})
	h := NewHandler(testToken, target)

	cases := []struct{ inPath, wantPath string }{
		{"/" + testToken + "/mcp", "/mcp"},
		{"/" + testToken + "/mcp/foo", "/mcp/foo"},
	}
	for _, c := range cases {
		req := httptest.NewRequest(http.MethodPost, "https://funnel.example"+c.inPath+"?a=1", strings.NewReader("body"))
		h.ServeHTTP(httptest.NewRecorder(), req)
		if seenPath != c.wantPath || seenQuery != "a=1" {
			t.Errorf("%q: upstream saw %q?a=%s, want %q?a=1", c.inPath, seenPath, seenQuery, c.wantPath)
		}
	}
}

package main

import (
	"errors"
	"net"
	"net/http"
	"testing"
)

func TestExtractAuthURL(t *testing.T) {
	msg := "To start this tsnet server, restart with TS_AUTHKEY set, or go to: https://login.tailscale.com/a/1234567890"
	got := extractAuthURL(msg)
	if got != "https://login.tailscale.com/a/1234567890" {
		t.Fatalf("unexpected auth URL: %q", got)
	}
}

func TestExtractAuthURLRejectsOtherHosts(t *testing.T) {
	for _, msg := range []string{
		"visit https://evil.example/a/123 to login",
		"http://login.tailscale.com/a/123 must be https",
		"no url here",
	} {
		if got := extractAuthURL(msg); got != "" {
			t.Fatalf("extractAuthURL(%q) = %q, want empty", msg, got)
		}
	}
}

func TestFunnelErrorCodeMapping(t *testing.T) {
	cases := []struct{ err, want string }{
		{"funnel not allowed by nodeattr: funnel", "FUNNEL_NOT_ALLOWED"},
		{"Funnel requires nodeAttr", "FUNNEL_NOT_ALLOWED"},
		{"failed to obtain certificate for node", "TAILSCALE_HTTPS_NOT_READY"},
		{"https is not enabled on the tailnet", "TAILSCALE_HTTPS_NOT_READY"},
		{"dial timeout", "TAILSCALE_START_FAILED"},
	}
	for _, c := range cases {
		if got := funnelErrorCode(errString(c.err)); got != c.want {
			t.Errorf("funnelErrorCode(%q) = %q, want %q", c.err, got, c.want)
		}
	}
}

func TestStartHTTPServerReturnsOnlyAfterAcceptLoopIsReady(t *testing.T) {
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	server := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusNoContent)
	})}

	errCh := startHTTPServer(server, ln)

	resp, err := http.Get("http://" + ln.Addr().String())
	if err != nil {
		t.Fatalf("request immediately after startHTTPServer failed: %v", err)
	}
	_ = resp.Body.Close()
	if resp.StatusCode != http.StatusNoContent {
		t.Fatalf("status = %d, want %d", resp.StatusCode, http.StatusNoContent)
	}

	if err := server.Close(); err != nil {
		t.Fatal(err)
	}
	if err := <-errCh; err != nil && !errors.Is(err, http.ErrServerClosed) {
		t.Fatalf("Serve returned unexpected error: %v", err)
	}
}

type errString string

func (e errString) Error() string { return string(e) }

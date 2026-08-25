// Command tailscale-funnel-sidecar exposes the local loopback MCP server
// through a Tailscale Funnel HTTPS endpoint, gated by a capability token.
//
// It communicates with its Rust parent over stdout NDJSON (see internal/events)
// and reads the capability token only from MCP_CAPABILITY_TOKEN — never from
// argv, config files, or logs.
package main

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"net"
	"net/http"
	"net/url"
	"os"
	"regexp"
	"strings"
	"sync"
	"time"

	"mcp-gptweb/tailscale-funnel-sidecar/internal/bridge"
	"mcp-gptweb/tailscale-funnel-sidecar/internal/events"

	"tailscale.com/tsnet"
)

type config struct {
	target   *url.URL
	stateDir string
	hostname string
}

func parseArgs(argv []string, token string) (*config, error) {
	fs := flag.NewFlagSet("tailscale-funnel-sidecar", flag.ContinueOnError)
	target := fs.String("target", "", "http://127.0.0.1:<port> upstream URL")
	stateDir := fs.String("state-dir", "", "persistent tsnet state directory")
	hostname := fs.String("hostname", "", "Tailscale node DNS label")
	if err := fs.Parse(argv); err != nil {
		return nil, err
	}

	u, err := url.Parse(*target)
	if err != nil || u.Scheme != "http" || u.Hostname() != "127.0.0.1" {
		return nil, errors.New("target must be an http://127.0.0.1 loopback URL")
	}
	if *stateDir == "" {
		return nil, errors.New("--state-dir is required")
	}
	if *hostname == "" {
		return nil, errors.New("--hostname is required")
	}
	if !validHostname(*hostname) {
		return nil, errors.New("--hostname must be a DNS label (1-63 chars: lowercase letters, digits, hyphens)")
	}
	if token == "" {
		return nil, errors.New("MCP_CAPABILITY_TOKEN is required in the environment")
	}
	return &config{target: u, stateDir: *stateDir, hostname: *hostname}, nil
}

func validHostname(h string) bool {
	if len(h) < 1 || len(h) > 63 {
		return false
	}
	for i, c := range h {
		switch {
		case c >= 'a' && c <= 'z', c >= '0' && c <= '9':
		case c == '-' && i > 0 && i < len(h)-1:
		default:
			return false
		}
	}
	return true
}

// authURLRe matches only Tailscale's own login URLs — never a broader pattern,
// so a crafted log line cannot redirect the user to an attacker login page.
var authURLRe = regexp.MustCompile(`https://login\.tailscale\.com/a/[A-Za-z0-9]+`)

func extractAuthURL(msg string) string {
	return authURLRe.FindString(msg)
}

// funnelErrorCode maps a ListenFunnel failure to a stable error code the Rust
// parent can act on.
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

type acceptReadyListener struct {
	net.Listener
	once  sync.Once
	ready chan struct{}
}

func (l *acceptReadyListener) Accept() (net.Conn, error) {
	l.once.Do(func() { close(l.ready) })
	return l.Listener.Accept()
}

// startHTTPServer starts Serve in the background and returns only after the
// server has entered its first Accept call. That makes any subsequent
// funnel_listening event truthful: the HTTP bridge is already accepting
// connections instead of racing the Rust verifier.
func startHTTPServer(server *http.Server, ln net.Listener) <-chan error {
	ready := make(chan struct{})
	wrapped := &acceptReadyListener{Listener: ln, ready: ready}
	errCh := make(chan error, 1)
	go func() {
		errCh <- server.Serve(wrapped)
	}()
	<-ready
	return errCh
}

func main() {
	token := strings.TrimSpace(os.Getenv("MCP_CAPABILITY_TOKEN"))
	cfg, err := parseArgs(os.Args[1:], token)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
	out := events.NewWriter(os.Stdout)
	if err := run(cfg, token, out); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func run(cfg *config, token string, out *events.Writer) error {
	_ = out.Emit(events.Event{Type: "starting"})

	if err := os.MkdirAll(cfg.stateDir, 0o700); err != nil {
		return fmt.Errorf("creating state dir: %w", err)
	}

	srv := &tsnet.Server{
		Dir:       cfg.stateDir,
		Hostname:  cfg.hostname,
		Ephemeral: false,
		UserLogf: func(format string, args ...any) {
			msg := fmt.Sprintf(format, args...)
			if authURL := extractAuthURL(msg); authURL != "" {
				_ = out.Emit(events.Event{Type: "auth_required", URL: authURL})
			}
			fmt.Fprintln(os.Stderr, redact(token, msg))
		},
		Logf: func(format string, args ...any) {}, // silence verbose tsnet logs
	}
	defer srv.Close()

	ln, err := srv.ListenFunnel("tcp", ":443", tsnet.FunnelOnly())
	if err != nil {
		code := funnelErrorCode(err)
		_ = out.Emit(events.Event{Type: "error", Code: code, Message: redact(token, err.Error())})
		return fmt.Errorf("ListenFunnel: %w", err)
	}
	defer ln.Close()

	domains := srv.CertDomains()
	if len(domains) == 0 {
		const code = "CERT_DOMAIN_UNAVAILABLE"
		_ = out.Emit(events.Event{Type: "error", Code: code, Message: "no certificate domain available"})
		return errors.New(code)
	}

	handler := bridge.NewHandler(token, cfg.target)
	server := &http.Server{
		Handler:           handler,
		ReadHeaderTimeout: 15 * time.Second,
	}
	defer func() {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_ = server.Shutdown(ctx)
	}()

	serveErr := startHTTPServer(server, ln)
	_ = out.Emit(events.Event{Type: "authenticated"})
	_ = out.Emit(events.Event{Type: "funnel_listening", FQDN: domains[0]})

	err = <-serveErr
	if err != nil && !errors.Is(err, http.ErrServerClosed) {
		_ = out.Emit(events.Event{Type: "error", Code: "FUNNEL_SERVE_FAILED", Message: redact(token, err.Error())})
		return fmt.Errorf("serve: %w", err)
	}
	_ = out.Emit(events.Event{Type: "stopped"})
	return nil
}

// redact removes the capability token from any message before it reaches
// stderr or an event payload.
func redact(token, msg string) string {
	if token == "" {
		return msg
	}
	return strings.ReplaceAll(msg, token, "[REDACTED]")
}

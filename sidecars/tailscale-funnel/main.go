// Command tailscale-funnel-sidecar exposes the local loopback MCP server
// through a Tailscale Funnel HTTPS endpoint, gated by a capability token.
//
// It communicates with its Rust parent over stdout NDJSON (see internal/events)
// and reads the capability token only from MCP_CAPABILITY_TOKEN — never from
// argv, config files, or logs.
package main

import (
	"errors"
	"flag"
	"fmt"
	"net/url"
	"os"
	"strings"
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

func main() {
	token := strings.TrimSpace(os.Getenv("MCP_CAPABILITY_TOKEN"))
	cfg, err := parseArgs(os.Args[1:], token)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
	if err := run(cfg, token); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

// run is wired up by the tsnet lifecycle in Task 4; it fails fast until then.
func run(cfg *config, token string) error {
	return errors.New("sidecar lifecycle not implemented yet")
}

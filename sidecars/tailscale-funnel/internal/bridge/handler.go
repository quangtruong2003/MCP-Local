// Package bridge exposes exactly two capability-token-protected routes on the
// public Funnel listener: /TOKEN/health and /TOKEN/mcp... Everything else is a
// bare 404 whose body reveals nothing about the bridge.
package bridge

import (
	"net/http"
	"net/http/httputil"
	"net/url"
	"strings"
)

// NewHandler returns an http.Handler that serves the token-scoped routes and
// reverse-proxies MCP traffic to the loopback target.
func NewHandler(token string, target *url.URL) http.Handler {
	healthPath := "/" + token + "/health"
	mcpPrefix := "/" + token + "/mcp"

	proxy := httputil.NewSingleHostReverseProxy(target)
	proxy.FlushInterval = -1 // stream responses immediately (SSE-friendly)
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

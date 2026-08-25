package agentgatewaybackend

import (
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"strconv"
	"strings"
	"sync"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/util/wait"
	"sigs.k8s.io/yaml"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/openapischema"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
)

// openAPISchemaCache holds fetched openapi.schema.url content, off the krt
// reconciliation queue (see package openapischema for why). It is lazily
// initialized on first use so this package works standalone in tests that
// never call NewBackendPlugin; NewBackendPlugin re-initializes it with the
// controller's real stop channel so background fetches actually stop on
// shutdown.
var (
	openAPISchemaCacheMu sync.Mutex
	openAPISchemaCache   *openapischema.Cache
)

func setOpenAPISchemaCacheStop(stop <-chan struct{}) {
	openAPISchemaCacheMu.Lock()
	defer openAPISchemaCacheMu.Unlock()
	openAPISchemaCache = openapischema.NewCache(stop)
}

func getOpenAPISchemaCache() *openapischema.Cache {
	openAPISchemaCacheMu.Lock()
	defer openAPISchemaCacheMu.Unlock()
	if openAPISchemaCache == nil {
		openAPISchemaCache = openapischema.NewCache(wait.NeverStop)
	}
	return openAPISchemaCache
}

// openAPISchemaSource is the shape of McpOpenAPITarget.Schema when it isn't a
// literal inline string: an object naming exactly one remote source.
type openAPISchemaSource struct {
	URL          *string                      `json:"url,omitempty"`
	ConfigMapRef *corev1.ConfigMapKeySelector `json:"configMapRef,omitempty"`
}

// resolveOpenAPISchema returns the resolved OpenAPI schema text (JSON or
// YAML) for an MCP OpenAPI target. Schema is either a literal inline string,
// or an object with a `url` (fetched in the background, off the krt
// reconciliation queue — see openAPISchemaCache) or `configMapRef` (a
// synchronous, network-free krt lookup) source.
//
// Whichever source it came from, the resolved text is validated as a
// structurally-plausible OpenAPI document before being accepted: this is
// the same content that will eventually reach the Rust data plane's strict
// openapiv3 parser over xDS, and a target that fails there — rather than
// here — takes the whole AgentgatewayBackend's MCP backend down with it
// (every target in it, not just the broken one) while this CRD's own
// status still says Accepted. Catching an obviously-malformed schema here
// instead means the status honestly reflects a translation error.
func resolveOpenAPISchema(ctx plugins.PolicyCtx, namespace string, t *agentgateway.McpOpenAPITarget) (string, error) {
	schema, err := resolveOpenAPISchemaSource(ctx, namespace, t)
	if err != nil {
		return "", err
	}
	if err := validateOpenAPISchema(schema); err != nil {
		return "", fmt.Errorf("openapi schema: %w", err)
	}
	return schema, nil
}

func resolveOpenAPISchemaSource(ctx plugins.PolicyCtx, namespace string, t *agentgateway.McpOpenAPITarget) (string, error) {
	var inline string
	if err := json.Unmarshal(t.Schema.Raw, &inline); err == nil {
		return inline, nil
	}

	var source openAPISchemaSource
	if err := json.Unmarshal(t.Schema.Raw, &source); err != nil {
		return "", fmt.Errorf("openapi schema must be a literal string or an object with url or configMapRef: %w", err)
	}

	switch {
	case source.URL != nil && source.ConfigMapRef != nil:
		return "", fmt.Errorf("openapi schema must set exactly one of url or configMapRef, not both")
	case source.URL != nil:
		schema, ready, err := getOpenAPISchemaCache().Get(ctx.Krt, *source.URL)
		if err != nil {
			return "", err
		}
		if !ready {
			return "", fmt.Errorf("openapi schema.url %s has not been fetched yet; will retry automatically", *source.URL)
		}
		return schema, nil
	case source.ConfigMapRef != nil:
		return resolveOpenAPISchemaConfigMap(ctx, namespace, source.ConfigMapRef)
	default:
		return "", fmt.Errorf("openapi schema must be a literal string or an object with url or configMapRef")
	}
}

// minimalOpenAPIDoc mirrors only the REQUIRED top-level fields of the Rust
// data plane's openapiv3::OpenAPI struct (crates.io openapiv3 v2.2.0,
// src/openapi.rs): `openapi`, `info.title`, `info.version`, and `paths` all
// have no #[serde(default)], so deserialization there fails if any is
// missing. This is intentionally not a full OpenAPI validator — it exists
// only to catch the common, obvious mistakes (a URL pasted as a literal
// schema string, truncated JSON, a missing required field) before they
// reach xDS, not to reject every schema the Rust parser might.
type minimalOpenAPIDoc struct {
	OpenAPI string         `json:"openapi"`
	Info    *minimalOAInfo `json:"info"`
	Paths   map[string]any `json:"paths"`
}

type minimalOAInfo struct {
	Title   string `json:"title"`
	Version string `json:"version"`
}

func validateOpenAPISchema(schema string) error {
	var doc minimalOpenAPIDoc
	if err := yaml.Unmarshal([]byte(schema), &doc); err != nil {
		return fmt.Errorf("not valid JSON or YAML: %w", err)
	}
	if doc.OpenAPI == "" {
		return errors.New(`missing required top-level "openapi" version field`)
	}
	if doc.Info == nil || doc.Info.Title == "" || doc.Info.Version == "" {
		return errors.New(`missing required "info.title" and/or "info.version" fields`)
	}
	if doc.Paths == nil {
		return errors.New(`missing required top-level "paths" field`)
	}
	return nil
}

func resolveOpenAPISchemaConfigMap(ctx plugins.PolicyCtx, namespace string, ref *corev1.ConfigMapKeySelector) (string, error) {
	if ref.Name == "" {
		return "", fmt.Errorf("openapi schema.configMapRef name is required")
	}
	if ref.Key == "" {
		return "", fmt.Errorf("openapi schema.configMapRef key is required")
	}

	key := namespace + "/" + ref.Name
	cm := ptr.Flatten(krt.FetchOne(ctx.Krt, ctx.Collections.ConfigMaps, krt.FilterKey(key)))
	if cm == nil {
		return "", fmt.Errorf("openapi schema.configMapRef configmap %s not found", key)
	}

	schema, ok := cm.Data[ref.Key]
	if !ok {
		return "", fmt.Errorf("openapi schema.configMapRef configmap %s missing key %q", key, ref.Key)
	}
	return schema, nil
}

// splitOpenAPIHost splits McpOpenAPITarget.Host into a hostname and numeric
// port. Two forms are accepted:
//   - "host:port" (e.g. "localhost:8080") — the common case.
//   - a bare hostname or IP with no port (e.g. "localhost"), which defaults
//     to port 80 — matching the standalone config path's McpBackendHost
//     when only a host is given (crates/agentgateway/src/types/local.rs).
//
// A scheme prefix (e.g. "https://localhost:8080") is deliberately rejected
// rather than parsed: standalone mode infers TLS from the scheme, but here
// TLS is a separate, explicit policy (`policies.tls`) — silently accepting
// "https://" without actually enabling TLS would connect over plain TCP to
// what's likely a TLS-only endpoint, failing in a confusing way. Rejecting
// it outright and pointing at the real mechanism is safer than guessing.
func splitOpenAPIHost(hostPort string) (string, int32, error) {
	// Checked before net.SplitHostPort deliberately: for a value like
	// "http://localhost" (no port), SplitHostPort doesn't error — it splits
	// on the colon after "http" and reports port="//localhost", which would
	// otherwise surface a confusing "invalid port" error instead of this
	// clear one.
	if strings.Contains(hostPort, "://") {
		return "", 0, fmt.Errorf(
			`openapi host %q must not include a scheme; use "host:port" and set policies.tls to connect over TLS`,
			hostPort,
		)
	}

	if host, portStr, err := net.SplitHostPort(hostPort); err == nil {
		port, perr := parseOpenAPIPort(portStr)
		if perr != nil {
			return "", 0, fmt.Errorf("openapi host %q has an invalid port: %w", hostPort, perr)
		}
		return host, port, nil
	}

	if hostPort != "" && !strings.ContainsAny(hostPort, ":/") {
		return hostPort, 80, nil
	}

	return "", 0, fmt.Errorf(`openapi host %q must be in "host:port" form, or a bare hostname (defaults to port 80)`, hostPort)
}

func parseOpenAPIPort(s string) (int32, error) {
	port, err := strconv.ParseUint(s, 10, 16)
	if err != nil || port == 0 {
		return 0, fmt.Errorf("invalid port %q", s)
	}
	return int32(port), nil //nolint:gosec // G115: bounded by ParseUint(_, 16) above
}

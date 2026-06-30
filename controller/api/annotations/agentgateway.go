package annotations

import (
	"fmt"
	"strconv"
	"strings"

	"k8s.io/apimachinery/pkg/util/sets"
)

// LegacyMCPServiceHTTPPath is the legacy annotation used to specify the HTTP path for the MCP service. Users should switch to MCPServiceHTTPPath.
const LegacyMCPServiceHTTPPath = "kgateway.dev/mcp-path"

// MCPServiceHTTPPath is the annotation used to specify the HTTP path for the MCP service
const MCPServiceHTTPPath = "agentgateway.dev/mcp-path"

// MCPServiceTargetName is the annotation used to specify the target name for the MCP service.
// The value must be a valid Gateway API SectionName.
const MCPServiceTargetName = "agentgateway.dev/mcp-target-name"

// InternalPorts is a comma-separated list of ports whose bind should be internal
// (routing-only: no OS listener socket, no Service port, no container port). It may
// be set on a Gateway or a ListenerSet, and may only reference ports defined by that
// same object's listeners.
const InternalPorts = "agentgateway.dev/internal-ports"

// ParseInternalPorts parses the InternalPorts annotation value into the set of ports
// marked internal. isListenerPort reports whether a port is defined by the annotated
// object's own listeners; any referenced port that is malformed, out of range, or not
// an own listener port is returned as an error message so the caller can surface an
// Accepted=False condition. On any error the returned set is empty (the annotation is
// rejected wholesale rather than partially applied). The membership check is passed as
// a closure so callers can use whichever set type they already have.
func ParseInternalPorts(value string, isListenerPort func(port int32) bool) (sets.Set[int32], []string) {
	internal := sets.New[int32]()
	var errs []string
	for raw := range strings.SplitSeq(value, ",") {
		p := strings.TrimSpace(raw)
		if p == "" {
			continue
		}
		n, err := strconv.ParseInt(p, 10, 32)
		if err != nil || n < 1 || n > 65535 {
			errs = append(errs, fmt.Sprintf("invalid port %q: must be an integer between 1 and 65535", p))
			continue
		}
		port := int32(n)
		if !isListenerPort(port) {
			errs = append(errs, fmt.Sprintf("port %d is not defined by any listener on this resource", port))
			continue
		}
		internal.Insert(port)
	}
	if len(errs) > 0 {
		return sets.New[int32](), errs
	}
	return internal, errs
}

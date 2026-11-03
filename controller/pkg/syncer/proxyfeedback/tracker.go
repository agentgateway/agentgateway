// Package proxyfeedback carries configuration problems that proxies report over xDS back to the
// status of the Kubernetes objects they were translated from.
//
// Each controller replica tracks what its own proxy connections report and publishes the active
// set to a ConfigMap. The leader reads every replica's ConfigMap and adds the problems to policy
// status, because a proxy may be connected to a replica that does not write status.
package proxyfeedback

import (
	"cmp"
	"fmt"
	"slices"
	"strings"
	"sync"
	"unicode/utf8"

	discovery "github.com/envoyproxy/go-control-plane/envoy/service/discovery/v3"
	"google.golang.org/protobuf/types/known/anypb"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/api"
)

// Severity is how a proxy handled a resource it reported.
type Severity string

const (
	// SeverityWarning means the proxy accepted the resource but reported a problem with it.
	SeverityWarning Severity = "Warning"
	// SeverityError means the proxy rejected the resource.
	SeverityError Severity = "Error"
)

// maxMessageLength bounds each reported message so a replica's ConfigMap stays small.
const maxMessageLength = 1024

// maxPendingResponses bounds the responses kept per type while waiting for the proxy to reply.
// Proxies reply in order, so a reply to a newer response means older ones will not get one.
const maxPendingResponses = 8

// Source identifies the Kubernetes object an xDS resource was translated from.
type Source struct {
	Kind      string `json:"kind"`
	Namespace string `json:"namespace"`
	Name      string `json:"name"`
}

func (s Source) String() string {
	return s.Kind + "/" + s.Namespace + "/" + s.Name
}

// Diagnostic is one problem a proxy reported for one xDS resource.
type Diagnostic struct {
	Gateway  types.NamespacedName `json:"gateway"`
	Source   Source               `json:"source"`
	Resource string               `json:"resource"`
	Severity Severity             `json:"severity"`
	Message  string               `json:"message"`
}

func compareDiagnostics(a, b Diagnostic) int {
	return cmp.Or(
		strings.Compare(a.Source.String(), b.Source.String()),
		strings.Compare(a.Gateway.String(), b.Gateway.String()),
		strings.Compare(a.Resource, b.Resource),
		strings.Compare(string(a.Severity), string(b.Severity)),
		strings.Compare(a.Message, b.Message),
	)
}

// Report is one entry of the feedback a proxy sent for a response.
type Report struct {
	Resource string
	Severity Severity
	Message  string
}

// Tracker holds the diagnostics reported by the proxies connected to this replica.
type Tracker struct {
	mu sync.Mutex
	// reporters counts the connections currently reporting each diagnostic, so several proxy
	// replicas of one Gateway report a problem once.
	reporters map[Diagnostic]int
	changed   chan struct{}
}

func NewTracker() *Tracker {
	return &Tracker{
		reporters: map[Diagnostic]int{},
		changed:   make(chan struct{}, 1),
	}
}

// Changed is signalled when the set returned by Snapshot changes.
func (t *Tracker) Changed() <-chan struct{} {
	return t.changed
}

// Snapshot returns the active diagnostics in a stable order.
func (t *Tracker) Snapshot() []Diagnostic {
	t.mu.Lock()
	defer t.mu.Unlock()
	out := make([]Diagnostic, 0, len(t.reporters))
	for d := range t.reporters {
		out = append(out, d)
	}
	slices.SortFunc(out, compareDiagnostics)
	return out
}

func (t *Tracker) update(add, remove []Diagnostic) {
	if len(add) == 0 && len(remove) == 0 {
		return
	}
	t.mu.Lock()
	changed := false
	for _, d := range remove {
		t.reporters[d]--
		if t.reporters[d] <= 0 {
			delete(t.reporters, d)
			changed = true
		}
	}
	for _, d := range add {
		if t.reporters[d] == 0 {
			changed = true
		}
		t.reporters[d]++
	}
	t.mu.Unlock()
	if changed {
		select {
		case t.changed <- struct{}{}:
		default:
		}
	}
}

// NewConnection starts tracking one xDS connection from a proxy of the given Gateway. lookup
// returns the resource currently sent under a name, so a report can be traced to its source.
func (t *Tracker) NewConnection(gateway types.NamespacedName, lookup func(typeURL, name string) *anypb.Any) *Connection {
	return &Connection{
		tracker: t,
		gateway: gateway,
		lookup:  lookup,
		pending: map[string][]sentResponse{},
		active:  map[string]map[string][]Diagnostic{},
	}
}

// Connection tracks the diagnostics one proxy connection reports. A proxy only reports problems
// for the resources in the response it is replying to, so a resource stays reported until a
// later response carries it again without a report, removes it, or the connection closes.
type Connection struct {
	tracker *Tracker
	gateway types.NamespacedName
	lookup  func(typeURL, name string) *anypb.Any

	mu      sync.Mutex
	closed  bool
	pending map[string][]sentResponse
	// active holds the reported diagnostics by type and resource name.
	active map[string]map[string][]Diagnostic
}

type sentResponse struct {
	nonce     string
	resources []*discovery.Resource
	removed   []string
}

// Sent records a response so the proxy's reply to its nonce can be matched to its resources.
func (c *Connection) Sent(typeURL, nonce string, resources []*discovery.Resource, removed []string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closed {
		return
	}
	pending := append(c.pending[typeURL], sentResponse{nonce: nonce, resources: resources, removed: removed})
	if len(pending) > maxPendingResponses {
		pending = pending[len(pending)-maxPendingResponses:]
	}
	c.pending[typeURL] = pending
}

// Replied applies the proxy's reply to the response with the given nonce. Resources the
// response carried or removed without a report are no longer reported.
func (c *Connection) Replied(typeURL, nonce string, reports []Report) {
	c.mu.Lock()
	pending := c.pending[typeURL]
	i := slices.IndexFunc(pending, func(r sentResponse) bool { return r.nonce == nonce })
	if c.closed || i < 0 {
		c.mu.Unlock()
		return
	}
	sent := pending[i]
	c.pending[typeURL] = pending[i+1:]
	active := c.active[typeURL]
	if len(active) == 0 && len(reports) == 0 {
		c.mu.Unlock()
		return
	}
	if active == nil {
		active = map[string][]Diagnostic{}
		c.active[typeURL] = active
	}

	reported := map[string][]Report{}
	for _, r := range reports {
		reported[r.Resource] = append(reported[r.Resource], r)
	}
	var add, remove []Diagnostic
	for _, resource := range sent.resources {
		previous := active[resource.Name]
		current := c.diagnostics(typeURL, resource.Name, reported[resource.Name])
		if slices.Equal(previous, current) {
			continue
		}
		remove = append(remove, previous...)
		add = append(add, current...)
		if len(current) == 0 {
			delete(active, resource.Name)
		} else {
			active[resource.Name] = current
		}
	}
	for _, name := range sent.removed {
		remove = append(remove, active[name]...)
		delete(active, name)
	}
	c.mu.Unlock()
	c.tracker.update(add, remove)
}

// Close stops reporting everything this connection reported.
func (c *Connection) Close() {
	c.mu.Lock()
	c.closed = true
	var remove []Diagnostic
	for _, byName := range c.active {
		for _, diagnostics := range byName {
			remove = append(remove, diagnostics...)
		}
	}
	c.active = nil
	c.pending = nil
	c.mu.Unlock()
	c.tracker.update(nil, remove)
}

func (c *Connection) diagnostics(typeURL, name string, reports []Report) []Diagnostic {
	if len(reports) == 0 {
		return nil
	}
	source := c.source(typeURL, name)
	out := make([]Diagnostic, 0, len(reports))
	for _, r := range reports {
		out = append(out, Diagnostic{
			Gateway:  c.gateway,
			Source:   source,
			Resource: name,
			Severity: r.Severity,
			Message:  truncate(r.Message, maxMessageLength),
		})
	}
	slices.SortFunc(out, compareDiagnostics)
	return slices.Compact(out)
}

// source returns the object a policy resource was translated from. Other resources have no
// source yet, so their diagnostics are tracked but not added to any status.
func (c *Connection) source(typeURL, name string) Source {
	raw := c.lookup(typeURL, name)
	if raw == nil {
		return Source{}
	}
	var resource api.Resource
	if err := raw.UnmarshalTo(&resource); err != nil {
		return Source{}
	}
	owner := resource.GetPolicy().GetName()
	if owner == nil {
		return Source{}
	}
	return Source{Kind: owner.GetKind(), Namespace: owner.GetNamespace(), Name: owner.GetName()}
}

func truncate(s string, limit int) string {
	if len(s) <= limit {
		return s
	}
	cut := limit
	for cut > 0 && !utf8.RuneStart(s[cut]) {
		cut--
	}
	return fmt.Sprintf("%s... (truncated)", s[:cut])
}

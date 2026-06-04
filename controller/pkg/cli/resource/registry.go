package resource

import (
	"fmt"
	"slices"
	"strings"

	"k8s.io/apimachinery/pkg/api/meta"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"sigs.k8s.io/controller-runtime/pkg/client"
)

// Column describes one column in short or wide table output.
type Column struct {
	Header string
	Wide   bool // if true, only shown with -o wide
	// Field returns the string value for the column from the given object.
	Field func(obj client.Object) string
}

// Section is a named block of text shown in describe output.
type Section struct {
	Title string
	Body  string
}

// ResourceHandler is implemented once per resource type.
// AgentGateway handlers are registered via init(); out-of-tree handlers are registered
// from out-of-tree binary startup before Execute() is called.
type ResourceHandler interface {
	// Mapping returns the GroupVersionResource and Kind for this resource type.
	Mapping() meta.RESTMapping
	// Aliases returns all names that identify this resource (plural, singular, short names).
	Aliases() []string
	// Columns returns the columns for table output. Wide-only columns are included in the slice
	// but have Wide=true set; callers filter based on the selected output format.
	Columns() []Column
	// DescribeExtra returns additional sections shown in `agctl describe` output
	// that are specific to this resource type (e.g. attached policies, resolved refs).
	DescribeExtra(obj client.Object) ([]Section, error)
}

var (
	// handlers is the registry of ResourceHandler implementations keyed by canonical plural name.
	handlers = map[string]ResourceHandler{}
	// aliasMap maps every alias (plural, singular, short name) to the canonical plural name.
	aliasMap = map[string]string{}
)

// Register adds a handler to the global registry.
// It is safe to call from init() in each resource-specific file
// or from out-of-tree code at startup before Execute().
func Register(h ResourceHandler) {
	canonical := canonicalName(h.Mapping().Resource.Resource)
	if _, exists := handlers[canonical]; exists {
		panic(fmt.Sprintf("resource handler already registered for %q", canonical))
	}
	handlers[canonical] = h
	for _, alias := range h.Aliases() {
		lower := strings.ToLower(alias)
		if existing, conflict := aliasMap[lower]; conflict && existing != canonical {
			panic(fmt.Sprintf("alias %q already registered for %q, cannot register for %q", lower, existing, canonical))
		}
		aliasMap[lower] = canonical
	}
}

// Lookup finds a handler by any alias (case-insensitive).
func Lookup(name string) (ResourceHandler, error) {
	canonical, ok := aliasMap[strings.ToLower(name)]
	if !ok {
		return nil, fmt.Errorf("unknown resource type %q; run `agctl get --help` to see available resources", name)
	}
	h, ok := handlers[canonical]
	if !ok {
		return nil, fmt.Errorf("internal error: handler for %q not found", canonical)
	}
	return h, nil
}

// All returns all registered handlers in stable order (sorted by canonical plural name).
func All() []ResourceHandler {
	names := make([]string, 0, len(handlers))
	for k := range handlers {
		names = append(names, k)
	}
	slices.Sort(names)
	result := make([]ResourceHandler, 0, len(names))
	for _, n := range names {
		result = append(result, handlers[n])
	}
	return result
}

// GVR is a convenience constructor for schema.GroupVersionResource.
func GVR(group, version, resource string) schema.GroupVersionResource {
	return schema.GroupVersionResource{Group: group, Version: version, Resource: resource}
}

// Mapping constructs a RESTMapping from a GVR and kind.
func Mapping(gvr schema.GroupVersionResource, kind string) meta.RESTMapping {
	return meta.RESTMapping{
		Resource:         gvr,
		GroupVersionKind: schema.GroupVersionKind{Group: gvr.Group, Version: gvr.Version, Kind: kind},
	}
}

func canonicalName(s string) string {
	return strings.ToLower(s)
}

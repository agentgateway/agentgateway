package remotecache

import (
	"strings"
	"time"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/slices"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

// CollapseSources collapses sources sharing a fetch key: lowest ownerKey
// (stable across input order) and minimum TTL. Panics on len 0.
func CollapseSources[S any](sources []S, ownerKey func(S) string, ttl func(S) time.Duration) (S, time.Duration) {
	primary := slices.MinFunc(sources, func(a, b S) int {
		return strings.Compare(ownerKey(a), ownerKey(b))
	})
	return primary, slices.Min(slices.Map(sources, ttl))
}

// NewSharedRequestCollection groups sources by FetchKey and collapses each group
// via `collapse`. namePrefix (e.g. "jwks", "oidc") names the three derived KRT
// collections consistently.
func NewSharedRequestCollection[S any, R any](
	sources krt.Collection[S],
	namePrefix string,
	krtOpts krtutil.KrtOptions,
	requestKey func(S) remotehttp.FetchKey,
	collapse func(krt.IndexObject[remotehttp.FetchKey, S]) *R,
) krt.Collection[R] {
	byRequestKey := krt.NewIndex(sources, namePrefix+"-request-key", func(source S) []remotehttp.FetchKey {
		return []remotehttp.FetchKey{requestKey(source)}
	})
	groups := byRequestKey.AsCollection(append(krtOpts.ToOptions(namePrefix+"/requestGroups"), FetchKeyIndexCollectionOption)...)
	return krt.NewCollection(groups, func(kctx krt.HandlerContext, grouped krt.IndexObject[remotehttp.FetchKey, S]) *R {
		return collapse(grouped)
	}, krtOpts.ToOptions(namePrefix+"/sharedRequests")...)
}

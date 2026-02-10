package deployer

import (
	"context"

	"k8s.io/client-go/tools/cache"
	"sigs.k8s.io/controller-runtime/pkg/client"
)

// Deployer uses HelmValueGenerator implementation to generate a set of helm values
// when rendering a Helm chart
type HelmValuesGenerator interface {
	// GetValues returns the helm values used to render the dynamically provisioned resources
	// for the given object (e.g. Gateway). If the values returned are nil, it indicates that
	// the object is self-managed and no resources should be provisioned.
	GetValues(ctx context.Context, obj client.Object) (map[string]any, error)

	// GetCacheSyncHandlers returns the cache sync handlers for the HelmValuesGenerator controller
	GetCacheSyncHandlers() []cache.InformerSynced
}

// ObjectPostProcessor is an optional interface that can be implemented by HelmValuesGenerator
// to post-process rendered objects before they are deployed. This is used for applying
// strategic merge patch overlays from AgentgatewayParameters.
type ObjectPostProcessor interface {
	// PostProcessObjects applies any post-processing to the rendered objects.
	// This is called after helm rendering but before deployment.
	// It returns the (potentially modified) slice of objects, as new objects may be added
	// (e.g., PodDisruptionBudget, HorizontalPodAutoscaler).
	PostProcessObjects(ctx context.Context, obj client.Object, rendered []client.Object) ([]client.Object, error)
}

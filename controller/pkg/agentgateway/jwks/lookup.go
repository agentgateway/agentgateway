package jwks

import (
	"fmt"

	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
)

type Lookup interface {
	InlineForOwner(krtctx krt.HandlerContext, owner RemoteJwksOwner) (string, error)
}

type lookup struct {
	resolver Resolver
	cache    *artifactCache
}

func NewLookup(configMaps krt.Collection[*corev1.ConfigMap], resolver Resolver, storePrefix, storeNamespace string) Lookup {
	return &lookup{
		resolver: resolver,
		cache:    newArtifactCache(configMaps, storePrefix, storeNamespace),
	}
}

func (l *lookup) InlineForOwner(krtctx krt.HandlerContext, owner RemoteJwksOwner) (string, error) {
	resolved, err := l.resolver.ResolveOwner(krtctx, owner)
	if err != nil {
		return "", err
	}

	artifact, ok := l.cache.Get(krtctx, RequestKey(resolved.Endpoint.Key))
	if !ok {
		return "", fmt.Errorf("jwks artifact for request key %s isn't available", resolved.Endpoint.Key)
	}
	return artifact.JwksJSON, nil
}

// artifactCache provides an in-memory view of persisted JWKS artifacts. It is
// hydrated by the ConfigMap informer, so lookup does not depend on raw
// ConfigMap fetches or persistence naming details.
type artifactCache struct {
	storePrefix string
	artifacts   krt.Collection[cachedArtifact]
}

type cachedArtifact struct {
	Name string
	Artifact
}

func (c cachedArtifact) ResourceName() string {
	return c.Name
}

func (c cachedArtifact) Equals(other cachedArtifact) bool {
	return c.Name == other.Name && c.Artifact == other.Artifact
}

func newArtifactCache(configMaps krt.Collection[*corev1.ConfigMap], storePrefix, storeNamespace string) *artifactCache {
	artifacts := krt.NewCollection(configMaps, func(krtctx krt.HandlerContext, cm *corev1.ConfigMap) *cachedArtifact {
		if cm.Namespace != storeNamespace {
			return nil
		}

		artifact, err := JwksFromConfigMap(cm)
		if err != nil {
			return nil
		}

		cached := cachedArtifact{Name: cm.Name, Artifact: artifact}
		return &cached
	})

	return &artifactCache{
		storePrefix: storePrefix,
		artifacts:   artifacts,
	}
}

func (c *artifactCache) Get(krtctx krt.HandlerContext, requestKey RequestKey) (Artifact, bool) {
	artifact := krt.FetchOne(krtctx, c.artifacts, krt.FilterKey(JwksConfigMapName(c.storePrefix, requestKey)))
	if artifact == nil {
		return Artifact{}, false
	}
	return artifact.Artifact, true
}

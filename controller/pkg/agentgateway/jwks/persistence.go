package jwks

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"

	"istio.io/istio/pkg/kube/kclient"
	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/types"
	"sigs.k8s.io/controller-runtime/pkg/log"

	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

const configMapKey = "jwks-store"
const jwksStoreComponentLabel = "app.kubernetes.io/component"
const storedArtifactVersion = 1

func JwksStoreLabelSelector(storePrefix string) string {
	return jwksStoreComponentLabel + "=" + storePrefix
}

func JwksStoreConfigMapLabel(storePrefix string) map[string]string {
	return map[string]string{jwksStoreComponentLabel: storePrefix}
}

type configMapSyncer struct {
	storePrefix         string
	deploymentNamespace string
	cmCollection        krt.Collection[*corev1.ConfigMap]
}

func newConfigMapSyncer(client apiclient.Client, storePrefix, deploymentNamespace string, krtOptions krtutil.KrtOptions) *configMapSyncer {
	cmCollection := krt.NewFilteredInformer[*corev1.ConfigMap](client,
		kclient.Filter{
			ObjectFilter:  client.ObjectFilter(),
			LabelSelector: JwksStoreLabelSelector(storePrefix)},
		krtOptions.ToOptions("config_map_syncer/ConfigMaps")...)

	return &configMapSyncer{
		deploymentNamespace: deploymentNamespace,
		storePrefix:         storePrefix,
		cmCollection:        cmCollection,
	}
}

func JwksFromConfigMap(cm *corev1.ConfigMap) (Artifact, error) {
	jwksStore := cm.Data[configMapKey]

	var stored storedArtifact
	if err := json.Unmarshal([]byte(jwksStore), &stored); err != nil {
		return Artifact{}, err
	}
	if stored.Version != storedArtifactVersion {
		return Artifact{}, fmt.Errorf("unsupported jwks artifact version %d", stored.Version)
	}
	return stored.Artifact, nil
}

func RequestKeyFromConfigMap(cm *corev1.ConfigMap) (RequestKey, error) {
	artifact, err := JwksFromConfigMap(cm)
	if err != nil {
		return "", err
	}
	return artifact.RequestKey, nil
}

func JwksConfigMapName(storePrefix string, requestKey RequestKey) string {
	sum := sha256.Sum256([]byte(requestKey))
	return fmt.Sprintf("%s-%s", storePrefix, hex.EncodeToString(sum[:]))
}

func JwksConfigMapNamespacedName(storePrefix, namespace string, requestKey RequestKey) types.NamespacedName {
	return types.NamespacedName{
		Namespace: namespace,
		Name:      JwksConfigMapName(storePrefix, requestKey),
	}
}

func SetJwksInConfigMap(cm *corev1.ConfigMap, artifact Artifact) error {
	b, err := json.Marshal(storedArtifact{
		Version:  storedArtifactVersion,
		Artifact: artifact,
	})
	if err != nil {
		return err
	}
	if cm.Data == nil {
		cm.Data = make(map[string]string)
	}
	cm.Data[configMapKey] = string(b)
	return nil
}

func (cs *configMapSyncer) LoadJwksFromConfigMaps(ctx context.Context) ([]Artifact, error) {
	log := log.FromContext(ctx)

	allPersistedJwks := cs.cmCollection.List()
	if len(allPersistedJwks) == 0 {
		return nil, nil
	}

	errs := make([]error, 0)
	artifacts := make([]Artifact, 0, len(allPersistedJwks))
	for _, cm := range allPersistedJwks {
		artifact, err := JwksFromConfigMap(cm)
		if err != nil {
			log.Error(err, "error deserializing jwks ConfigMap", "ConfigMap", cm.Name)
			errs = append(errs, err)
			continue
		}
		artifacts = append(artifacts, artifact)
	}

	return artifacts, errors.Join(errs...)
}

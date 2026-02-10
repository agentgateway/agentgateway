package jwks

import (
	"context"
	"crypto/md5" //nolint:gosec
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"maps"

	"istio.io/istio/pkg/kube/kclient"
	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	"sigs.k8s.io/controller-runtime/pkg/log"

	"github.com/kgateway-dev/kgateway/v2/pkg/apiclient"
	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/krtutil"
)

const configMapKey = "jwks-store"
const jwksStoreComponentLabel = "app.kubernetes.io/component"

func JwksStoreLabelSelector(storePrefix string) string {
	return jwksStoreComponentLabel + "=" + storePrefix
}

func JwksStoreConfigMapLabel(storePrefix string) map[string]string {
	return map[string]string{jwksStoreComponentLabel: storePrefix}
}

// util-like functions to help with handling of ConfigMaps used to store jwks data.
// TODO (dmitri-d) there's probably a better name for this file.
type configMapSyncer struct {
	storePrefix         string
	deploymentNamespace string
	cmCollection        krt.Collection[*corev1.ConfigMap]
}

func NewConfigMapSyncer(client apiclient.Client, storePrefix, deploymentNamespace string, krtOptions krtutil.KrtOptions) *configMapSyncer {
	cmCollection := krt.NewFilteredInformer[*corev1.ConfigMap](client,
		kclient.Filter{
			ObjectFilter:  client.ObjectFilter(),
			LabelSelector: JwksStoreLabelSelector(storePrefix)},
		krtOptions.ToOptions("config_map_syncer/ConfigMaps")...)

	toret := configMapSyncer{
		deploymentNamespace: deploymentNamespace,
		storePrefix:         storePrefix,
		cmCollection:        cmCollection,
	}

	return &toret
}

// Load jwks from a ConfigMap.
// Returns a map of jwks-uri -> jwks (currently one jwks-uri per ConfigMap).
func JwksFromConfigMap(cm *corev1.ConfigMap) (map[string]string, error) {
	jwksStore := cm.Data[configMapKey]
	jwks := make(map[string]string)
	err := json.Unmarshal(([]byte)(jwksStore), &jwks)
	if err != nil {
		return nil, err
	}
	return jwks, nil
}

// Generates ConfigMap name based on jwks uri. Resulting name is a concatenation of "jwks-store-" prefix and an MD5 hash of the jwks uri.
// The length of the name is a constant 32 chars (hash) + legth of the prefix.
func JwksConfigMapName(storePrefix, jwksUri string) string {
	hash := md5.Sum([]byte(jwksUri)) //nolint:gosec
	return fmt.Sprintf("%s-%s", storePrefix, hex.EncodeToString(hash[:]))
}

func SetJwksInConfigMap(cm *corev1.ConfigMap, uri, jwks string) error {
	b, err := json.Marshal(map[string]string{uri: jwks})
	if err != nil {
		return err
	}
	cm.Data[configMapKey] = string(b)
	return nil
}

// Loads all jwks persisted in ConfigMaps. The result is a map of jwks-uri to serialized jwks.
func (cs *configMapSyncer) LoadJwksFromConfigMaps(ctx context.Context) (map[string]string, error) {
	log := log.FromContext(ctx)

	allPersistedJwks := cs.cmCollection.List()

	if len(allPersistedJwks) == 0 {
		return nil, nil
	}

	errs := make([]error, 0)
	toret := make(map[string]string)
	for _, cm := range allPersistedJwks {
		jwks, err := JwksFromConfigMap(cm)
		if err != nil {
			log.Error(err, "error deserializing jwks ConfigMap", "ConfigMap", cm.Name)
			errs = append(errs, err)
			continue
		}

		maps.Copy(toret, jwks)
	}

	return toret, errors.Join(errs...)
}

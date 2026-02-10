package translator

import (
	"testing"

	"github.com/stretchr/testify/assert"
	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/krtutil"
)

func TestDefaultGatewayCollectionOptions(t *testing.T) {
	cfg := getConfig(t)
	processGatewayCollectionOptions(&cfg)

	assert.NotNil(t, cfg.listenerIndex)
	assert.NotNil(t, cfg.transformationFunc)
}

func TestWithGatewayTransformationFunc(t *testing.T) {
	called := false
	customTransformationFunc := func(cfg GatewayCollectionConfig) func(ctx krt.HandlerContext, obj *gwv1.Gateway) (*gwv1.GatewayStatus, []*GatewayListener) {
		called = true
		return func(ctx krt.HandlerContext, obj *gwv1.Gateway) (*gwv1.GatewayStatus, []*GatewayListener) {
			return nil, nil
		}
	}

	GatewayCollection(getConfig(t), WithGatewayTransformationFunc(customTransformationFunc))
	assert.True(t, called)
}

func getConfig(t *testing.T) GatewayCollectionConfig {
	opts := krtutil.NewKrtOptions(t.Context().Done(), nil)
	return GatewayCollectionConfig{
		ControllerName: "random-name",
		Gateways:       krt.NewStaticCollection[*gwv1.Gateway](nil, nil, opts.ToOptions("Gateways")...),
		ListenerSets:   krt.NewStaticCollection[ListenerSet](nil, nil, opts.ToOptions("ListenerSets")...),
		GatewayClasses: krt.NewStaticCollection[GatewayClass](nil, nil, opts.ToOptions("GatewayClasses")...),
		Namespaces:     krt.NewStaticCollection[*corev1.Namespace](nil, nil, opts.ToOptions("Namespaces")...),
		Grants: ReferenceGrants{
			collection: krt.NewStaticCollection[ReferenceGrant](nil, nil, opts.ToOptions("Grants")...),
			index:      nil,
		},
		Secrets:    krt.NewStaticCollection[*corev1.Secret](nil, nil, opts.ToOptions("Secrets")...),
		ConfigMaps: krt.NewStaticCollection[*corev1.ConfigMap](nil, nil, opts.ToOptions("ConfigMaps")...),
		KrtOpts:    opts,
	}
}

package translator

import (
	"istio.io/istio/pkg/kube/krt"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

type GatewayTransformationFunction func(GatewayCollectionConfig) func(ctx krt.HandlerContext, obj *gwv1.Gateway) (*gwv1.GatewayStatus, []*GatewayListener)

type GatewayCollectionConfigOption func(o *GatewayCollectionConfig)

func WithGatewayTransformationFunc(f GatewayTransformationFunction) GatewayCollectionConfigOption {
	return func(o *GatewayCollectionConfig) {
		if f != nil {
			o.transformationFunc = f
		}
	}
}

func processGatewayCollectionOptions(cfg *GatewayCollectionConfig, opts ...GatewayCollectionConfigOption) {
	cfg.listenerIndex = krt.NewIndex(cfg.ListenerSets, "gatewayParent", func(o ListenerSet) []types.NamespacedName {
		return []types.NamespacedName{o.GatewayParent}
	})
	cfg.transformationFunc = GatewayTransformationFunc
	for _, fn := range opts {
		fn(cfg)
	}
}

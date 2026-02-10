package controller

import (
	"context"
	"time"

	"golang.org/x/time/rate"
	"istio.io/istio/pkg/kube/kubetypes"
	"k8s.io/client-go/util/workqueue"
	"sigs.k8s.io/controller-runtime/pkg/certwatcher"
	"sigs.k8s.io/controller-runtime/pkg/manager"

	"github.com/kgateway-dev/kgateway/v2/pkg/apiclient"
	"github.com/kgateway-dev/kgateway/v2/pkg/deployer"
	internaldeployer "github.com/kgateway-dev/kgateway/v2/pkg/kgateway/deployer"
	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/collections"
)

// rateLimiter uses token bucket for overall rate limiting and exponential backoff for per-item rate limiting
var rateLimiter = workqueue.NewTypedMaxOfRateLimiter(
	workqueue.NewTypedItemExponentialFailureRateLimiter[any](500*time.Millisecond, 10*time.Second),
	// 10 qps, 100 bucket size.  This is only for retry speed and its only the overall factor (not per item)
	&workqueue.TypedBucketRateLimiter[any]{Limiter: rate.NewLimiter(rate.Limit(10), 100)},
)

// TODO [danehans]: Refactor so controller config is organized into shared and Gateway/InferencePool-specific controllers.
type GatewayConfig struct {
	Client apiclient.Client
	Mgr    manager.Manager
	// Dev enables development mode for the controller.
	Dev bool
	// AgwControllerName is the name of the agentgateway controller. Any GatewayClass objects
	// managed by this controller must have this name as their ControllerName.
	AgwControllerName string
	// ControlPlane sets the default control plane information the deployer will use.
	ControlPlane deployer.ControlPlaneInfo
	// DiscoveryNamespaceFilter filters namespaced objects based on the discovery namespace filter.
	DiscoveryNamespaceFilter kubetypes.DynamicObjectFilter
	// CommonCollections used to fetch ir.Gateways for the deployer to generate the ports for the proxy service
	CommonCollections *collections.CommonCollections
	// AgentgatewayClassName is the configured agent gateway class name.
	AgentgatewayClassName string
	// Additional GatewayClass definitions to support extending to other well-known gateway classes
	AdditionalGatewayClasses map[string]*deployer.GatewayClassInfo
	// CertWatcher is the shared certificate watcher for xDS TLS
	CertWatcher *certwatcher.CertWatcher
}

type HelmValuesGeneratorOverrideFunc func(inputs *deployer.Inputs) deployer.HelmValuesGenerator

func NewBaseGatewayController(
	ctx context.Context,
	cfg GatewayConfig,
	classInfos map[string]*deployer.GatewayClassInfo,
	helmValuesGeneratorOverride HelmValuesGeneratorOverrideFunc,
) error {
	logger.Info("starting controllers")

	// Initialize Gateway reconciler
	if err := watchGw(cfg, helmValuesGeneratorOverride); err != nil {
		return nil
	}

	// Initialize GatewayClass reconciler
	if err := cfg.Mgr.Add(newGatewayClassReconciler(cfg, classInfos)); err != nil {
		return err
	}

	return nil
}

func watchGw(
	cfg GatewayConfig,
	helmValuesGeneratorOverride HelmValuesGeneratorOverrideFunc,
) error {
	logger.Info("creating gateway deployer",
		"agwctrlname", cfg.AgwControllerName,
		"server", cfg.ControlPlane.XdsHost,
		"port", cfg.ControlPlane.AgwXdsPort, "tls", cfg.ControlPlane.XdsTLS,
	)

	inputs := &deployer.Inputs{
		Dev:                        cfg.Dev,
		ControlPlane:               cfg.ControlPlane,
		CommonCollections:          cfg.CommonCollections,
		AgentgatewayClassName:      cfg.AgentgatewayClassName,
		AgentgatewayControllerName: cfg.AgwControllerName,
	}

	gwParams := internaldeployer.NewGatewayParameters(cfg.Client, inputs)
	if helmValuesGeneratorOverride != nil {
		gwParams.WithHelmValuesGeneratorOverride(helmValuesGeneratorOverride(inputs))
	}

	d, err := internaldeployer.NewGatewayDeployer(
		cfg.AgwControllerName,
		cfg.AgentgatewayClassName,
		cfg.Mgr.GetScheme(),
		cfg.Client,
		gwParams,
	)
	if err != nil {
		return err
	}

	return cfg.Mgr.Add(NewGatewayReconciler(cfg, d, gwParams))
}

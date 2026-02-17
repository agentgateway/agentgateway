package setup

import (
	"context"

	"istio.io/istio/pkg/kube/kubetypes"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/client-go/rest"
	"k8s.io/client-go/tools/cache"
	ctrl "sigs.k8s.io/controller-runtime"
	"sigs.k8s.io/controller-runtime/pkg/manager"

	apisettings "github.com/agentgateway/agentgateway/controller/api/settings"
	agwplugins "github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/deployer"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/agentgatewaysyncer"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/setup"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/collections"
)

type Options struct {
	APIClient                      apiclient.Client
	ExtraInformerCacheSyncHandlers []cache.InformerSynced
	GatewayControllerExtension     pluginsdk.GatewayControllerExtension

	AgentgatewayControllerName string
	AgentgatewayClassName      string
	AdditionalGatewayClasses   map[string]*deployer.GatewayClassInfo
	ExtraAgwPlugins            func(ctx context.Context, agw *agwplugins.AgwCollections) []agwplugins.AgwPlugin
	// HelmValuesGeneratorOverride allows replacing the default helm values generation logic.
	// When set, this generator will be used instead of the built-in GatewayParameters-based generator
	// for all Gateways. This is a 1:1 replacement - you provide one generator that handles everything.
	HelmValuesGeneratorOverride func(inputs *deployer.Inputs) deployer.HelmValuesGenerator
	RestConfig                  *rest.Config
	CtrlMgrOptions              func(context.Context) *ctrl.Options
	// extra controller manager config, like registering additional controllers
	ExtraManagerConfig []func(context.Context, manager.Manager, kubetypes.DynamicObjectFilter) error
	// ExtraRunnables are additional runnables to add to the manager
	ExtraRunnables []func(ctx context.Context, commoncol *collections.CommonCollections, agw *agwplugins.AgwCollections, s *apisettings.Settings) (bool, manager.Runnable)
	// ExtraAgwResourceStatusHandlers maps resource kinds to their status sync handlers for AgentGateway
	ExtraAgwResourceStatusHandlers map[schema.GroupVersionKind]agwplugins.AgwResourceStatusSyncHandler

	AgentGatewaySyncerOptions []agentgatewaysyncer.AgentgatewaySyncerOption
}

func New(opts Options) (setup.Server, error) {
	// internal setup already accepted functional-options; we wrap only extras.
	return setup.New(
		setup.WithAPIClient(opts.APIClient),
		setup.WithExtraInformerCacheSyncHandlers(opts.ExtraInformerCacheSyncHandlers),
		setup.WithGatewayControllerExtension(opts.GatewayControllerExtension),
		setup.WithExtraAgwPlugins(opts.ExtraAgwPlugins),
		setup.WithHelmValuesGeneratorOverride(opts.HelmValuesGeneratorOverride),
		setup.WithAgwControllerName(opts.AgentgatewayControllerName),
		setup.WithAgentgatewayClassName(opts.AgentgatewayClassName),
		setup.WithAdditionalGatewayClasses(opts.AdditionalGatewayClasses),
		setup.WithRestConfig(opts.RestConfig),
		setup.WithControllerManagerOptions(opts.CtrlMgrOptions),
		setup.WithExtraRunnables(opts.ExtraRunnables...),
		setup.WithExtraManagerConfig(opts.ExtraManagerConfig...),
		setup.WithExtraAgwResourceStatusHandlers(opts.ExtraAgwResourceStatusHandlers),
		setup.WithAgentgatewaySyncerOptions(opts.AgentGatewaySyncerOptions),
	)
}

package controller

import (
	"context"
	"errors"
	"log/slog"
	"maps"
	"net/http"
	"sync/atomic"

	"istio.io/istio/pkg/kube/krt"
	istiolog "istio.io/istio/pkg/log"
	"istio.io/istio/pkg/ptr"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/client-go/rest"
	ctrl "sigs.k8s.io/controller-runtime"
	"sigs.k8s.io/controller-runtime/pkg/certwatcher"
	"sigs.k8s.io/controller-runtime/pkg/manager"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	apisettings "github.com/agentgateway/agentgateway/controller/api/settings"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/jwks"
	agwplugins "github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/deployer"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/agentgatewaysyncer"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/collections"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/namespaces"
	"github.com/agentgateway/agentgateway/controller/pkg/version"
)

// TLSRootCAPath is the path to the TLS root CA
const TLSRootCAPath = "/etc/xds-tls/ca.crt"

type SetupOpts struct {
	KrtDebugger *krt.DebugHandler

	// static set of global Settings
	GlobalSettings *apisettings.Settings

	// CertWatcher is the shared certificate watcher for xDS TLS
	// Used by the Gateway controller to trigger reconciliation on cert changes
	CertWatcher *certwatcher.CertWatcher

	PprofBindAddress       string
	HealthProbeBindAddress string
	MetricsBindAddress     string
}

var setupLog = ctrl.Log.WithName("setup")

type StartConfig struct {
	Manager                  manager.Manager
	AgwControllerName        string
	AgentgatewayClassName    string
	AdditionalGatewayClasses map[string]*deployer.GatewayClassInfo
	GatewayClassInfos        map[string]*deployer.GatewayClassInfo

	Dev             bool
	SetupOpts       *SetupOpts
	RestConfig      *rest.Config
	ExtraAgwPlugins func(ctx context.Context, agw *agwplugins.AgwCollections) []agwplugins.AgwPlugin
	// HelmValuesGeneratorOverride allows replacing the default helm values generation logic.
	// When set, this generator will be used instead of the built-in GatewayParameters-based generator
	// for all Gateways. This is a 1:1 replacement - you provide one generator that handles everything.
	HelmValuesGeneratorOverride HelmValuesGeneratorOverrideFunc

	Client apiclient.Client

	AgwCollections    *agwplugins.AgwCollections
	CommonCollections *collections.CommonCollections

	KrtOptions                     krtutil.KrtOptions
	ExtraAgwResourceStatusHandlers map[schema.GroupVersionKind]agwplugins.AgwResourceStatusSyncHandler

	// GatewayControllerExtension is an extension that can be used to extend Gateway controller
	GatewayControllerExtension pluginsdk.GatewayControllerExtension

	// AgentgatewaySyncerOptions is the list of options to be passed when creating the AgentGatewaySyncer
	AgentgatewaySyncerOptions []agentgatewaysyncer.AgentgatewaySyncerOption
}

// Start runs the controllers responsible for processing the K8s Gateway API objects
// It is intended to be run in a goroutine as the function will block until the supplied
// context is cancelled
type ControllerBuilder struct {
	agwSyncer *agentgatewaysyncer.Syncer
	cfg       StartConfig
	mgr       ctrl.Manager
	commoncol *collections.CommonCollections

	ready atomic.Bool
}

func NewControllerBuilder(ctx context.Context, cfg StartConfig) (*ControllerBuilder, error) {
	loggingOptions := istiolog.DefaultOptions()
	loggingOptions.JSONEncoding = true
	if cfg.Dev {
		setupLog.Info("starting log in dev mode")
		loggingOptions.SetDefaultOutputLevel(istiolog.OverrideScopeName, istiolog.DebugLevel)
	}
	istiolog.Configure(loggingOptions)

	setupLog.Info("initializing agentgateway extensions")

	// TODO: re-enable metrics processing https://github.com/agentgateway/agentgateway/issues/970
	// Begin background processing of resource sync metrics.
	// This only effects metrics in the resources subsystem and is not required for other metrics.
	//metrics.StartResourceSyncMetricsProcessing(ctx)

	runtimeDeps := newRuntimeDeps(cfg.AgwCollections)
	agwMergedPlugins := agwPluginFactory(cfg, runtimeDeps.JWKSLookup)(ctx, cfg.AgwCollections)

	// Compute the extra GVKs list to provide at initialization time
	var gvks []schema.GroupVersionKind
	for gvk := range cfg.ExtraAgwResourceStatusHandlers {
		gvks = append(gvks, gvk)
	}

	agwSyncer := agentgatewaysyncer.NewAgwSyncer(
		cfg.AgwControllerName,
		cfg.Client,
		cfg.AgwCollections,
		agwMergedPlugins,
		runtimeDeps.JWKSLookup,
		cfg.AdditionalGatewayClasses,
		cfg.KrtOptions,
		gvks,
		cfg.AgentgatewaySyncerOptions...,
	)

	if err := cfg.Manager.Add(agwSyncer); err != nil {
		setupLog.Error(err, "unable to add agentgateway Syncer runnable")
		return nil, err
	}

	agwStatusSyncer := agentgatewaysyncer.NewAgwStatusSyncer(
		cfg.AgwControllerName,
		cfg.AgentgatewayClassName,
		cfg.Client,
		agwSyncer.StatusCollections(),
		agwSyncer.CacheSyncs(),
		cfg.ExtraAgwResourceStatusHandlers,
		cfg.CommonCollections.Settings.EnableInferExt,
	)
	if err := cfg.Manager.Add(agwStatusSyncer); err != nil {
		setupLog.Error(err, "unable to add agentgateway StatusSyncer runnable")
		return nil, err
	}

	setupLog.Info("starting controller builder")
	cb := &ControllerBuilder{
		agwSyncer: agwSyncer,
		cfg:       cfg,
		mgr:       cfg.Manager,
		commoncol: cfg.CommonCollections,
	}

	// wait for the ControllerBuilder to Start
	// as well as its subcomponents (mainly ProxySyncer) before marking ready
	if err := cfg.Manager.AddReadyzCheck("ready-ping", func(_ *http.Request) error {
		if !cb.HasSynced() {
			return errors.New("not synced")
		}
		return nil
	}); err != nil {
		setupLog.Error(err, "failed setting up healthz")
	}

	return cb, nil
}

type runtimeDeps struct {
	JWKSLookup jwks.Lookup
}

func newRuntimeDeps(agw *agwplugins.AgwCollections) runtimeDeps {
	remoteHTTPResolver := remotehttp.NewResolver(remotehttp.Inputs{
		ConfigMaps:           agw.ConfigMaps,
		Services:             agw.Services,
		Backends:             agw.Backends,
		AgentgatewayPolicies: agw.AgentgatewayPolicies,
		BackendTLSPolicies:   agw.BackendTLSPolicies,
	})
	jwksResolver := jwks.NewResolver(remoteHTTPResolver)
	jwksLookup := jwks.NewLookup(
		agw.ConfigMaps,
		jwksResolver,
		jwks.DefaultJwksStorePrefix,
		agw.SystemNamespace,
	)

	return runtimeDeps{
		JWKSLookup: jwksLookup,
	}
}

func agwPluginFactory(cfg StartConfig, jwksLookup jwks.Lookup) func(ctx context.Context, agw *agwplugins.AgwCollections) agwplugins.AgwPlugin {
	return func(ctx context.Context, agw *agwplugins.AgwCollections) agwplugins.AgwPlugin {
		plugins := BuiltinAgwPlugins(agw, jwksLookup)
		if cfg.ExtraAgwPlugins != nil {
			plugins = append(plugins, cfg.ExtraAgwPlugins(ctx, agw)...)
		}
		return agwplugins.MergePlugins(plugins...)
	}
}

func defaultProxyImageTag(globalSettings *apisettings.Settings) *string {
	if globalSettings.ProxyImageTag != nil {
		return globalSettings.ProxyImageTag
	}

	// version.Version is unprefixed semver-like data such as 1.0.1-dev. Downstream
	// chart/deployer paths handle adding a single "v" when appropriate.
	if version.Version != "" {
		return ptr.Of(version.Version)
	}

	return ptr.Of(version.GitVersion)
}

func (c *ControllerBuilder) Build(ctx context.Context) (*agentgatewaysyncer.Syncer, error) {
	slog.Info("creating gateway controllers")

	globalSettings := c.cfg.SetupOpts.GlobalSettings

	xdsHost := globalSettings.XdsServiceHost
	if xdsHost == "" {
		xdsHost = kubeutils.ServiceFQDN(metav1.ObjectMeta{
			Name:      globalSettings.XdsServiceName,
			Namespace: namespaces.GetPodNamespace(),
		})
	}

	agwXdsPort := globalSettings.AgentgatewayXdsServicePort
	slog.Info("got agentgateway xds address for deployer", "agw_xds_host", xdsHost, "agw_xds_port", agwXdsPort)

	gwCfg := GatewayConfig{
		Client:            c.cfg.Client,
		Mgr:               c.mgr,
		AgwControllerName: c.cfg.AgwControllerName,
		ImageDefaults: &agentgateway.Image{
			Registry:   &globalSettings.ProxyImageRegistry,
			Repository: &globalSettings.ProxyImageRepository,
			Tag:        defaultProxyImageTag(globalSettings),
		},
		ControlPlane: deployer.ControlPlaneInfo{
			XdsHost:      xdsHost,
			AgwXdsPort:   agwXdsPort,
			XdsTLS:       globalSettings.XdsTLS,
			XdsTlsCaPath: apisettings.TLSRootCAPath,
		},
		ImageInfo: &deployer.ImageInfo{
			Registry:   globalSettings.DefaultImageRegistry,
			Tag:        globalSettings.DefaultImageTag,
			PullPolicy: globalSettings.DefaultImagePullPolicy,
		},
		DiscoveryNamespaceFilter: c.cfg.Client.ObjectFilter(),
		CommonCollections:        c.commoncol,
		AgentgatewayClassName:    c.cfg.AgentgatewayClassName,
		CertWatcher:              c.cfg.SetupOpts.CertWatcher,
	}

	setupLog.Info("creating base gateway controller")
	if err := NewBaseGatewayController(
		ctx,
		gwCfg,
		c.cfg.GatewayClassInfos,
		c.cfg.HelmValuesGeneratorOverride,
		c.cfg.GatewayControllerExtension,
	); err != nil {
		setupLog.Error(err, "unable to create gateway controller")
		return nil, err
	}

	// TODO (dmitri-d) don't think c.ready field is used anywhere and can be removed
	// mgr WaitForCacheSync is part of proxySyncer's HasSynced
	// so we can mark ready here before we call mgr.Start
	c.ready.Store(true)
	return c.agwSyncer, nil
}

func (c *ControllerBuilder) HasSynced() bool {
	if c.agwSyncer != nil && !c.agwSyncer.HasSynced() {
		return false
	}
	return true
}

// GetDefaultClassInfo returns the default GatewayClass for the agentgateway controller.
// Exported for testing.
func GetDefaultClassInfo(
	globalSettings *apisettings.Settings,
	agwClassName,
	agwControllerName string,
	additionalClassInfos map[string]*deployer.GatewayClassInfo,
) map[string]*deployer.GatewayClassInfo {
	classInfos := map[string]*deployer.GatewayClassInfo{}
	refOverrides := globalSettings.GatewayClassParametersRefs
	// Only enable agentgateway gateway class if it's enabled in the settings
	logger.Info("enabling agentgateway gateway class")
	classInfos[agwClassName] = &deployer.GatewayClassInfo{
		Description:       "Specialized class for agentgateway.",
		Labels:            map[string]string{},
		Annotations:       map[string]string{},
		ControllerName:    agwControllerName,
		SupportedFeatures: deployer.GetSupportedFeaturesForAgentGateway(),
	}
	applyGatewayClassParametersRef(classInfos[agwClassName], agwClassName, refOverrides)
	maps.Copy(classInfos, additionalClassInfos)
	return classInfos
}

func applyGatewayClassParametersRef(info *deployer.GatewayClassInfo, className string, refs apisettings.GatewayClassParametersRefs) {
	if info == nil || len(refs) == 0 {
		return
	}
	ref, ok := refs[className]
	if !ok || ref == nil || ref.Name == "" {
		return
	}

	// Set default Group and Kind if not provided
	// Use AgentgatewayParametersGVK for agentgateway class, GatewayParametersGVK for others
	paramsRef := *ref
	if paramsRef.Group == "" || paramsRef.Kind == "" {
		defaultGVK := wellknown.AgentgatewayParametersGVK
		if paramsRef.Group == "" {
			paramsRef.Group = gwv1.Group(defaultGVK.Group)
		}
		if paramsRef.Kind == "" {
			paramsRef.Kind = gwv1.Kind(defaultGVK.Kind)
		}
	}

	info.ParametersRef = &paramsRef
}

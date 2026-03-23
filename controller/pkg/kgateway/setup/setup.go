package setup

import (
	"context"
	"fmt"
	"log/slog"
	"net"
	"sync"

	"github.com/go-logr/logr"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/kube/kubetypes"
	"istio.io/istio/pkg/security"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/client-go/rest"
	"k8s.io/client-go/tools/cache"
	"k8s.io/klog/v2"
	ctrl "sigs.k8s.io/controller-runtime"
	"sigs.k8s.io/controller-runtime/pkg/certwatcher"
	"sigs.k8s.io/controller-runtime/pkg/manager"
	metricsserver "sigs.k8s.io/controller-runtime/pkg/metrics/server"

	apisettings "github.com/agentgateway/agentgateway/controller/api/settings"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/jwks"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/jwks_url"
	agentjwksstore "github.com/agentgateway/agentgateway/controller/pkg/agentgateway/jwksstore"
	agwplugins "github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/common"
	"github.com/agentgateway/agentgateway/controller/pkg/deployer"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/admin"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/agentgatewaysyncer"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/controller"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
	"github.com/agentgateway/agentgateway/controller/pkg/metrics"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/collections"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/schemes"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/namespaces"
)

type Server interface {
	Start(ctx context.Context) error
}

type Options struct {
	APIClient                      apiclient.Client
	ExtraInformerCacheSyncHandlers []cache.InformerSynced
	GatewayControllerExtension     pluginsdk.GatewayControllerExtension

	AgentgatewayControllerName     string
	AgentgatewayClassName          string
	AdditionalGatewayClasses       map[string]*deployer.GatewayClassInfo
	ExtraAgwPlugins                func(ctx context.Context, agw *agwplugins.AgwCollections) []agwplugins.AgwPlugin
	HelmValuesGeneratorOverride    func(inputs *deployer.Inputs) deployer.HelmValuesGenerator
	AgwXDSListener                 net.Listener
	RestConfig                     *rest.Config
	CtrlMgrOptions                 func(context.Context) *ctrl.Options
	ExtraManagerConfig             []func(context.Context, manager.Manager, kubetypes.DynamicObjectFilter) error
	ExtraRunnables                 []func(ctx context.Context, commoncol *collections.CommonCollections, agw *agwplugins.AgwCollections, s *apisettings.Settings) (bool, manager.Runnable)
	KrtDebugger                    *krt.DebugHandler
	GlobalSettings                 *apisettings.Settings
	LeaderElectionID               string
	ExtraAgwResourceStatusHandlers map[schema.GroupVersionKind]agwplugins.AgwResourceStatusSyncHandler

	AgentGatewaySyncerOptions []agentgatewaysyncer.AgentgatewaySyncerOption
}

type setup struct {
	Options
}

var _ Server = &setup{}

// ensure global logger wiring happens once to avoid data races
var setLoggerOnce sync.Once

func New(opts Options) (*setup, error) {
	s := &setup{
		Options: opts,
	}

	if s.AgentgatewayControllerName == "" {
		s.AgentgatewayControllerName = wellknown.DefaultAgwControllerName
	}
	if s.AgentgatewayClassName == "" {
		s.AgentgatewayClassName = wellknown.DefaultAgwClassName
	}
	if s.LeaderElectionID == "" {
		s.LeaderElectionID = wellknown.LeaderElectionID
	}

	if s.GlobalSettings == nil {
		var err error
		s.GlobalSettings, err = apisettings.BuildSettings()
		if err != nil {
			slog.Error("error loading settings from env", "error", err)
			return nil, err
		}
	}

	SetupLogging(s.GlobalSettings.LogLevel)

	if s.RestConfig == nil {
		s.RestConfig = ctrl.GetConfigOrDie()
	}
	if s.APIClient == nil {
		apiClient, err := apiclient.New(s.RestConfig)
		if err != nil {
			return nil, fmt.Errorf("error creating API client: %w", err)
		}
		s.APIClient = apiClient
	}

	// Adjust leader election ID based on which controllers are enabled.
	// This allows split helm charts to deploy separate controllers that don't compete for the same lease.
	// When only one controller type is enabled, append a suffix to make the lease unique.
	leaderElectionID := s.LeaderElectionID + "-agentgateway"
	// If both are enabled, use the default ID (single controller handling both)

	if s.CtrlMgrOptions == nil {
		s.CtrlMgrOptions = func(ctx context.Context) *ctrl.Options {
			return &ctrl.Options{
				BaseContext:      func() context.Context { return ctx },
				Scheme:           runtime.NewScheme(),
				PprofBindAddress: "127.0.0.1:9099",
				// if you change the port here, also change the port "health" in the helmchart.
				HealthProbeBindAddress: ":9093",
				Metrics: metricsserver.Options{
					BindAddress: ":9092",
				},
				LeaderElectionNamespace: namespaces.GetPodNamespace(),
				LeaderElection:          !s.GlobalSettings.DisableLeaderElection,
				LeaderElectionID:        leaderElectionID,
			}
		}
	}

	if s.KrtDebugger == nil {
		s.KrtDebugger = new(krt.DebugHandler)
	}

	var err error
	if s.AgwXDSListener == nil {
		s.AgwXDSListener, err = newXDSListener("0.0.0.0", s.GlobalSettings.AgentgatewayXdsServicePort)
		if err != nil {
			slog.Error("error creating agw xds listener", "error", err)
			return nil, err
		}
	}

	return s, nil
}

func (s *setup) Start(ctx context.Context) error {
	slog.Info("starting kgateway")

	mgrOpts := s.CtrlMgrOptions(ctx)

	metrics.SetRegistry(s.GlobalSettings.EnableBuiltinDefaultMetrics, nil)
	metrics.SetActive(!(mgrOpts.Metrics.BindAddress == "" || mgrOpts.Metrics.BindAddress == "0"))

	mgr, err := ctrl.NewManager(s.RestConfig, *mgrOpts)
	if err != nil {
		return err
	}

	if err := schemes.AddToScheme(mgr.GetScheme()); err != nil {
		slog.Error("unable to extend scheme", "error", err)
		return err
	}

	authenticators := []security.Authenticator{
		NewKubeJWTAuthenticator(s.APIClient.Kube()),
	}

	// Create shared certificate watcher if TLS is enabled. This watcher is used by both the xDS server
	// and the Gateway controller to kick reconciliation on cert changes.
	var certWatcher *certwatcher.CertWatcher
	if s.GlobalSettings.XdsTLS {
		var err error
		certWatcher, err = certwatcher.New(apisettings.TLSCertPath, apisettings.TLSKeyPath)
		if err != nil {
			return err
		}
		go func() {
			if err := certWatcher.Start(ctx); err != nil {
				slog.Error("failed to start TLS certificate watcher", "error", err)
			}
			slog.Info("started TLS certificate watcher")
		}()
	}

	setupOpts := &controller.SetupOpts{
		KrtDebugger:    s.KrtDebugger,
		GlobalSettings: s.GlobalSettings,
		CertWatcher:    certWatcher,
	}

	slog.Info("creating krt collections")
	krtOpts := krtutil.NewKrtOptions(ctx.Done(), setupOpts.KrtDebugger)

	commoncol, err := collections.NewCommonCollections(
		krtOpts,
		s.APIClient,
		s.AgentgatewayControllerName,
		*s.GlobalSettings,
	)
	if err != nil {
		slog.Error("error creating common collections", "error", err)
		return err
	}

	agwCollections, err := agwplugins.NewAgwCollections(
		commoncol,
		s.AgentgatewayControllerName,
		// control plane system namespace (default is agentgateway-system)
		namespaces.GetPodNamespace(),
		s.APIClient.ClusterID().String(),
	)
	if err != nil {
		slog.Error("error creating agw common collections", "error", err)
		return err
	}

	jwksUrlFactory := jwks_url.NewJwksUrlFactory(agwCollections.ConfigMaps, agwCollections.Backends, agwCollections.AgentgatewayPolicies)
	jwks_url.JwksUrlBuilderFactory = func() jwks_url.JwksUrlBuilder { return jwksUrlFactory }

	for _, mgrCfgFunc := range s.ExtraManagerConfig {
		err := mgrCfgFunc(ctx, mgr, s.APIClient.ObjectFilter())
		if err != nil {
			return err
		}
	}

	runnablesRegistry := make(map[string]any)
	for _, runnable := range s.ExtraRunnables {
		enabled, r := runnable(ctx, commoncol, agwCollections, s.GlobalSettings)
		if !enabled {
			continue
		}
		if named, ok := r.(common.NamedRunnable); ok {
			runnablesRegistry[named.RunnableName()] = struct{}{}
		}
		if err := mgr.Add(r); err != nil {
			return fmt.Errorf("error adding extra Runnable to manager: %w", err)
		}
	}

	// rebuild jwks store if it doesn't exist
	if _, exists := runnablesRegistry[jwks.RunnableName]; !exists {
		if err := buildJwksStore(ctx, mgr, s.APIClient, commoncol, agwCollections); err != nil {
			return fmt.Errorf("error creating jwks store %w", err)
		}
	}

	agw, err := s.buildKgatewayWithConfig(ctx, mgr, setupOpts, commoncol, agwCollections)
	if err != nil {
		return err
	}

	if s.AgwXDSListener != nil && agw != nil {
		NewAgwControlPlane(ctx, s.AgwXDSListener, authenticators, s.GlobalSettings.XdsAuth, certWatcher, agw.NackPublisher, agw.Registrations...)
	}

	slog.Info("starting admin server")
	go admin.RunAdminServer(ctx, setupOpts)

	slog.Info("starting manager")
	return mgr.Start(ctx)
}

func newXDSListener(ip string, port uint32) (net.Listener, error) {
	bindAddr := net.TCPAddr{IP: net.ParseIP(ip), Port: int(port)}
	return net.Listen(bindAddr.Network(), bindAddr.String())
}

func (s *setup) buildKgatewayWithConfig(
	ctx context.Context,
	mgr manager.Manager,
	setupOpts *controller.SetupOpts,
	commonCollections *collections.CommonCollections,
	agwCollections *agwplugins.AgwCollections,
) (*agentgatewaysyncer.Syncer, error) {
	slog.Info("creating krt collections")
	krtOpts := krtutil.NewKrtOptions(ctx.Done(), setupOpts.KrtDebugger)

	gatewayClassInfos := controller.GetDefaultClassInfo(
		setupOpts.GlobalSettings,
		s.AgentgatewayClassName,
		s.AgentgatewayControllerName,
		s.AdditionalGatewayClasses,
	)

	slog.Info("initializing controller")
	c, err := controller.NewControllerBuilder(ctx, controller.StartConfig{
		Manager:                        mgr,
		AgwControllerName:              s.AgentgatewayControllerName,
		AgentgatewayClassName:          s.AgentgatewayClassName,
		AdditionalGatewayClasses:       s.AdditionalGatewayClasses,
		GatewayClassInfos:              gatewayClassInfos,
		ExtraAgwPlugins:                s.ExtraAgwPlugins,
		HelmValuesGeneratorOverride:    s.HelmValuesGeneratorOverride,
		RestConfig:                     s.RestConfig,
		SetupOpts:                      setupOpts,
		Client:                         s.APIClient,
		Dev:                            logging.MustGetLevel(logging.DefaultComponent) <= logging.LevelTrace,
		KrtOptions:                     krtOpts,
		CommonCollections:              commonCollections,
		AgwCollections:                 agwCollections,
		ExtraAgwResourceStatusHandlers: s.ExtraAgwResourceStatusHandlers,
		GatewayControllerExtension:     s.GatewayControllerExtension,
		AgentgatewaySyncerOptions:      s.AgentGatewaySyncerOptions,
	})
	if err != nil {
		slog.Error("failed initializing controller: ", "error", err)
		return nil, err
	}

	slog.Info("waiting for cache sync")

	agwSyncer, err := c.Build(ctx)
	if err != nil {
		return nil, err
	}

	// RunAndWait must be called AFTER all Informers clients have been created
	s.APIClient.RunAndWait(ctx.Done())

	// Wait for extra Informer caches to sync
	s.APIClient.WaitForCacheSync("extra-informers", ctx.Done(), s.ExtraInformerCacheSyncHandlers...)

	return agwSyncer, nil
}

// SetupLogging configures the global slog logger
func SetupLogging(levelStr string) {
	level, err := logging.ParseLevel(levelStr)
	if err != nil {
		slog.Error("failed to parse log level, defaulting to info", "error", err)
		level = slog.LevelInfo
	}
	// set all loggers to the specified level
	logging.Reset(level)
	// set controller-runtime and klog loggers only once to avoid data races with concurrent readers
	setLoggerOnce.Do(func() {
		controllerLogger := logr.FromSlogHandler(logging.New("controller-runtime").Handler())
		ctrl.SetLogger(controllerLogger)
		klogLogger := logr.FromSlogHandler(logging.New("klog").Handler())
		klog.SetLogger(klogLogger)
	})
}

func buildJwksStore(ctx context.Context, mgr manager.Manager, apiClient apiclient.Client, commonCollections *collections.CommonCollections, agwCollections *agwplugins.AgwCollections) error {
	jwksStorePolicyCtrl := agentjwksstore.NewJWKSStorePolicyController(apiClient, agwCollections, jwks_url.JwksUrlBuilderFactory)
	if err := mgr.Add(jwksStorePolicyCtrl); err != nil {
		return err
	}
	jwksStorePolicyCtrl.Init(ctx)

	jwksStore := jwks.BuildJwksStore(ctx, apiClient, commonCollections, jwksStorePolicyCtrl.JwksChanges(), jwks.DefaultJwksStorePrefix, namespaces.GetPodNamespace())
	if err := mgr.Add(jwksStore); err != nil {
		return err
	}

	jwksStoreCMCtrl := agentjwksstore.NewJWKSStoreConfigMapsController(apiClient, jwks.DefaultJwksStorePrefix, namespaces.GetPodNamespace(), jwksStore)
	jwksStoreCMCtrl.Init(ctx)
	if err := mgr.Add(jwksStoreCMCtrl); err != nil {
		return err
	}

	return nil
}

package setup

import (
	"context"
	"fmt"
	"log/slog"
	"net"
	"sync"

	"github.com/go-logr/logr"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/security"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/client-go/rest"
	"k8s.io/klog/v2"
	ctrl "sigs.k8s.io/controller-runtime"
	"sigs.k8s.io/controller-runtime/pkg/certwatcher"
	"sigs.k8s.io/controller-runtime/pkg/manager"
	metricsserver "sigs.k8s.io/controller-runtime/pkg/metrics/server"

	"github.com/kgateway-dev/kgateway/v2/api/settings"
	apisettings "github.com/kgateway-dev/kgateway/v2/api/settings"
	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/jwks"
	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/jwks_url"
	agentjwksstore "github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/jwksstore"
	agwplugins "github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/plugins"
	"github.com/kgateway-dev/kgateway/v2/pkg/apiclient"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/admin"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/agentgatewaysyncer"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/controller"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
	"github.com/kgateway-dev/kgateway/v2/pkg/logging"
	"github.com/kgateway-dev/kgateway/v2/pkg/metrics"
	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/collections"
	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/krtutil"
	"github.com/kgateway-dev/kgateway/v2/pkg/schemes"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/namespaces"
)

type Server interface {
	Start(ctx context.Context) error
}

type setup struct {
	apiClient                    apiclient.Client
	agwControllerName            string
	agentgatewayClassName        string
	agwXdsListener               net.Listener
	restConfig                   *rest.Config
	ctrlMgrOptionsInitFunc       func(context.Context) *ctrl.Options
	krtDebugger                  *krt.DebugHandler
	globalSettings               *apisettings.Settings
	leaderElectionID             string
	extraAgwPolicyStatusHandlers map[schema.GroupVersionKind]agwplugins.AgwResourceStatusSyncHandler

	agentgatewaySyncerOptions []agentgatewaysyncer.AgentgatewaySyncerOption
}

var _ Server = &setup{}

// ensure global logger wiring happens once to avoid data races
var setLoggerOnce sync.Once

func New() (*setup, error) {
	s := &setup{
		agwControllerName:     wellknown.DefaultAgwControllerName,
		agentgatewayClassName: wellknown.DefaultAgwClassName,
		leaderElectionID:      wellknown.LeaderElectionID,
	}

	if s.globalSettings == nil {
		var err error
		s.globalSettings, err = apisettings.BuildSettings()
		if err != nil {
			slog.Error("error loading settings from env", "error", err)
			return nil, err
		}
	}

	SetupLogging(s.globalSettings.LogLevel)

	if s.restConfig == nil {
		s.restConfig = ctrl.GetConfigOrDie()
	}
	if s.apiClient == nil {
		apiClient, err := apiclient.New(s.restConfig)
		if err != nil {
			return nil, fmt.Errorf("error creating API client: %w", err)
		}
		s.apiClient = apiClient
	}

	// Adjust leader election ID based on which controllers are enabled.
	// This allows split helm charts to deploy separate controllers that don't compete for the same lease.
	// When only one controller type is enabled, append a suffix to make the lease unique.
	leaderElectionID := s.leaderElectionID + "-agentgateway"
	// If both are enabled, use the default ID (single controller handling both)

	if s.ctrlMgrOptionsInitFunc == nil {
		s.ctrlMgrOptionsInitFunc = func(ctx context.Context) *ctrl.Options {
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
				LeaderElection:          !s.globalSettings.DisableLeaderElection,
				LeaderElectionID:        leaderElectionID,
			}
		}
	}

	if s.krtDebugger == nil {
		s.krtDebugger = new(krt.DebugHandler)
	}

	if s.agwXdsListener == nil {
		var err error
		s.agwXdsListener, err = newXDSListener("0.0.0.0", s.globalSettings.AgentgatewayXdsServicePort)
		if err != nil {
			slog.Error("error creating agw xds listener", "error", err)
			return nil, err
		}
	}

	return s, nil
}

func (s *setup) Start(ctx context.Context) error {
	slog.Info("starting kgateway")

	mgrOpts := s.ctrlMgrOptionsInitFunc(ctx)

	metrics.SetRegistry(s.globalSettings.EnableBuiltinDefaultMetrics, nil)
	metrics.SetActive(!(mgrOpts.Metrics.BindAddress == "" || mgrOpts.Metrics.BindAddress == "0"))

	mgr, err := ctrl.NewManager(s.restConfig, *mgrOpts)
	if err != nil {
		return err
	}

	if err := schemes.AddToScheme(mgr.GetScheme()); err != nil {
		slog.Error("unable to extend scheme", "error", err)
		return err
	}

	authenticators := []security.Authenticator{
		NewKubeJWTAuthenticator(s.apiClient.Kube()),
	}

	// Create shared certificate watcher if TLS is enabled. This watcher is used by both the xDS server
	// and the Gateway controller to kick reconciliation on cert changes.
	var certWatcher *certwatcher.CertWatcher
	if s.globalSettings.XdsTLS {
		var err error
		certWatcher, err = certwatcher.New(settings.TLSCertPath, settings.TLSKeyPath)
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
		KrtDebugger:    s.krtDebugger,
		GlobalSettings: s.globalSettings,
		CertWatcher:    certWatcher,
	}

	slog.Info("creating krt collections")
	krtOpts := krtutil.NewKrtOptions(ctx.Done(), setupOpts.KrtDebugger)

	commoncol, err := collections.NewCommonCollections(
		krtOpts,
		s.apiClient,
		s.agwControllerName,
		*s.globalSettings,
	)
	if err != nil {
		slog.Error("error creating common collections", "error", err)
		return err
	}

	agwCollections, err := agwplugins.NewAgwCollections(
		commoncol,
		s.agwControllerName,
		// control plane system namespace (default is kgateway-system)
		namespaces.GetPodNamespace(),
		s.apiClient.ClusterID().String(),
	)
	if err != nil {
		slog.Error("error creating agw common collections", "error", err)
		return err
	}

	jwksUrlFactory := jwks_url.NewJwksUrlFactory(agwCollections.ConfigMaps, agwCollections.Backends, agwCollections.AgentgatewayPolicies)
	jwks_url.JwksUrlBuilderFactory = func() jwks_url.JwksUrlBuilder { return jwksUrlFactory }

	if err := buildJwksStore(ctx, mgr, s.apiClient, commoncol, agwCollections); err != nil {
		return fmt.Errorf("error creating jwks store %w", err)
	}

	agw, err := s.buildKgatewayWithConfig(ctx, mgr, setupOpts, commoncol, agwCollections)
	if err != nil {
		return err
	}

	if s.agwXdsListener != nil && agw != nil {
		NewAgwControlPlane(ctx, s.agwXdsListener, authenticators, s.globalSettings.XdsAuth, certWatcher, agw.NackPublisher, agw.Registrations...)
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
		s.agentgatewayClassName,
		s.agwControllerName,
	)

	slog.Info("initializing controller")
	c, err := controller.NewControllerBuilder(ctx, controller.StartConfig{
		Manager:                        mgr,
		AgwControllerName:              s.agwControllerName,
		AgentgatewayClassName:          s.agentgatewayClassName,
		GatewayClassInfos:              gatewayClassInfos,
		RestConfig:                     s.restConfig,
		SetupOpts:                      setupOpts,
		Client:                         s.apiClient,
		Dev:                            logging.MustGetLevel(logging.DefaultComponent) <= logging.LevelTrace,
		KrtOptions:                     krtOpts,
		CommonCollections:              commonCollections,
		AgwCollections:                 agwCollections,
		ExtraAgwResourceStatusHandlers: s.extraAgwPolicyStatusHandlers,
		AgentgatewaySyncerOptions:      s.agentgatewaySyncerOptions,
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
	s.apiClient.RunAndWait(ctx.Done())

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

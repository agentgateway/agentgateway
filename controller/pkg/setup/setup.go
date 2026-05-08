package setup

import (
	"context"
	"fmt"
	"log/slog"
	"net"
	"strings"
	"sync"

	"github.com/go-logr/logr"
	"github.com/soheilhy/cmux"
	"istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/kube/kclient"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/security"
	"istio.io/istio/pkg/util/sets"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/client-go/rest"
	"k8s.io/client-go/tools/cache"
	"k8s.io/klog/v2"
	ctrl "sigs.k8s.io/controller-runtime"
	"sigs.k8s.io/controller-runtime/pkg/manager"
	metricsserver "sigs.k8s.io/controller-runtime/pkg/metrics/server"

	apisettings "github.com/agentgateway/agentgateway/controller/api/settings"
	"github.com/agentgateway/agentgateway/controller/pkg/admin"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/jwks"
	agwplugins "github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/policyselection"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/common"
	"github.com/agentgateway/agentgateway/controller/pkg/controller"
	"github.com/agentgateway/agentgateway/controller/pkg/deployer"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
	"github.com/agentgateway/agentgateway/controller/pkg/metrics"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk"
	pluginsdkcol "github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/collections"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/schemes"
	"github.com/agentgateway/agentgateway/controller/pkg/syncer"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/namespaces"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

type Server interface {
	Start(ctx context.Context) error
}

type Options struct {
	APIClient                      apiclient.Client
	ExtraInformerCacheSyncHandlers []cache.InformerSynced
	GatewayControllerExtension     pluginsdk.GatewayControllerExtension

	ControllerName              string
	AgentgatewayClassName       string
	AdditionalGatewayClasses    map[string]*deployer.GatewayClassInfo
	ExtraAgwPlugins             func(ctx context.Context, agw *agwplugins.AgwCollections) []agwplugins.AgwPlugin
	HelmValuesGeneratorOverride func(inputs *deployer.Inputs) deployer.HelmValuesGenerator
	XDSListener                 net.Listener
	RestConfig                  *rest.Config
	CtrlMgrOptions              func(context.Context) *ctrl.Options
	ExtraManagerConfig          []func(mgr manager.Manager) error
	ExtraRunnables              []func(ctx context.Context, agw *agwplugins.AgwCollections, s *apisettings.Settings) (bool, manager.Runnable)
	KrtDebugger                 *krt.DebugHandler
	GlobalSettings              *apisettings.Settings
	LeaderElectionID            string
	ExtraStatusHandlers         map[schema.GroupVersionKind]syncer.ResourceStatusSyncer

	AgentGatewaySyncerOptions []syncer.AgentgatewaySyncerOption

	// PersistedJWKS overrides the default JWKS persistence layer.
	// When set, setup uses this instance instead of creating one with DefaultJwksStorePrefix.
	PersistedJWKS *jwks.PersistedEntries
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

	if s.ControllerName == "" {
		s.ControllerName = wellknown.DefaultAgwControllerName
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
				PprofBindAddress: "",
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
	if s.XDSListener == nil {
		s.XDSListener, err = newXDSListener("0.0.0.0", s.GlobalSettings.AgentgatewayXdsServicePort)
		if err != nil {
			slog.Error("error creating agw xds listener", "error", err)
			return nil, err
		}
	}

	return s, nil
}

func (s *setup) Start(ctx context.Context) error {
	slog.Info("starting agentgateway")

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
	var certWatcher *xdsTLSMaterial
	if s.GlobalSettings.IsXdsTLSEnabled() {
		var err error
		certWatcher, err = setupXdsTLSMaterial(ctx, s.APIClient, namespaces.GetPodNamespace(), apisettings.TLSSecretName, s.xdsTLSHosts())
		if err != nil {
			return err
		}
	}

	setupOpts := &controller.SetupOpts{
		KrtDebugger:    s.KrtDebugger,
		GlobalSettings: s.GlobalSettings,
		CertWatcher:    certWatcher,
	}

	if err := initDiscoveryNSFilter(ctx, s.APIClient, s.GlobalSettings.DiscoveryNamespaceSelectors); err != nil {
		return err
	}

	slog.Info("creating krt collections")
	krtOpts := krtutil.NewKrtOptions(ctx.Done(), setupOpts.KrtDebugger)

	agwCollections, err := agwplugins.NewAgwCollections(
		krtOpts,
		s.APIClient,
		s.ControllerName,
		*s.GlobalSettings,
		// control plane system namespace (default is agentgateway-system)
		namespaces.GetPodNamespace(),
		s.APIClient.ClusterID().String(),
	)
	if err != nil {
		slog.Error("error creating agw collections", "error", err)
		return err
	}

	policySelector := policyselection.NewSelector(agwCollections.AgentgatewayPolicies, agwCollections.BackendTLSPolicies)
	resolver := remotehttp.NewResolver(remotehttp.Inputs{
		ConfigMaps:     agwCollections.ConfigMaps,
		Services:       agwCollections.Services,
		Backends:       agwCollections.Backends,
		PolicySelector: policySelector,
	})
	persistedJWKS := s.PersistedJWKS
	if persistedJWKS == nil {
		persistedJWKS = jwks.NewPersistedEntries(s.APIClient, krtOpts, jwks.DefaultJwksStorePrefix, namespaces.GetPodNamespace())
	}
	jwksLookup := jwks.NewLookup(persistedJWKS, jwks.NewResolver(resolver))

	for _, mgrCfgFunc := range s.ExtraManagerConfig {
		err := mgrCfgFunc(mgr)
		if err != nil {
			return err
		}
	}

	runnablesRegistry := sets.New[string]()
	for _, runnable := range s.ExtraRunnables {
		enabled, r := runnable(ctx, agwCollections, s.GlobalSettings)
		if !enabled {
			continue
		}
		if named, ok := r.(common.NamedRunnable); ok {
			runnablesRegistry.Insert(named.RunnableName())
		}
		if err := mgr.Add(r); err != nil {
			return fmt.Errorf("error adding extra Runnable to manager: %w", err)
		}
	}

	// build jwks store if it doesn't exist
	if !runnablesRegistry.Contains(jwks.RunnableName) {
		if err := buildJwksStore(ctx, mgr, s.APIClient, agwCollections, persistedJWKS, resolver); err != nil {
			return fmt.Errorf("error creating jwks store %w", err)
		}
	}

	agw, err := s.buildSyncer(ctx, mgr, setupOpts, agwCollections, resolver, jwksLookup)
	if err != nil {
		return err
	}

	if s.XDSListener != nil && agw != nil {
		if s.GlobalSettings.XdsMode == apisettings.XdsModeEither {
			xdsMux := cmux.New(s.XDSListener)
			tlsListener := xdsMux.Match(cmux.TLS())
			plaintextListener := xdsMux.Match(cmux.Any())
			runXDSServer(ctx, tlsListener, authenticators, s.GlobalSettings.XdsAuth, certWatcher, agw.NackPublisher, agw.Registrations...)
			runXDSServer(ctx, plaintextListener, authenticators, s.GlobalSettings.XdsAuth, nil, agw.NackPublisher, agw.Registrations...)
			context.AfterFunc(ctx, xdsMux.Close)
			go func() {
				if err := xdsMux.Serve(); err != nil && err != cmux.ErrListenerClosed && err != cmux.ErrServerClosed {
					slog.Error("xDS listener mux stopped", "error", err)
				}
			}()
		} else if s.GlobalSettings.IsXdsTLSEnabled() {
			runXDSServer(ctx, s.XDSListener, authenticators, s.GlobalSettings.XdsAuth, certWatcher, agw.NackPublisher, agw.Registrations...)
		} else if s.GlobalSettings.IsXdsPlaintextEnabled() {
			runXDSServer(ctx, s.XDSListener, authenticators, s.GlobalSettings.XdsAuth, nil, agw.NackPublisher, agw.Registrations...)
		}
	}

	slog.Info("starting admin server")
	go admin.RunAdminServer(ctx, setupOpts)

	slog.Info("starting manager")
	return mgr.Start(ctx)
}

func (s *setup) xdsTLSHosts() []string {
	var hosts []string
	if s.GlobalSettings.XdsServiceHost != "" {
		hosts = []string{s.GlobalSettings.XdsServiceHost}
	} else {
		namespace := namespaces.GetPodNamespace()
		serviceName := s.GlobalSettings.XdsServiceName
		hosts = []string{
			serviceName + "." + namespace,
			serviceName + "." + namespace + ".svc",
			kubeutils.ServiceFQDN(metav1.ObjectMeta{Name: serviceName, Namespace: namespace}),
		}
	}
	for _, host := range s.GlobalSettings.AdditionalXdsTLSHosts {
		host = strings.TrimSpace(host)
		if host != "" {
			hosts = append(hosts, host)
		}
	}
	return hosts
}

func newXDSListener(ip string, port uint32) (net.Listener, error) {
	bindAddr := net.TCPAddr{IP: net.ParseIP(ip), Port: int(port)}
	return net.Listen(bindAddr.Network(), bindAddr.String())
}

func (s *setup) buildSyncer(
	ctx context.Context,
	mgr manager.Manager,
	setupOpts *controller.SetupOpts,
	agwCollections *agwplugins.AgwCollections,
	resolver remotehttp.Resolver,
	jwksLookup jwks.Lookup,
) (*syncer.Syncer, error) {
	slog.Info("creating krt collections")
	krtOpts := krtutil.NewKrtOptions(ctx.Done(), setupOpts.KrtDebugger)

	gatewayClassInfos := controller.GetDefaultClassInfo(
		setupOpts.GlobalSettings,
		s.AgentgatewayClassName,
		s.ControllerName,
		s.AdditionalGatewayClasses,
	)

	slog.Info("initializing controller")
	c, err := controller.NewControllerBuilder(ctx, controller.StartConfig{
		Manager:                        mgr,
		AgwControllerName:              s.ControllerName,
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
		AgwCollections:                 agwCollections,
		Resolver:                       resolver,
		JWKSLookup:                     jwksLookup,
		ExtraAgwResourceStatusHandlers: s.ExtraStatusHandlers,
		GatewayControllerExtension:     s.GatewayControllerExtension,
		AgentgatewaySyncerOptions:      s.AgentGatewaySyncerOptions,
	})
	if err != nil {
		slog.Error("failed initializing controller: ", "error", err)
		return nil, err
	}

	slog.Info("waiting for cache sync")

	agwSyncer, err := c.Build()
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

func initDiscoveryNSFilter(
	ctx context.Context,
	cli apiclient.Client,
	discoveryNamespaceSetting string,
) error {
	// NOTE: Do not apply an ObjectFilter to namespaces as the discovery namespace ObjectFilter for other clients
	// requires all namespaces to be watched
	nsClient := kclient.New[*corev1.Namespace](cli) //nolint:forbidigo // okay to use non-filtered client
	// Initialize discovery namespace filter
	discoveryNamespacesFilter, err := pluginsdkcol.NewDiscoveryNamespacesFilter(nsClient, discoveryNamespaceSetting, ctx.Done())
	if err != nil {
		return fmt.Errorf("error creating discovery namespace filter: %w", err)
	}
	kube.SetObjectFilter(cli.Core(), discoveryNamespacesFilter)
	return nil
}

func buildJwksStore(
	ctx context.Context,
	mgr manager.Manager,
	apiClient apiclient.Client,
	agwCollections *agwplugins.AgwCollections,
	persistedJWKS *jwks.PersistedEntries,
	resolver remotehttp.Resolver,
) error {
	jwksCollections := jwks.NewCollections(jwks.CollectionInputs{
		AgentgatewayPolicies: agwCollections.AgentgatewayPolicies,
		Backends:             agwCollections.Backends,
		Resolver:             jwks.NewResolver(resolver),
		KrtOpts:              agwCollections.KrtOpts,
	})

	jwksStore := jwks.NewStore(jwksCollections.SharedRequests, persistedJWKS, jwks.DefaultJwksStorePrefix)
	if err := mgr.Add(jwksStore); err != nil {
		return err
	}

	jwksStoreCMCtrl := jwks.NewConfigMapController(apiClient, jwks.DefaultJwksStorePrefix, namespaces.GetPodNamespace(), jwksStore, persistedJWKS)
	jwksStoreCMCtrl.Init(ctx)
	if err := mgr.Add(jwksStoreCMCtrl); err != nil {
		return err
	}

	return nil
}

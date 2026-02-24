package controller

import (
	"context"
	"crypto/tls"
	"errors"
	"fmt"
	"maps"
	"math"
	"slices"

	"istio.io/istio/pkg/config/schema/gvk"
	"istio.io/istio/pkg/config/schema/gvr"
	"istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/kclient"
	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/labels"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/apimachinery/pkg/util/sets"
	"k8s.io/client-go/tools/cache"
	utilretry "k8s.io/client-go/util/retry"
	"sigs.k8s.io/controller-runtime/pkg/certwatcher"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/manager"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/deployer"
	internaldeployer "github.com/agentgateway/agentgateway/controller/pkg/kgateway/deployer"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk"
	"github.com/agentgateway/agentgateway/controller/pkg/reports"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
)

var logger = logging.New("gateway-controller")

var _ manager.LeaderElectionRunnable = (*gatewayReconciler)(nil)

type gatewayReconciler struct {
	deployer          *deployer.Deployer
	gwParams          *internaldeployer.GatewayParameters
	scheme            *runtime.Scheme
	agwControllerName string

	gwClient         kclient.Client[*gwv1.Gateway]
	gwClassClient    kclient.Client[*gwv1.GatewayClass]
	agwParamClient   kclient.Client[*agentgateway.AgentgatewayParameters]
	nsClient         kclient.Client[*corev1.Namespace]
	svcClient        kclient.Client[*corev1.Service]
	deploymentClient kclient.Client[*appsv1.Deployment]
	svcAccountClient kclient.Client[*corev1.ServiceAccount]
	configMapClient  kclient.Client[*corev1.ConfigMap]

	controllerExtension pluginsdk.GatewayControllerExtension

	queue controllers.Queue
}

func NewGatewayReconciler(
	cfg GatewayConfig,
	deployer *deployer.Deployer,
	gwParams *internaldeployer.GatewayParameters,
	controllerExtension pluginsdk.GatewayControllerExtension,
) *gatewayReconciler {
	filter := kclient.Filter{ObjectFilter: cfg.Client.ObjectFilter()}
	r := &gatewayReconciler{
		deployer:            deployer,
		gwParams:            gwParams,
		scheme:              cfg.Mgr.GetScheme(),
		agwControllerName:   cfg.AgwControllerName,
		controllerExtension: controllerExtension,

		gwClient:         kclient.NewFilteredDelayed[*gwv1.Gateway](cfg.Client, gvr.KubernetesGateway, filter),
		gwClassClient:    kclient.NewFilteredDelayed[*gwv1.GatewayClass](cfg.Client, gvr.GatewayClass, filter),
		nsClient:         kclient.NewFiltered[*corev1.Namespace](cfg.Client, filter),
		svcClient:        kclient.NewFiltered[*corev1.Service](cfg.Client, filter),
		deploymentClient: kclient.NewFiltered[*appsv1.Deployment](cfg.Client, filter),
		svcAccountClient: kclient.NewFiltered[*corev1.ServiceAccount](cfg.Client, filter),
		configMapClient:  kclient.NewFiltered[*corev1.ConfigMap](cfg.Client, filter),
	}

	// Reuse the parameter clients from the deployer to avoid duplicate watches
	// These clients are only created when the respective controllers are enabled
	r.agwParamClient = gwParams.GetAgentgatewayParametersClient()

	r.queue = controllers.NewQueue("GatewayController", controllers.WithReconciler(r.Reconcile), controllers.WithMaxAttempts(math.MaxInt), controllers.WithRateLimiter(rateLimiter))

	// Gateway event handler
	r.gwClient.AddEventHandler(
		controllers.FromEventHandler(func(o controllers.Event) {
			switch o.Event {
			case controllers.EventAdd:
				logger.Debug("reconciling Gateway due to add event", "ref", kubeutils.NamespacedNameFrom(o.New))
				r.queue.AddObject(o.New)
			case controllers.EventUpdate:
				if o.New.GetGeneration() != o.Old.GetGeneration() {
					logger.Debug("reconciling Gateway due to generation change", "ref", kubeutils.NamespacedNameFrom(o.New))
					r.queue.AddObject(o.New)
					break
				}
				// TODO: check if we want to reconcile when labels change
				if !maps.Equal(o.New.GetAnnotations(), o.Old.GetAnnotations()) {
					logger.Debug("reconciling Gateway due to annotation change", "ref", kubeutils.NamespacedNameFrom(o.New))
					r.queue.AddObject(o.New)
					break
				}
				logger.Debug("skip reconciling Gateway with no relevant changes", "ref", kubeutils.NamespacedNameFrom(o.New))
			case controllers.EventDelete:
				logger.Debug("reconciling Gateway due to delete event", "ref", kubeutils.NamespacedNameFrom(o.Old))
				r.queue.AddObject(o.Old)
			}
		}))

	// GatewayClass event handler
	r.gwClassClient.AddEventHandler(controllers.ObjectHandler(func(o controllers.Object) {
		gwClass, ok := o.(*gwv1.GatewayClass)
		if !ok {
			// should not happen
			logger.Error("got unexpected type instead of GatewayClass", "kind", o.GetObjectKind())
			return
		}
		// If this GatewayClass is not ours, ignore it
		if gwClass.Spec.ControllerName != gwv1.GatewayController(r.agwControllerName) {
			return
		}
		for _, g := range r.gwClient.List(metav1.NamespaceAll, labels.Everything()) {
			if string(g.Spec.GatewayClassName) == o.GetName() {
				logger.Debug("reconciling Gateway due to GatewayClass change",
					"ref", kubeutils.NamespacedNameFrom(g), "gwclass", kubeutils.NamespacedNameFrom(gwClass))
				r.queue.AddObject(g)
			}
		}
	}))

	// GatewayParameters event handler
	gatewaysByParamsRef := kclient.CreateIndex(r.gwClient, "parametersRef", func(o *gwv1.Gateway) []types.NamespacedName {
		p := fetchGatewaysByParametersRef(o)
		if p == nil {
			return nil
		}
		return []types.NamespacedName{*p}
	})
	gatewaysByClass := kclient.CreateIndex(r.gwClient, "gatewayClass", func(o *gwv1.Gateway) []types.NamespacedName {
		p := fetchGatewaysByGatewayClass(o)
		return []types.NamespacedName{p}
	})
	// AgentgatewayParameters event handler (same logic as GatewayParameters)
	// agwParamEventHandler is a handler that reconciles Gateways based on AgentgatewayParameters changes
	agwParamEventHandler := controllers.ObjectHandler(func(o controllers.Object) {
		agwpName := o.GetName()
		agwpNamespace := o.GetNamespace()

		// 1. Look up Gateways directly using this AgentgatewayParameters object (via spec.infrastructure.parametersRef)
		gateways := gatewaysByParamsRef.Lookup(types.NamespacedName{Namespace: agwpNamespace, Name: agwpName})
		for _, gw := range gateways {
			logger.Debug("reconciling Gateway due to AgentgatewayParameters change",
				"ref", kubeutils.NamespacedNameFrom(gw), "agwparam", types.NamespacedName{Namespace: agwpNamespace, Name: agwpName})
			r.queue.AddObject(gw)
		}

		// 2. Look up GatewayClasses using this AgentgatewayParameters object (via spec.parametersRef)
		gwClasses := r.gwClassClient.List(metav1.NamespaceAll, labels.Everything())
		// For each GatewayClass that references this parameter, find all Gateways using that class
		for _, gc := range gwClasses {
			// Only process GatewayClasses managed by our controllers
			if gc.Spec.ControllerName != gwv1.GatewayController(r.agwControllerName) {
				continue
			}
			if gc.Spec.ParametersRef != nil &&
				gc.Spec.ParametersRef.Name == agwpName &&
				gc.Spec.ParametersRef.Namespace != nil && string(*gc.Spec.ParametersRef.Namespace) == agwpNamespace {
				// This GatewayClass references our AgentgatewayParameters, find all Gateways using this class
				gateways := gatewaysByClass.Lookup(types.NamespacedName{Name: gc.Name})
				for _, gw := range gateways {
					logger.Debug("reconciling Gateway due to AgentgatewayParameters change via GatewayClass",
						"ref", kubeutils.NamespacedNameFrom(gw), "agwparam", types.NamespacedName{Namespace: agwpNamespace, Name: agwpName},
						"gwclass", gc.Name)
					r.queue.AddObject(gw)
				}
			}
		}
	})
	if r.agwParamClient != nil {
		r.agwParamClient.AddEventHandler(agwParamEventHandler)
	}
	if controllerExtension != nil {
		controllerExtension.Register(r.queue, agwParamEventHandler)
	}

	// Add a handler to reconcile the parent Gateway when child objects (Deployment, Service, etc.)
	parentHandler := controllers.ObjectHandler(controllers.EnqueueForParentHandler(r.queue, gvk.KubernetesGateway))
	r.deploymentClient.AddEventHandler(parentHandler)
	r.svcAccountClient.AddEventHandler(parentHandler)
	r.svcClient.AddEventHandler(parentHandler)
	r.configMapClient.AddEventHandler(parentHandler)

	// Add a handler to reconcile the Gateways when the xDS TLS certificate changes
	r.setupTLSCertificateWatch(cfg.CertWatcher)

	return r
}

// NeedLeaderElection returns true to ensure that the Gateway reconciler runs only on the leader
func (r *gatewayReconciler) NeedLeaderElection() bool {
	return true
}

// Start starts the Gateway reconciler and blocks until the stop channel is closed.
func (r *gatewayReconciler) Start(ctx context.Context) error {
	// Add all clients handlers on gatewayReconciler
	hasSynced := []cache.InformerSynced{
		r.gwClient.HasSynced,
		r.gwClassClient.HasSynced,
		r.nsClient.HasSynced,
		r.deploymentClient.HasSynced,
		r.svcAccountClient.HasSynced,
		r.svcClient.HasSynced,
		r.configMapClient.HasSynced,
	}
	// Add GatewayParameters cache sync handlers (includes both gwParamClient and agwParamClient)
	hasSynced = append(hasSynced, r.gwParams.GetCacheSyncHandlers()...)

	// Wait for all caches to sync
	kube.WaitForCacheSync("GatewayController", ctx.Done(), hasSynced...)
	if r.controllerExtension != nil {
		r.controllerExtension.Start(ctx)
	}
	r.queue.Run(ctx.Done())

	// Shutdown all the clients
	clients := []controllers.Shutdowner{
		r.gwClient,
		r.gwClassClient,
		r.nsClient,
		r.deploymentClient,
		r.svcAccountClient,
		r.svcClient,
		r.configMapClient,
	}
	if r.agwParamClient != nil {
		clients = append(clients, r.agwParamClient)
	}
	controllers.ShutdownAll(clients...)
	if r.controllerExtension != nil {
		r.controllerExtension.Stop()
	}
	return nil
}

func (r *gatewayReconciler) Reconcile(req types.NamespacedName) (rErr error) {
	finishMetrics := collectReconciliationMetrics("gateway", req)
	defer func() {
		finishMetrics(rErr)
	}()

	gw := r.gwClient.Get(req.Name, req.Namespace)
	if gw == nil || gw.GetDeletionTimestamp() != nil {
		// ignore the event if the Gateway is not found. A subsequent event should handle this if needed
		logger.Debug("gateway not found, skipping reconciliation", "ref", req)
		return nil
	}

	// make sure we're the right controller for this
	gwc := r.gwClassClient.Get(string(gw.Spec.GatewayClassName), "")
	if gwc == nil {
		return fmt.Errorf("gatewayclass %s not found for gateway %s", gw.Spec.GatewayClassName, req)
	}

	// Only reconcile Gateways for enabled controllers
	isAgwGateway := gwc.Spec.ControllerName == gwv1.GatewayController(r.agwControllerName)
	if !isAgwGateway {
		// Not our GatewayClass at all
		return nil
	}

	logger.Info("reconciling Gateway", "ref", req)
	ctx := context.Background()
	objs, err := r.deployer.GetObjsToDeploy(ctx, gw)
	if err != nil {
		if errors.Is(err, internaldeployer.ErrNoValidPorts) {
			// status is reported from translator, so return normally
			return err
		}
		// if we fail to either reference a valid GatewayParameters or
		// the GatewayParameters configuration leads to issues building the
		// objects, we want to set the status to InvalidParameters.
		condition := metav1.Condition{
			Type:               string(gwv1.GatewayConditionAccepted),
			Status:             metav1.ConditionFalse,
			ObservedGeneration: gw.Generation,
			Reason:             string(gwv1.GatewayReasonInvalidParameters),
			Message:            err.Error(),
		}
		if statusErr := r.updateGatewayStatusWithRetry(ctx, gw, condition); statusErr != nil {
			return fmt.Errorf("failed to update status for Gateway %s: %w", req, statusErr)
		}
		return err
	} else if existing := meta.FindStatusCondition(gw.Status.Conditions, string(gwv1.GatewayConditionAccepted)); existing != nil &&
		existing.Status == metav1.ConditionFalse &&
		existing.Reason == string(gwv1.GatewayReasonInvalidParameters) {
		// set the status Accepted=true if it had been set to false due to InvalidParameters
		condition := metav1.Condition{
			Type:               string(gwv1.GatewayConditionAccepted),
			Status:             metav1.ConditionTrue,
			ObservedGeneration: gw.Generation,
			Reason:             string(gwv1.GatewayReasonAccepted),
			Message:            reports.GatewayAcceptedMessage,
		}
		if statusErr := r.updateGatewayStatusWithRetry(ctx, gw, condition); statusErr != nil {
			return fmt.Errorf("failed to update status for Gateway %s: %w", req, statusErr)
		}
	}
	objs = r.deployer.SetNamespaceAndOwnerWithGVK(gw, wellknown.GatewayGVK, objs)
	err = r.deployer.DeployObjsWithSource(ctx, objs, gw)
	if err != nil {
		return err
	}

	// find the name/ns of the service we own so we can grab addresses
	// from it for status
	var generatedSvc *metav1.ObjectMeta
	for _, obj := range objs {
		if svc, ok := obj.(*corev1.Service); ok {
			generatedSvc = &svc.ObjectMeta
			break
		}
	}
	// update status (whether we generated a service or not, for unmanaged)
	err = r.updateStatus(ctx, gw, generatedSvc)
	if err != nil {
		return fmt.Errorf("error updating status for Gateway %s: %w", req, err)
	}

	return nil
}

func (r *gatewayReconciler) updateStatus(ctx context.Context, gw *gwv1.Gateway, svcMeta *metav1.ObjectMeta) error {
	var svc *corev1.Service
	if svcMeta != nil {
		svcnns := client.ObjectKey{
			Namespace: svcMeta.Namespace,
			Name:      svcMeta.Name,
		}

		svc = r.svcClient.Get(svcnns.Name, svcnns.Namespace)
		if svc == nil {
			return fmt.Errorf("Service %s not found for Gateway %s", svcnns, kubeutils.NamespacedNameFrom(gw))
		}

		// make sure we own this service
		controller := metav1.GetControllerOf(svc)
		if controller == nil {
			return nil
		}

		if gw.UID != controller.UID {
			return nil
		}
	}

	// update gateway addresses in the status
	desiredAddresses := getDesiredAddresses(gw, svc)
	return updateGatewayAddresses(ctx, r.gwClient, client.ObjectKeyFromObject(gw), desiredAddresses)
}

func getDesiredAddresses(gw *gwv1.Gateway, svc *corev1.Service) []gwv1.GatewayStatusAddress {
	var ret []gwv1.GatewayStatusAddress
	seen := sets.New[gwv1.GatewayStatusAddress]()

	if svc != nil && svc.Spec.Type == corev1.ServiceTypeLoadBalancer {
		if len(svc.Status.LoadBalancer.Ingress) == 0 {
			return nil
		}
		for _, ing := range svc.Status.LoadBalancer.Ingress {
			if addr, ok := convertIngressAddr(ing); ok {
				seen.Insert(addr)
				ret = append(ret, addr)
			}
		}

		return ret
	} else if svc != nil {
		t := gwv1.IPAddressType
		if len(svc.Spec.ClusterIPs) != 0 {
			for _, ip := range svc.Spec.ClusterIPs {
				ret = append(ret, gwv1.GatewayStatusAddress{
					Type:  &t,
					Value: ip,
				})
			}
		} else if svc.Spec.ClusterIP != "" {
			ret = append(ret, gwv1.GatewayStatusAddress{
				Type:  &t,
				Value: svc.Spec.ClusterIP,
			})
		}
	}

	for _, specAddr := range gw.Spec.Addresses {
		addr := gwv1.GatewayStatusAddress{
			Type:  specAddr.Type,
			Value: specAddr.Value,
		}
		if !seen.Has(addr) {
			ret = append(ret, addr)
		}
	}

	return ret
}

// updateGatewayStatusWithRetryFunc updates a Gateway's status with retry logic.
// The updateFunc receives the latest Gateway and should modify its status as needed.
// If updateFunc returns false, the update is skipped (no changes needed).
func updateGatewayStatusWithRetryFunc(
	_ context.Context,
	cli kclient.Client[*gwv1.Gateway],
	gwNN types.NamespacedName,
	updateFunc func(*gwv1.Gateway) (gwv1.GatewayStatus, bool),
) error {
	err := utilretry.RetryOnConflict(utilretry.DefaultRetry, func() error {
		gw := cli.Get(gwNN.Name, gwNN.Namespace)
		if gw == nil {
			// If the Gateway no longer exists, there's nothing to update.
			logger.Warn("gateway not found during status update", "ref", gwNN)
			return nil
		}
		status, needsUpdate := updateFunc(gw)
		if !needsUpdate {
			return nil
		}
		_, err := cli.UpdateStatus(&gwv1.Gateway{
			ObjectMeta: pluginsdk.CloneObjectMetaForStatus(gw.ObjectMeta),
			Status:     status,
		})
		return err
	})
	if err != nil {
		return fmt.Errorf("failed to update gateway status: %w", err)
	}

	return nil
}

// updateGatewayAddresses updates the addresses of a Gateway resource.
func updateGatewayAddresses(
	ctx context.Context,
	cli kclient.Client[*gwv1.Gateway],
	gwNN types.NamespacedName,
	desired []gwv1.GatewayStatusAddress,
) error {
	return updateGatewayStatusWithRetryFunc(
		ctx,
		cli,
		gwNN,
		func(gw *gwv1.Gateway) (gwv1.GatewayStatus, bool) {
			// Check if an update is needed
			if slices.Equal(desired, gw.Status.Addresses) {
				return gw.Status, false
			}
			newStatus := gw.Status.DeepCopy()
			newStatus.Addresses = desired
			return *newStatus, true
		},
	)
}

// updateGatewayStatusWithRetry attempts to update the Gateway status with retry logic
// to handle transient failures when updating the status subresource
func (r *gatewayReconciler) updateGatewayStatusWithRetry(ctx context.Context, gw *gwv1.Gateway, condition metav1.Condition) error {
	return updateGatewayStatusWithRetryFunc(
		ctx,
		r.gwClient,
		client.ObjectKeyFromObject(gw),
		func(latest *gwv1.Gateway) (gwv1.GatewayStatus, bool) {
			newStatus := latest.Status.DeepCopy()
			meta.SetStatusCondition(&newStatus.Conditions, condition)
			return *newStatus, true
		},
	)
}

// setupTLSCertificateWatch configures a watch for xDS TLS certificate changes.
// When certificates are rotated, all Gateways managed by this controller will be reconciled
// to update the proxy CA certificates.
func (r *gatewayReconciler) setupTLSCertificateWatch(certWatcher *certwatcher.CertWatcher) {
	if certWatcher == nil {
		return
	}

	// Register callback to send events when certificate changes
	certWatcher.RegisterCallback(func(_ tls.Certificate) {
		logger.Info("xDS TLS certificate changed, triggering Gateway reconciliation")
		gateways := r.gwClient.List(metav1.NamespaceAll, labels.Everything())
		for _, gw := range gateways {
			gwClass := r.gwClassClient.Get(string(gw.Spec.GatewayClassName), "")
			ref := kubeutils.NamespacedNameFrom(gw)
			if gwClass == nil {
				logger.Error("error getting GatewayClass for Gateway during certificate change", "ref", ref)
				continue
			}
			if gwClass.Spec.ControllerName == gwv1.GatewayController(r.agwControllerName) {
				logger.Debug("enqueueing Gateway for reconciliation due to certificate change", "ref", ref)
				r.queue.AddObject(gw)
			}
		}
	})
}

func convertIngressAddr(ing corev1.LoadBalancerIngress) (gwv1.GatewayStatusAddress, bool) {
	if ing.Hostname != "" {
		t := gwv1.HostnameAddressType
		return gwv1.GatewayStatusAddress{
			Type:  &t,
			Value: ing.Hostname,
		}, true
	}
	if ing.IP != "" {
		t := gwv1.IPAddressType
		return gwv1.GatewayStatusAddress{
			Type:  &t,
			Value: ing.IP,
		}, true
	}
	return gwv1.GatewayStatusAddress{}, false
}

func fetchGatewaysByParametersRef(
	gw *gwv1.Gateway,
) *types.NamespacedName {
	if gw.Spec.Infrastructure != nil && gw.Spec.Infrastructure.ParametersRef != nil {
		pr := gw.Spec.Infrastructure.ParametersRef
		return &types.NamespacedName{
			Namespace: gw.Namespace,
			Name:      pr.Name,
		}
	}
	return nil
}

func fetchGatewaysByGatewayClass(gw *gwv1.Gateway) types.NamespacedName {
	return types.NamespacedName{
		Name: string(gw.Spec.GatewayClassName),
	}
}

package controller

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"math"

	"istio.io/istio/pkg/config/schema/gvr"
	"istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/kclient"
	"k8s.io/apimachinery/pkg/api/meta"
	apivalidation "k8s.io/apimachinery/pkg/api/validation"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/apimachinery/pkg/util/sets"
	"k8s.io/utils/ptr"
	"sigs.k8s.io/controller-runtime/pkg/manager"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/apiclient"
	"github.com/kgateway-dev/kgateway/v2/pkg/deployer"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
	"github.com/kgateway-dev/kgateway/v2/pkg/reports"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils"
)

var _ manager.LeaderElectionRunnable = (*gatewayClassReconciler)(nil)

// emptyGatewayClass is a sentinel value for an empty GatewayClass used to create
// the default GatewayClass
var emptyGatewayClass = types.NamespacedName{}

type gatewayClassReconciler struct {
	classInfo             map[string]*deployer.GatewayClassInfo
	defaultControllerName string
	gwClassClient         kclient.Client[*gwv1.GatewayClass]
	client                apiclient.Client
	queue                 controllers.Queue
}

func newGatewayClassReconciler(
	cfg GatewayConfig,
	classInfo map[string]*deployer.GatewayClassInfo,
) *gatewayClassReconciler {
	filter := kclient.Filter{ObjectFilter: cfg.Client.ObjectFilter()}
	r := &gatewayClassReconciler{
		defaultControllerName: cfg.AgwControllerName,
		classInfo:             classInfo,
		gwClassClient:         kclient.NewFilteredDelayed[*gwv1.GatewayClass](cfg.Client, gvr.GatewayClass, filter),
		client:                cfg.Client,
	}
	r.queue = controllers.NewQueue("GatewayClassController", controllers.WithReconciler(r.reconcile), controllers.WithMaxAttempts(math.MaxInt), controllers.WithRateLimiter(rateLimiter))
	ourControllers := sets.New(cfg.AgwControllerName)

	r.gwClassClient.AddEventHandler(
		controllers.FromEventHandler(func(o controllers.Event) {
			switch o.Event {
			case controllers.EventAdd:
				logger.Debug("reconciling GatewayClass due to add event", "ref", kubeutils.NamespacedNameFrom(o.New))
				if isOurGatewayClass(o.New.(*gwv1.GatewayClass), ourControllers) {
					r.queue.AddObject(o.New)
				}
			case controllers.EventUpdate:
				if o.New.GetGeneration() != o.Old.GetGeneration() && isOurGatewayClass(o.New.(*gwv1.GatewayClass), ourControllers) {
					logger.Debug("reconciling GatewayClass due to generation change", "ref", kubeutils.NamespacedNameFrom(o.New))
					r.queue.AddObject(o.New)
					return
				}
				logger.Debug("skip reconciling GatewayClass with no relevant changes", "ref", kubeutils.NamespacedNameFrom(o.New))
			case controllers.EventDelete:
				logger.Debug("reconciling GatewayClass due to delete event", "ref", kubeutils.NamespacedNameFrom(o.Old))
				if isOurGatewayClass(o.Old.(*gwv1.GatewayClass), ourControllers) {
					r.queue.AddObject(o.Old)
				}
			}
		}))

	return r
}

// NeedLeaderElection returns true to ensure that the GatewayClass runs only on the leader
func (r *gatewayClassReconciler) NeedLeaderElection() bool {
	return true
}

// Start starts the GatewayClass reconciler and blocks until the stop channel is closed.
func (r *gatewayClassReconciler) Start(ctx context.Context) error {
	// Seed the queue with an initial event to ensure default GatewayClass creation
	r.queue.Add(emptyGatewayClass)

	// Wait for all caches to sync
	kube.WaitForCacheSync("GatewayClassController", ctx.Done(), r.gwClassClient.HasSynced)
	r.queue.Run(ctx.Done())

	// Shutdown all the clients
	controllers.ShutdownAll(r.gwClassClient)
	return nil
}

func (r *gatewayClassReconciler) reconcile(req types.NamespacedName) (rErr error) {
	finishMetrics := collectReconciliationMetrics("gatewayclass", req)
	defer func() {
		finishMetrics(rErr)
	}()

	if req == emptyGatewayClass {
		// This is the initial reconciliation event to ensure the default GatewayClasses are created/updated
		return r.reconcileGatewayClasses()
	}
	// Reconcile spec if this GatewayClass is managed by us
	if info, ok := r.classInfo[req.Name]; ok {
		if err := r.reconcileGatewayClass(req.Name, info); err != nil {
			return fmt.Errorf("error reconciling GatewayClass spec: %w", err)
		}
	}

	gwClass := r.gwClassClient.Get(req.Name, req.Namespace)
	if gwClass == nil || gwClass.GetDeletionTimestamp() != nil {
		logger.Debug("gatewayclass not found, skipping status update", "ref", req)
		return nil
	}

	// Update status
	status := gwClass.Status
	meta.SetStatusCondition(&status.Conditions, metav1.Condition{
		Type:               string(gwv1.GatewayClassConditionStatusAccepted),
		Status:             metav1.ConditionTrue,
		Reason:             string(gwv1.GatewayClassReasonAccepted),
		ObservedGeneration: gwClass.Generation,
		Message:            reports.GatewayClassAcceptedMessage,
	})
	if i, ok := r.classInfo[gwClass.Name]; ok {
		status.SupportedFeatures = i.SupportedFeatures
	}

	_, err := r.gwClassClient.UpdateStatus(&gwv1.GatewayClass{
		ObjectMeta: CloneObjectMetaForStatus(gwClass.ObjectMeta),
		Status:     status,
	})
	if err != nil {
		return fmt.Errorf("error updating GatewayClass status: %w", err)
	}

	return nil
}

func (r *gatewayClassReconciler) reconcileGatewayClasses() error {
	var errs []error
	for name, info := range r.classInfo {
		// Validate the GatewayClass name using Kubernetes object name validation
		if validationErrs := apivalidation.NameIsDNSSubdomain(name, false); len(validationErrs) > 0 {
			errs = append(errs, fmt.Errorf("invalid GatewayClass name %q: %v", name, validationErrs))
			continue
		}
		if err := r.reconcileGatewayClass(name, info); err != nil {
			errs = append(errs, err)
		}
	}
	return errors.Join(errs...)
}

func (r *gatewayClassReconciler) reconcileGatewayClass(name string, info *deployer.GatewayClassInfo) error {
	// Build desired GatewayClass with only fields we want to manage via SSA
	desired := r.buildDesiredGatewayClass(name, info)

	// Always apply using SSA - SSA is idempotent and will only update if needed
	logger.Debug("applying GatewayClass via SSA", "name", name)
	if err := r.applyGatewayClass(desired, r.getControllerName(name)); err != nil {
		return fmt.Errorf("error applying GatewayClass %s: %w", name, err)
	}
	return nil
}

func (r *gatewayClassReconciler) buildDesiredGatewayClass(name string, info *deployer.GatewayClassInfo) *gwv1.GatewayClass {
	gwc := &gwv1.GatewayClass{
		TypeMeta: metav1.TypeMeta{
			APIVersion: wellknown.GatewayClassGVK.GroupVersion().String(),
			Kind:       wellknown.GatewayClassKind,
		},
		ObjectMeta: metav1.ObjectMeta{
			Name:        name,
			Annotations: info.Annotations,
			Labels:      info.Labels,
		},
		Spec: gwv1.GatewayClassSpec{
			ControllerName: gwv1.GatewayController(r.getControllerName(name)),
			ParametersRef:  info.ParametersRef,
		},
	}
	if info.Description != "" {
		gwc.Spec.Description = ptr.To(info.Description)
	}
	return gwc
}

func (r *gatewayClassReconciler) applyGatewayClass(gwc *gwv1.GatewayClass, controllerName string) error {
	gvr := gvr.GatewayClass
	c := r.client.Dynamic().Resource(gvr).Namespace(metav1.NamespaceNone)

	// Convert to unstructured for SSA
	u, err := kubeutils.ToUnstructured(gwc)
	if err != nil {
		return fmt.Errorf("error converting GatewayClass to unstructured: %w", err)
	}

	js, err := json.Marshal(u.Object)
	if err != nil {
		return fmt.Errorf("error marshaling GatewayClass: %w", err)
	}

	_, err = c.Patch(context.Background(), gwc.Name, types.ApplyPatchType, js, metav1.PatchOptions{
		Force:        ptr.To(true),
		FieldManager: controllerName,
	})
	return err
}

func isOurGatewayClass(gwc *gwv1.GatewayClass, ourControllers sets.Set[string]) bool {
	return ourControllers.Has(string(gwc.Spec.ControllerName))
}

func (r *gatewayClassReconciler) getControllerName(gwc string) string {
	controllerName := r.defaultControllerName
	info, ok := r.classInfo[gwc]
	if ok && info.ControllerName != "" {
		controllerName = info.ControllerName
	}
	return controllerName
}

func CloneObjectMetaForStatus(m metav1.ObjectMeta) metav1.ObjectMeta {
	return metav1.ObjectMeta{
		Name:            m.GetName(),
		Namespace:       m.GetNamespace(),
		ResourceVersion: m.GetResourceVersion(),
	}
}

package nack

import (
	"time"

	"istio.io/istio/pkg/config/schema/gvr"
	"istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/kube/kclient"
	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	typedcorev1 "k8s.io/client-go/kubernetes/typed/core/v1"
	"k8s.io/client-go/tools/record"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
	"github.com/agentgateway/agentgateway/controller/pkg/schemes"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

var log = logging.New("nack/publisher")

// Event reasons for Kubernetes Events created by agentgateway NACK detection
const (
	ReasonNack = "AgentGatewayNackError"
)

// NackEvent represents a NACK received from an agentgateway gateway
type NackEvent struct {
	Gateway   types.NamespacedName
	TypeUrl   string
	ErrorMsg  string
	Timestamp time.Time
}

// Publisher converts NACK events from the agentgateway xDS server into Kubernetes Events.
type Publisher struct {
	eventRecorder      record.EventRecorder
	gatewayClient      kclient.Client[*gwv1.Gateway]
	gatewayClassClient kclient.Client[*gwv1.GatewayClass]
	agwParamClient     kclient.Client[*agentgateway.AgentgatewayParameters]
	deploymentClient   kclient.Client[*appsv1.Deployment]
	daemonSetClient    kclient.Client[*appsv1.DaemonSet]
	HasSynced          func() bool
}

// NewPublisher creates a new NACK event publisher that will publish k8s events
func NewPublisher(client kube.Client) *Publisher {
	eventBroadcaster := record.NewBroadcaster()
	eventRecorder := eventBroadcaster.NewRecorder(
		schemes.DefaultScheme(),
		corev1.EventSource{Component: wellknown.DefaultAgwControllerName},
	)
	eventBroadcaster.StartRecordingToSink(&typedcorev1.EventSinkImpl{
		Interface: client.Kube().CoreV1().Events(""),
	})

	filter := kclient.Filter{ObjectFilter: client.ObjectFilter()}
	gatewayClient := kclient.NewFilteredDelayed[*gwv1.Gateway](client, gvr.KubernetesGateway, filter)
	gatewayClassClient := kclient.NewFilteredDelayed[*gwv1.GatewayClass](client, gvr.GatewayClass, filter)
	agwParamClient := kclient.NewFilteredDelayed[*agentgateway.AgentgatewayParameters](
		client,
		wellknown.AgentgatewayParametersGVR,
		filter,
	)
	deploymentClient := kclient.NewFiltered[*appsv1.Deployment](client, filter)
	daemonSetClient := kclient.NewFiltered[*appsv1.DaemonSet](client, filter)
	return &Publisher{
		eventRecorder:      eventRecorder,
		gatewayClient:      gatewayClient,
		gatewayClassClient: gatewayClassClient,
		agwParamClient:     agwParamClient,
		deploymentClient:   deploymentClient,
		daemonSetClient:    daemonSetClient,
		HasSynced: func() bool {
			return gatewayClient.HasSynced() &&
				gatewayClassClient.HasSynced() &&
				agwParamClient.HasSynced() &&
				deploymentClient.HasSynced() &&
				daemonSetClient.HasSynced()
		},
	}
}

// PublishNack publishes a NACK event as a k8s event.
func (p *Publisher) PublishNack(event *NackEvent) {
	gw := p.gatewayClient.Get(event.Gateway.Name, event.Gateway.Namespace)
	if gw == nil {
		log.Error("failed to get gateway from cache")
		return
	}

	gatewayRef := &corev1.ObjectReference{
		Kind:       wellknown.GatewayKind,
		APIVersion: wellknown.GatewayGVK.GroupVersion().String(),
		Name:       event.Gateway.Name,
		Namespace:  event.Gateway.Namespace,
		UID:        gw.GetUID(),
	}
	p.eventRecorder.Event(gatewayRef, corev1.EventTypeWarning, ReasonNack, event.ErrorMsg)

	workloadRef := p.workloadObjectReference(gw)
	if workloadRef == nil {
		log.Debug("failed to resolve backing workload from cache", "gateway", event.Gateway)
		return
	}
	p.eventRecorder.Event(workloadRef, corev1.EventTypeWarning, ReasonNack, event.ErrorMsg)

	log.Debug("published NACK event for Gateway", "gateway", event.Gateway, "type_url", event.TypeUrl)
}

func (p *Publisher) workloadObjectReference(gw *gwv1.Gateway) *corev1.ObjectReference {
	workloadKind := p.gatewayWorkloadKind(gw)
	var workload metav1.Object
	var gvk schema.GroupVersionKind

	switch workloadKind {
	case agentgateway.AgentgatewayParametersWorkloadDaemonSet:
		daemonSet := p.daemonSetClient.Get(gw.GetName(), gw.GetNamespace())
		if daemonSet == nil {
			return nil
		}
		workload = daemonSet
		gvk = wellknown.DaemonSetGVK
	default:
		deployment := p.deploymentClient.Get(gw.GetName(), gw.GetNamespace())
		if deployment == nil {
			return nil
		}
		workload = deployment
		gvk = wellknown.DeploymentGVK
	}

	return &corev1.ObjectReference{
		Kind:       gvk.Kind,
		APIVersion: gvk.GroupVersion().String(),
		Name:       workload.GetName(),
		Namespace:  workload.GetNamespace(),
		UID:        workload.GetUID(),
	}
}

func (p *Publisher) gatewayWorkloadKind(gw *gwv1.Gateway) agentgateway.AgentgatewayParametersWorkloadKind {
	workloadKind := agentgateway.AgentgatewayParametersWorkloadDeployment

	gwc := p.gatewayClassClient.Get(string(gw.Spec.GatewayClassName), metav1.NamespaceNone)
	if gwc != nil && gwc.Spec.ParametersRef != nil {
		ref := gwc.Spec.ParametersRef
		if ref.Group == agentgateway.GroupName && string(ref.Kind) == wellknown.AgentgatewayParametersGVK.Kind {
			namespace := metav1.NamespaceNone
			if ref.Namespace != nil {
				namespace = string(*ref.Namespace)
			}
			if resolved := p.workloadKindFromParameters(ref.Name, namespace); resolved != "" {
				workloadKind = resolved
			}
		}
	}

	if gw.Spec.Infrastructure != nil && gw.Spec.Infrastructure.ParametersRef != nil {
		ref := gw.Spec.Infrastructure.ParametersRef
		if ref.Group == agentgateway.GroupName && string(ref.Kind) == wellknown.AgentgatewayParametersGVK.Kind {
			if resolved := p.workloadKindFromParameters(ref.Name, gw.GetNamespace()); resolved != "" {
				workloadKind = resolved
			}
		}
	}

	return workloadKind
}

func (p *Publisher) workloadKindFromParameters(
	name string,
	namespace string,
) agentgateway.AgentgatewayParametersWorkloadKind {
	params := p.agwParamClient.Get(name, namespace)
	if params == nil || params.Spec.Workload == nil {
		return ""
	}
	return params.Spec.Workload.Kind
}

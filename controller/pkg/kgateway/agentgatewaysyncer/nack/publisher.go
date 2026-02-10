package nack

import (
	"time"

	"istio.io/istio/pkg/config/schema/gvr"
	"istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/kube/kclient"
	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/types"
	typedcorev1 "k8s.io/client-go/kubernetes/typed/core/v1"
	"k8s.io/client-go/tools/record"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
	"github.com/kgateway-dev/kgateway/v2/pkg/logging"
	"github.com/kgateway-dev/kgateway/v2/pkg/schemes"
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
	eventRecorder    record.EventRecorder
	gatewayClient    kclient.Client[*gwv1.Gateway]
	deploymentClient kclient.Client[*appsv1.Deployment]
	HasSynced        func() bool
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
	deploymentClient := kclient.NewFiltered[*appsv1.Deployment](client, filter)
	return &Publisher{
		eventRecorder:    eventRecorder,
		gatewayClient:    gatewayClient,
		deploymentClient: deploymentClient,
		HasSynced: func() bool {
			return gatewayClient.HasSynced() && deploymentClient.HasSynced()
		},
	}
}

// PublishNack publishes a NACK event as a k8s event.
func (p *Publisher) PublishNack(event *NackEvent) {
	var gatewayUID, deployUID types.UID
	gw := p.gatewayClient.Get(event.Gateway.Name, event.Gateway.Namespace)
	if gw == nil {
		log.Error("failed to get gateway from cache")
		return
	}
	gatewayUID = gw.GetUID()
	dep := p.deploymentClient.Get(event.Gateway.Name, event.Gateway.Namespace)
	if dep == nil {
		log.Error("failed to get deployment from cache")
		return
	}
	deployUID = dep.GetUID()

	gatewayRef := &corev1.ObjectReference{
		Kind:       wellknown.GatewayKind,
		APIVersion: wellknown.GatewayGVK.GroupVersion().String(),
		Name:       event.Gateway.Name,
		Namespace:  event.Gateway.Namespace,
		UID:        gatewayUID,
	}
	deploymentRef := &corev1.ObjectReference{
		Kind:       wellknown.DeploymentGVK.Kind,
		APIVersion: wellknown.DeploymentGVK.GroupVersion().String(),
		Name:       event.Gateway.Name,
		Namespace:  event.Gateway.Namespace,
		UID:        deployUID,
	}

	p.eventRecorder.Eventf(gatewayRef, corev1.EventTypeWarning, ReasonNack, event.ErrorMsg)
	p.eventRecorder.Eventf(deploymentRef, corev1.EventTypeWarning, ReasonNack, event.ErrorMsg)

	log.Debug("published NACK event for Gateway", "gateway", event.Gateway, "typeURL", event.TypeUrl)
}

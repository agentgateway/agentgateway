package deployer

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"log/slog"
	"strings"

	"helm.sh/helm/v3/pkg/chart"
	"istio.io/istio/pkg/kube/kclient"
	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/client-go/tools/cache"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	agwplugins "github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/helm"
)

// Inputs is the set of options used to configure gateway/inference pool deployment.
type Inputs struct {
	ImageDefaults              *agentgateway.Image
	ControlPlane               ControlPlaneInfo
	NoListenersDummyPort       uint16
	AgwCollections             *agwplugins.AgwCollections
	AgentgatewayClassName      string
	AgentgatewayControllerName string
}

// InMemoryGatewayParametersConfig holds the configuration for creating in-memory GatewayParameters.
type InMemoryGatewayParametersConfig struct {
	ClassName                  string
	ImageInfo                  *ImageInfo
	AgwControllerName          string
	OmitDefaultSecurityContext bool
}

var (
	// ErrNoValidPorts is returned when no valid ports are found for the Gateway
	ErrNoValidPorts = errors.New("no valid ports")
)

const sessionKeyChecksumAnnotation = "checksum/session-key"
const oidcCookieSecretChecksumAnnotation = "checksum/oidc-cookie-secret" //nolint:gosec // annotation key, not a credential

func NewGatewayParameters(cli apiclient.Client, inputs *Inputs) *GatewayParameters {
	gp := &GatewayParameters{
		inputs:                 inputs,
		agwHelmValuesGenerator: newAgentgatewayParametersHelmValuesGenerator(cli, inputs),
	}

	return gp
}

type GatewayParameters struct {
	inputs                      *Inputs
	helmValuesGeneratorOverride HelmValuesGenerator
	agwHelmValuesGenerator      *agentgatewayParametersHelmValuesGenerator
}

func (gp *GatewayParameters) WithHelmValuesGeneratorOverride(generator HelmValuesGenerator) *GatewayParameters {
	gp.helmValuesGeneratorOverride = generator
	return gp
}

func (gp *GatewayParameters) WithSessionKeyGenerator(generator func() (string, error)) *GatewayParameters {
	if gp.agwHelmValuesGenerator != nil && generator != nil {
		gp.agwHelmValuesGenerator.sessionKeyGen = generator
	}
	return gp
}

func (gp *GatewayParameters) WithOIDCCookieSecretGenerator(generator func() (string, error)) *GatewayParameters {
	if gp.agwHelmValuesGenerator != nil && generator != nil {
		gp.agwHelmValuesGenerator.oidcCookieGen = generator
	}
	return gp
}

// GetAgentgatewayParametersClient returns the AgentgatewayParameters client if Agentgateway is enabled, nil otherwise.
// This allows the reconciler to reuse the same client for watching changes.
func (gp *GatewayParameters) GetAgentgatewayParametersClient() kclient.Client[*agentgateway.AgentgatewayParameters] {
	if gp.agwHelmValuesGenerator != nil {
		return gp.agwHelmValuesGenerator.agwParamClient
	}
	return nil
}

func LoadAgentgatewayChart() (*chart.Chart, error) {
	return loadChart(helm.AgentgatewayHelmChart)
}

func (gp *GatewayParameters) GetValues(ctx context.Context, obj client.Object) (map[string]any, error) {
	generator, err := gp.getHelmValuesGenerator(obj)
	if err != nil {
		return nil, err
	}

	return generator.GetValues(ctx, obj)
}

func (gp *GatewayParameters) GetCacheSyncHandlers() []cache.InformerSynced {
	if gp.helmValuesGeneratorOverride != nil {
		return gp.helmValuesGeneratorOverride.GetCacheSyncHandlers()
	}

	var handlers []cache.InformerSynced
	if gp.agwHelmValuesGenerator != nil {
		handlers = append(handlers, gp.agwHelmValuesGenerator.GetCacheSyncHandlers()...)
	}
	return handlers
}

// PostProcessObjects implements deployer.ObjectPostProcessor.
// It applies GatewayParameters or AgentgatewayParameters overlays to the rendered objects.
// When both GatewayClass and Gateway have parameters, the overlays
// are applied in order: GatewayClass first, then Gateway on top.
func (gp *GatewayParameters) PostProcessObjects(ctx context.Context, obj client.Object, rendered []client.Object) ([]client.Object, error) {
	// Check if override implements ObjectPostProcessor and delegate to it
	if gp.helmValuesGeneratorOverride != nil {
		if postProcessor, ok := gp.helmValuesGeneratorOverride.(ObjectPostProcessor); ok {
			return postProcessor.PostProcessObjects(ctx, obj, rendered)
		}
	}

	gw, ok := obj.(*gwv1.Gateway)
	if !ok {
		return rendered, nil
	}

	// Determine which controller this Gateway uses
	var gwClassClient kclient.Client[*gwv1.GatewayClass]
	if gp.agwHelmValuesGenerator != nil {
		gwClassClient = gp.agwHelmValuesGenerator.gwClassClient
	} else {
		return nil, fmt.Errorf("no controller enabled for Gateway %s/%s", gw.GetNamespace(), gw.GetName())
	}

	gwc, err := getGatewayClassFromGateway(gwClassClient, gw)
	if err != nil {
		return nil, fmt.Errorf("failed to get GatewayClass for Gateway %s/%s: %w", gw.GetNamespace(), gw.GetName(), err)
	}

	// Check if this is an agentgateway or envoy gateway
	if string(gwc.Spec.ControllerName) == gp.inputs.AgentgatewayControllerName {
		// Agentgateway overlays
		if gp.agwHelmValuesGenerator == nil {
			// Agentgateway not enabled; skip overlays (not an error since overlays are optional).
			return rendered, nil
		}
		resolved, err := gp.agwHelmValuesGenerator.GetResolvedParametersForGateway(gw)
		if err != nil {
			return nil, fmt.Errorf("failed to resolve AgentgatewayParameters for Gateway %s/%s: %w", gw.GetNamespace(), gw.GetName(), err)
		}

		// Apply overlays in order: GatewayClass first, then Gateway.
		if resolved.gatewayClassAGWP != nil {
			applier := NewAgentgatewayParametersApplier(resolved.gatewayClassAGWP)
			rendered, err = applier.ApplyOverlaysToObjects(rendered)
			if err != nil {
				return nil, err
			}
		}
		if resolved.gatewayAGWP != nil {
			applier := NewAgentgatewayParametersApplier(resolved.gatewayAGWP)
			rendered, err = applier.ApplyOverlaysToObjects(rendered)
			if err != nil {
				return nil, err
			}
		}
		if usesManagedSessionKeyResolvedParameters(resolved) {
			sessionKeySecret, err := gp.agwHelmValuesGenerator.buildSessionKeySecret(
				ctx,
				gw,
				gatewaySessionKeySecretName(gw.Name),
			)
			if err != nil {
				return nil, fmt.Errorf("failed to build session key secret for Gateway %s/%s: %w", gw.GetNamespace(), gw.GetName(), err)
			}
			if err := addSessionKeyChecksumAnnotation(rendered, sessionKeySecret); err != nil {
				return nil, fmt.Errorf("failed to annotate session key checksum for Gateway %s/%s: %w", gw.GetNamespace(), gw.GetName(), err)
			}
			rendered = append(rendered, sessionKeySecret)
		}
		if usesManagedOIDCCookieSecretResolvedParameters(resolved) && gp.agwHelmValuesGenerator.gatewayRequiresOIDCCookieSecret(gw) {
			oidcCookieSecret, err := gp.agwHelmValuesGenerator.buildOIDCCookieSecret(
				ctx,
				gw,
				gatewayOIDCCookieSecretName(gw.Name),
			)
			if err != nil {
				return nil, fmt.Errorf("failed to build oidc cookie secret for Gateway %s/%s: %w", gw.GetNamespace(), gw.GetName(), err)
			}
			if err := addSecretChecksumAnnotation(rendered, oidcCookieSecret, oidcCookieSecretChecksumAnnotation, "oidc cookie secret"); err != nil {
				return nil, fmt.Errorf("failed to annotate oidc cookie secret checksum for Gateway %s/%s: %w", gw.GetNamespace(), gw.GetName(), err)
			}
			rendered = append(rendered, oidcCookieSecret)
		}
	}

	return rendered, nil
}

func addSessionKeyChecksumAnnotation(rendered []client.Object, secret *corev1.Secret) error {
	return addSecretChecksumAnnotation(rendered, secret, sessionKeyChecksumAnnotation, "session key secret")
}

func addSecretChecksumAnnotation(rendered []client.Object, secret *corev1.Secret, annotationKey, secretKind string) error {
	key, found := secret.Data["key"]
	if !found || len(key) == 0 {
		return fmt.Errorf("%s %s/%s missing key entry", secretKind, secret.Namespace, secret.Name)
	}

	checksum := sha256.Sum256(key)
	checksumHex := hex.EncodeToString(checksum[:])

	for _, obj := range rendered {
		deployment, ok := obj.(*appsv1.Deployment)
		if !ok {
			continue
		}
		if deployment.Spec.Template.Annotations == nil {
			deployment.Spec.Template.Annotations = map[string]string{}
		}
		deployment.Spec.Template.Annotations[annotationKey] = checksumHex
	}

	return nil
}

func GatewayReleaseNameAndNamespace(obj client.Object) (string, string) {
	// A helm release is never installed, only a template is generated, so the name doesn't matter
	// Use a hard-coded name to avoid going over the 53 character name limit
	return "release-name-placeholder", obj.GetNamespace()
}

func (gp *GatewayParameters) getHelmValuesGenerator(obj client.Object) (HelmValuesGenerator, error) {
	gw, ok := obj.(*gwv1.Gateway)
	if !ok {
		return nil, fmt.Errorf("expected a Gateway resource, got %s", obj.GetObjectKind().GroupVersionKind().String())
	}

	if gp.helmValuesGeneratorOverride != nil {
		slog.Debug("using override HelmValuesGenerator for Gateway",
			"gateway_name", gw.GetName(),
			"gateway_namespace", gw.GetNamespace(),
		)
		return gp.helmValuesGeneratorOverride, nil
	}

	return gp.agwHelmValuesGenerator, nil
}

func getGatewayClassFromGateway(cli kclient.Client[*gwv1.GatewayClass], gw *gwv1.Gateway) (*gwv1.GatewayClass, error) {
	if gw == nil {
		return nil, errors.New("nil Gateway")
	}
	if gw.Spec.GatewayClassName == "" {
		return nil, errors.New("GatewayClassName must not be empty")
	}

	gwc := cli.Get(string(gw.Spec.GatewayClassName), metav1.NamespaceNone)
	if gwc == nil {
		return nil, fmt.Errorf("failed to get GatewayClass for Gateway %s/%s", gw.GetName(), gw.GetNamespace())
	}

	return gwc, nil
}

func translateInfraMeta[K ~string, V ~string](meta map[K]V) map[string]string {
	infra := make(map[string]string, len(meta))
	for k, v := range meta {
		if strings.HasPrefix(string(k), "gateway.networking.k8s.io/") {
			continue // ignore this prefix to avoid conflicts
		}
		infra[string(k)] = string(v)
	}
	return infra
}

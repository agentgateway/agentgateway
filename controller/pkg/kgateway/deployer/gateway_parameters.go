package deployer

import (
	"context"
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
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/deployer"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/helm"
)

var (
	// ErrNoValidPorts is returned when no valid ports are found for the Gateway
	ErrNoValidPorts = errors.New("no valid ports")
)

const sessionKeyChecksumAnnotation = "checksum/session-key"

func NewGatewayParameters(cli apiclient.Client, inputs *deployer.Inputs) *GatewayParameters {
	gp := &GatewayParameters{
		inputs:                 inputs,
		agwHelmValuesGenerator: newAgentgatewayParametersHelmValuesGenerator(cli, inputs),
	}

	return gp
}

type GatewayParameters struct {
	inputs                      *deployer.Inputs
	helmValuesGeneratorOverride deployer.HelmValuesGenerator
	agwHelmValuesGenerator      *agentgatewayParametersHelmValuesGenerator
}

func (gp *GatewayParameters) WithHelmValuesGeneratorOverride(generator deployer.HelmValuesGenerator) *GatewayParameters {
	gp.helmValuesGeneratorOverride = generator
	return gp
}

func (gp *GatewayParameters) WithSessionKeyGenerator(generator func() (string, error)) *GatewayParameters {
	if gp.agwHelmValuesGenerator != nil && generator != nil {
		gp.agwHelmValuesGenerator.sessionKeyGen = generator
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
		if postProcessor, ok := gp.helmValuesGeneratorOverride.(deployer.ObjectPostProcessor); ok {
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
		sessionKeySecret, err := gp.agwHelmValuesGenerator.buildSessionKeySecret(
			ctx,
			gw,
			gatewaySessionKeySecretName(gw.Name),
		)
		if err != nil {
			return nil, fmt.Errorf("failed to build session key secret for Gateway %s/%s: %w", gw.GetNamespace(), gw.GetName(), err)
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
		if err := enforceManagedSessionKeyDeploymentWiring(rendered, sessionKeySecret); err != nil {
			return nil, fmt.Errorf("failed to enforce managed session key wiring for Gateway %s/%s: %w", gw.GetNamespace(), gw.GetName(), err)
		}
		rendered = append(rendered, sessionKeySecret)
	}

	return rendered, nil
}

func addSessionKeyChecksumAnnotation(rendered []client.Object, secret *corev1.Secret) error {
	checksumHex, err := sessionKeyChecksum(secret)
	if err != nil {
		return err
	}

	for _, obj := range rendered {
		deployment, ok := obj.(*appsv1.Deployment)
		if !ok {
			continue
		}
		if deployment.Spec.Template.Annotations == nil {
			deployment.Spec.Template.Annotations = map[string]string{}
		}
		deployment.Spec.Template.Annotations[sessionKeyChecksumAnnotation] = checksumHex
	}

	return nil
}

func enforceManagedSessionKeyDeploymentWiring(rendered []client.Object, secret *corev1.Secret) error {
	if err := addSessionKeyChecksumAnnotation(rendered, secret); err != nil {
		return err
	}

	for _, obj := range rendered {
		deployment, ok := obj.(*appsv1.Deployment)
		if !ok {
			continue
		}
		if err := enforceManagedSessionKeyDeploymentSpec(deployment, secret.Name); err != nil {
			return err
		}
	}

	return nil
}

func enforceManagedSessionKeyDeploymentSpec(deployment *appsv1.Deployment, secretName string) error {
	if err := enforceManagedSessionKeyVolume(deployment, secretName); err != nil {
		return err
	}

	for containerIndex := range deployment.Spec.Template.Spec.Containers {
		container := &deployment.Spec.Template.Spec.Containers[containerIndex]
		if container.Name != "agentgateway" {
			continue
		}

		managedEnv := corev1.EnvVar{
			Name:  sessionKeyringFileEnvVar,
			Value: managedSessionKeyMountPath + "/" + managedSessionKeyFileName,
		}
		managedMount := corev1.VolumeMount{
			Name:      managedSessionKeyVolumeName,
			MountPath: managedSessionKeyMountPath,
			ReadOnly:  true,
		}

		container.Env = removeEnvVar(container.Env, sessionKeyEnvVar)
		container.Env = replaceEnvVarPreservingPosition(container.Env, managedEnv)
		container.VolumeMounts = replaceVolumeMountPreservingPosition(container.VolumeMounts, managedMount)
		return nil
	}

	return fmt.Errorf("deployment %s/%s is missing the agentgateway container", deployment.Namespace, deployment.Name)
}

func replaceEnvVarPreservingPosition(envs []corev1.EnvVar, replacement corev1.EnvVar) []corev1.EnvVar {
	insertIndex := len(envs)
	filtered := make([]corev1.EnvVar, 0, len(envs)+1)
	for _, env := range envs {
		if env.Name == replacement.Name {
			if insertIndex == len(envs) {
				insertIndex = len(filtered)
			}
			continue
		}
		filtered = append(filtered, env)
	}

	if insertIndex > len(filtered) {
		insertIndex = len(filtered)
	}
	filtered = append(filtered, corev1.EnvVar{})
	copy(filtered[insertIndex+1:], filtered[insertIndex:])
	filtered[insertIndex] = replacement
	return filtered
}

func removeEnvVar(envs []corev1.EnvVar, name string) []corev1.EnvVar {
	filtered := make([]corev1.EnvVar, 0, len(envs))
	for _, env := range envs {
		if env.Name == name {
			continue
		}
		filtered = append(filtered, env)
	}
	return filtered
}

func enforceManagedSessionKeyVolume(deployment *appsv1.Deployment, secretName string) error {
	managedVolume := corev1.Volume{
		Name: managedSessionKeyVolumeName,
		VolumeSource: corev1.VolumeSource{
			Secret: &corev1.SecretVolumeSource{
				SecretName: secretName,
				Items: []corev1.KeyToPath{
					{
						Key:  managedSessionKeyDataKey,
						Path: managedSessionKeyFileName,
					},
				},
			},
		},
	}
	deployment.Spec.Template.Spec.Volumes = replaceVolumePreservingPosition(
		deployment.Spec.Template.Spec.Volumes,
		managedVolume,
	)
	return nil
}

func replaceVolumeMountPreservingPosition(mounts []corev1.VolumeMount, replacement corev1.VolumeMount) []corev1.VolumeMount {
	insertIndex := len(mounts)
	filtered := make([]corev1.VolumeMount, 0, len(mounts)+1)
	for _, mount := range mounts {
		if mount.Name == replacement.Name {
			if insertIndex == len(mounts) {
				insertIndex = len(filtered)
			}
			continue
		}
		filtered = append(filtered, mount)
	}

	if insertIndex > len(filtered) {
		insertIndex = len(filtered)
	}
	filtered = append(filtered, corev1.VolumeMount{})
	copy(filtered[insertIndex+1:], filtered[insertIndex:])
	filtered[insertIndex] = replacement
	return filtered
}

func replaceVolumePreservingPosition(volumes []corev1.Volume, replacement corev1.Volume) []corev1.Volume {
	insertIndex := len(volumes)
	filtered := make([]corev1.Volume, 0, len(volumes)+1)
	for _, volume := range volumes {
		if volume.Name == replacement.Name {
			if insertIndex == len(volumes) {
				insertIndex = len(filtered)
			}
			continue
		}
		filtered = append(filtered, volume)
	}

	if insertIndex > len(filtered) {
		insertIndex = len(filtered)
	}
	filtered = append(filtered, corev1.Volume{})
	copy(filtered[insertIndex+1:], filtered[insertIndex:])
	filtered[insertIndex] = replacement
	return filtered
}

func GatewayReleaseNameAndNamespace(obj client.Object) (string, string) {
	// A helm release is never installed, only a template is generated, so the name doesn't matter
	// Use a hard-coded name to avoid going over the 53 character name limit
	return "release-name-placeholder", obj.GetNamespace()
}

func (gp *GatewayParameters) getHelmValuesGenerator(obj client.Object) (deployer.HelmValuesGenerator, error) {
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

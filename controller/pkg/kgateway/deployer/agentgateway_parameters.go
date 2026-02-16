package deployer

import (
	"context"
	"fmt"
	"maps"

	"istio.io/istio/pkg/kube/kclient"
	"istio.io/istio/pkg/util/smallset"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/util/sets"
	"k8s.io/client-go/tools/cache"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/deployer"
	"github.com/agentgateway/agentgateway/controller/pkg/deployer/strategicpatch"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/collections"
)

// AgentgatewayParametersApplier applies AgentgatewayParameters configurations and overlays.
type AgentgatewayParametersApplier struct {
	params *agentgateway.AgentgatewayParameters
}

// NewAgentgatewayParametersApplier creates a new applier from the resolved parameters.
func NewAgentgatewayParametersApplier(params *agentgateway.AgentgatewayParameters) *AgentgatewayParametersApplier {
	return &AgentgatewayParametersApplier{params: params}
}

func setIfNonNil[T any](dst **T, src *T) {
	if src != nil {
		*dst = src
	}
}

func setIfNonZero[T comparable](dst *T, src T) {
	var zero T
	if src != zero {
		*dst = src
	}
}

// ApplyToHelmValues applies the AgentgatewayParameters configs to the helm
// values.  This is called before rendering the helm chart. (We render a helm
// chart, but we do not use helm beyond that point.)
func (a *AgentgatewayParametersApplier) ApplyToHelmValues(vals *deployer.HelmConfig) {
	if a.params == nil || vals == nil || vals.Agentgateway == nil {
		return
	}

	// Deep copy to avoid mutating the cached AgentgatewayParameters object.
	// Without this, the first Apply (GatewayClass) can alias configs.Resources
	// into res, and the second Apply (Gateway) would mutate the cached
	// GatewayClass object when merging maps in-place.
	configs := *a.params.Spec.AgentgatewayParametersConfigs.DeepCopy()
	res := vals.Agentgateway.AgentgatewayParametersConfigs

	// Do a manual merge of the fields.
	if configs.Image != nil {
		if res.Image == nil {
			res.Image = &agentgateway.Image{}
		}
		setIfNonNil(&res.Image.Tag, configs.Image.Tag)
		setIfNonNil(&res.Image.Registry, configs.Image.Registry)
		setIfNonNil(&res.Image.Repository, configs.Image.Repository)
		setIfNonNil(&res.Image.PullPolicy, configs.Image.PullPolicy)
		setIfNonNil(&res.Image.Digest, configs.Image.Digest)
	}
	// Merge resources field-by-field to preserve values from GatewayClass AGWP
	// when Gateway AGWP only sets some fields (e.g., GWC sets limits, GW sets requests).
	res.Resources = DeepMergeResourceRequirements(res.Resources, configs.Resources)
	setIfNonNil(&res.Shutdown, configs.Shutdown)
	// Merge Istio field-by-field to preserve values from GatewayClass AGWP
	// when Gateway AGWP only sets some fields (e.g., GWC sets caAddress, GW sets trustDomain).
	if configs.Istio != nil {
		if res.Istio == nil {
			res.Istio = &agentgateway.IstioSpec{}
		}
		setIfNonZero(&res.Istio.CaAddress, configs.Istio.CaAddress)
		setIfNonZero(&res.Istio.TrustDomain, configs.Istio.TrustDomain)
	}
	setIfNonNil(&res.RawConfig, configs.RawConfig)

	// Apply logging.level as RUST_LOG first, then merge explicit env vars on top.
	// This ensures explicit env vars override logging.level if both specify RUST_LOG.
	if configs.Logging != nil {
		if res.Logging == nil {
			res.Logging = &agentgateway.AgentgatewayParametersLogging{}
		}
		setIfNonZero(&res.Logging.Level, configs.Logging.Level)
		setIfNonZero(&res.Logging.Format, configs.Logging.Format)
	}

	// Apply explicit environment variables last so they can override logging.level.
	res.Env = mergeEnvVars(res.Env, configs.Env)

	vals.Agentgateway.AgentgatewayParametersConfigs = res
}

// mergeEnvVars merges two slices of environment variables.
// Variables in 'override' take precedence over variables in 'base' with the same name.
// The order is preserved: base vars first (minus overridden ones), then override vars.
func mergeEnvVars(base, override []corev1.EnvVar) []corev1.EnvVar {
	if len(override) == 0 {
		return base
	}
	if len(base) == 0 {
		return override
	}

	// Build a set of names in override
	overrideNames := make(map[string]struct{}, len(override))
	for _, env := range override {
		overrideNames[env.Name] = struct{}{}
	}

	// Keep base vars that are not overridden
	result := make([]corev1.EnvVar, 0, len(base)+len(override))
	for _, env := range base {
		if _, exists := overrideNames[env.Name]; !exists {
			result = append(result, env)
		}
	}

	// Append all override vars
	result = append(result, override...)
	return result
}

// ApplyOverlaysToObjects applies the strategic-merge-patch overlays to rendered k8s objects.
// This is called after rendering the helm chart.
// It returns the (potentially modified) slice of objects, as new objects may be added
// (e.g., PodDisruptionBudget, HorizontalPodAutoscaler).
func (a *AgentgatewayParametersApplier) ApplyOverlaysToObjects(objs []client.Object) ([]client.Object, error) {
	if a.params == nil {
		return objs, nil
	}
	applier := strategicpatch.NewOverlayApplier(a.params)
	return applier.ApplyOverlays(objs)
}

type agentgatewayParametersHelmValuesGenerator struct {
	agwParamClient kclient.Client[*agentgateway.AgentgatewayParameters]
	gwClassClient  kclient.Client[*gwv1.GatewayClass]
	inputs         *deployer.Inputs
}

func newAgentgatewayParametersHelmValuesGenerator(cli apiclient.Client, inputs *deployer.Inputs) *agentgatewayParametersHelmValuesGenerator {
	return &agentgatewayParametersHelmValuesGenerator{
		agwParamClient: kclient.NewFilteredDelayed[*agentgateway.AgentgatewayParameters](cli, wellknown.AgentgatewayParametersGVR, kclient.Filter{ObjectFilter: cli.ObjectFilter()}),
		gwClassClient:  kclient.NewFilteredDelayed[*gwv1.GatewayClass](cli, wellknown.GatewayClassGVR, kclient.Filter{ObjectFilter: cli.ObjectFilter()}),
		inputs:         inputs,
	}
}

// GetValues returns helm values derived from AgentgatewayParameters.
func (g *agentgatewayParametersHelmValuesGenerator) GetValues(ctx context.Context, obj client.Object) (map[string]any, error) {
	gw, ok := obj.(*gwv1.Gateway)
	if !ok {
		return nil, fmt.Errorf("expected a Gateway resource, got %s", obj.GetObjectKind().GroupVersionKind().String())
	}

	resolved, err := g.resolveParameters(gw)
	if err != nil {
		return nil, err
	}

	vals, err := g.getDefaultAgentgatewayHelmValues(gw)
	if err != nil {
		return nil, err
	}

	// Apply AGWP Configs in order: GatewayClass first, then Gateway on top.
	// This allows Gateway-level configs to override GatewayClass-level configs.
	if resolved.gatewayClassAGWP != nil {
		applier := NewAgentgatewayParametersApplier(resolved.gatewayClassAGWP)
		applier.ApplyToHelmValues(vals)
	}
	if resolved.gatewayAGWP != nil {
		applier := NewAgentgatewayParametersApplier(resolved.gatewayAGWP)
		applier.ApplyToHelmValues(vals)
	}

	if g.inputs.ControlPlane.XdsTLS {
		if err := injectXdsCACertificate(g.inputs.ControlPlane.XdsTlsCaPath, vals); err != nil {
			return nil, fmt.Errorf("failed to inject xDS CA certificate: %w", err)
		}
	}

	var jsonVals map[string]any
	err = deployer.JsonConvert(vals, &jsonVals)
	return jsonVals, err
}

// resolvedParameters holds the resolved parameters for a Gateway, supporting
// both GatewayClass-level and Gateway-level AgentgatewayParameters.
type resolvedParameters struct {
	// gatewayClassAGWP is the AgentgatewayParameters from the GatewayClass (if any).
	gatewayClassAGWP *agentgateway.AgentgatewayParameters
	// gatewayAGWP is the AgentgatewayParameters from the Gateway (if any).
	gatewayAGWP *agentgateway.AgentgatewayParameters
}

// resolveParameters resolves the AgentgatewayParameters for the Gateway.
// It returns both GatewayClass-level and Gateway-level
// separately to support ordered overlay merging (GatewayClass first, then Gateway).
func (g *agentgatewayParametersHelmValuesGenerator) resolveParameters(gw *gwv1.Gateway) (*resolvedParameters, error) {
	result := &resolvedParameters{}

	// Get GatewayClass parameters first
	gwc := g.gwClassClient.Get(string(gw.Spec.GatewayClassName), metav1.NamespaceNone)
	if gwc != nil && gwc.Spec.ParametersRef != nil {
		ref := gwc.Spec.ParametersRef

		// Check for AgentgatewayParameters on GatewayClass
		if ref.Group == agentgateway.GroupName && string(ref.Kind) == wellknown.AgentgatewayParametersGVK.Kind {
			agwpNamespace := ""
			if ref.Namespace != nil {
				agwpNamespace = string(*ref.Namespace)
			}
			agwp := g.agwParamClient.Get(ref.Name, agwpNamespace)
			if agwp == nil {
				return nil, fmt.Errorf("for GatewayClass %s, AgentgatewayParameters %s/%s not found",
					gwc.GetName(), agwpNamespace, ref.Name)
			}
			result.gatewayClassAGWP = agwp
		} else {
			return nil, fmt.Errorf("the GatewayClass %s references parameters of a type other than AgentgatewayParameters: %s",
				gwc.GetName(), ref.Name)
		}
	}

	// Check if Gateway has its own parametersRef
	if gw.Spec.Infrastructure != nil && gw.Spec.Infrastructure.ParametersRef != nil {
		ref := gw.Spec.Infrastructure.ParametersRef

		if ref.Group == agentgateway.GroupName && ref.Kind == gwv1.Kind(wellknown.AgentgatewayParametersGVK.Kind) {
			agwp := g.agwParamClient.Get(ref.Name, gw.GetNamespace())
			if agwp == nil {
				return nil, fmt.Errorf("AgentgatewayParameters %s/%s not found for Gateway %s/%s",
					gw.GetNamespace(), ref.Name, gw.GetNamespace(), gw.GetName())
			}
			result.gatewayAGWP = agwp
			return result, nil
		}

		return nil, fmt.Errorf("infrastructure.parametersRef on Gateway %s/%s references unsupported type: group=%s kind=%s; use AgentgatewayParameters instead",
			gw.GetNamespace(), gw.GetName(), ref.Group, ref.Kind)
	}

	return result, nil
}

func (g *agentgatewayParametersHelmValuesGenerator) GetCacheSyncHandlers() []cache.InformerSynced {
	return []cache.InformerSynced{g.agwParamClient.HasSynced, g.gwClassClient.HasSynced}
}

// GetResolvedParametersForGateway returns both the GatewayClass-level and Gateway-level
// AgentgatewayParameters for the given Gateway. This allows callers to apply overlays
// in order (GatewayClass first, then Gateway).
func (g *agentgatewayParametersHelmValuesGenerator) GetResolvedParametersForGateway(gw *gwv1.Gateway) (*resolvedParameters, error) {
	return g.resolveParameters(gw)
}
func DefaultGatewayIRGetter(gw *gwv1.Gateway, commonCollections *collections.CommonCollections) *collections.GatewayForDeployer {
	gwKey := collections.ObjectSource{
		Group:     wellknown.GatewayGVK.GroupKind().Group,
		Kind:      wellknown.GatewayGVK.GroupKind().Kind,
		Name:      gw.GetName(),
		Namespace: gw.GetNamespace(),
	}

	irGW := commonCollections.GatewaysForDeployer.GetKey(gwKey.ResourceName())
	if irGW == nil {
		// If its not in the IR we cannot tell, so need to make a guess.
		controllerNameGuess := commonCollections.AgentgatewayControllerName
		irGW = GatewayIRFrom(gw, controllerNameGuess)
	}

	return irGW
}
func (g *agentgatewayParametersHelmValuesGenerator) getDefaultAgentgatewayHelmValues(gw *gwv1.Gateway) (*deployer.HelmConfig, error) {
	irGW := DefaultGatewayIRGetter(gw, g.inputs.CommonCollections)
	ports := deployer.GetPortsValues(irGW, int32(g.inputs.NoListenersDummyPort))
	if len(ports) == 0 {
		return nil, ErrNoValidPorts
	}

	gtw := &deployer.AgentgatewayHelmGateway{
		Name: &gw.Name,
		GatewayClassName: func() *string {
			s := string(gw.Spec.GatewayClassName)
			return &s
		}(),
		Ports: ports,
		Xds: &deployer.HelmXds{
			Host: &g.inputs.ControlPlane.XdsHost,
			Port: &g.inputs.ControlPlane.AgwXdsPort,
			Tls: &deployer.HelmXdsTls{
				Enabled: func() *bool { b := g.inputs.ControlPlane.XdsTLS; return &b }(),
				CaCert:  &g.inputs.ControlPlane.XdsTlsCaPath,
			},
		},
	}

	if i := gw.Spec.Infrastructure; i != nil {
		gtw.GatewayAnnotations = translateInfraMeta(i.Annotations)
		gtw.GatewayLabels = translateInfraMeta(i.Labels)
	}

	gtw.Image = &agentgateway.Image{
		Registry:   g.inputs.ImageDefaults.Registry,
		Repository: g.inputs.ImageDefaults.Repository,
		Tag:        g.inputs.ImageDefaults.Tag,
		PullPolicy: nil,
	}

	gtw.Service = &deployer.AgentgatewayHelmService{}
	// Extract loadBalancerIP from Gateway.spec.addresses and set it on the service
	if err := deployer.SetLoadBalancerIPFromGatewayForAgentgateway(gw, gtw.Service); err != nil {
		return nil, err
	}

	return &deployer.HelmConfig{Agentgateway: gtw}, nil
}

func GatewayIRFrom(gw *gwv1.Gateway, controllerNameGuess string) *collections.GatewayForDeployer {
	ports := sets.New[int32]()
	for _, l := range gw.Spec.Listeners {
		ports.Insert(l.Port)
	}
	return &collections.GatewayForDeployer{
		ObjectSource: collections.ObjectSource{
			Group:     gwv1.GroupVersion.Group,
			Kind:      wellknown.GatewayKind,
			Namespace: gw.Namespace,
			Name:      gw.Name,
		},
		ControllerName: controllerNameGuess,
		Ports:          smallset.New(ports.UnsortedList()...),
	}
}
func DeepMergeResourceRequirements(dst, src *corev1.ResourceRequirements) *corev1.ResourceRequirements {
	// nil src override means just use dst
	if src == nil {
		return dst
	}

	if dst == nil {
		return src
	}

	dst.Limits = DeepMergeMaps(dst.Limits, src.Limits)
	dst.Requests = DeepMergeMaps(dst.Requests, src.Requests)

	return dst
}

// DeepMergeMaps will use dst if src is nil, src if dest is nil, or add all entries from src into dst
// if neither are nil
func DeepMergeMaps[keyT comparable, valT any](dst, src map[keyT]valT) map[keyT]valT {
	// nil src override means just use dst
	if src == nil {
		return dst
	}

	if dst == nil || len(src) == 0 {
		return src
	}

	maps.Copy(dst, src)
	return dst
}

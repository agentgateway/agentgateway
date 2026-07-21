package translator

import (
	"context"
	"fmt"
	"net/netip"
	"net/url"
	"slices"
	"strings"
	"time"

	"google.golang.org/protobuf/types/known/durationpb"
	"istio.io/istio/pkg/config"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	agwir "github.com/agentgateway/agentgateway/controller/pkg/agentgateway/ir"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/reporter"
	"github.com/agentgateway/agentgateway/controller/pkg/reports"
	"github.com/agentgateway/agentgateway/controller/pkg/syncer/status"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

func AgwModelCollection(
	queue *status.StatusCollections,
	models krt.Collection[*agentgateway.AgentgatewayModel],
	inputs RouteContextInputs,
	krtopts krtutil.KrtOptions,
) (krt.Collection[agwir.AgwResource], krt.Collection[*plugins.RouteAttachment]) {
	modelStatus, modelResources := krt.NewStatusManyCollection(models, func(krtctx krt.HandlerContext, obj *agentgateway.AgentgatewayModel) (*agentgateway.AgentgatewayModelStatus, []agwir.AgwResource) {
		ctx := inputs.WithCtx(krtctx)
		rm := reports.NewReportMap()
		rep := reports.NewReporter(&rm)
		routeReporter := rep.Route(obj)

		parentRefs := extractParentReferenceInfo(ctx, inputs.RouteParents, obj)
		resources := translateModelForParents(ctx, obj, parentRefs, routeReporter)

		status := rm.BuildRouteStatusWithParentRefDefaulting(context.Background(), obj, inputs.ControllerName, true)
		if status == nil {
			return &agentgateway.AgentgatewayModelStatus{}, resources
		}
		return &agentgateway.AgentgatewayModelStatus{Parents: status.Parents}, resources
	}, krtopts.ToOptions("translator/AgentgatewayModels")...)
	status.RegisterStatus(queue, modelStatus, GetStatus)

	attachments := gatewayRouteAttachmentCollection(inputs, models, wellknown.AgentgatewayModelGVK, krtopts)
	return modelResources, attachments
}

func translateModelForParents(
	ctx RouteContext,
	model *agentgateway.AgentgatewayModel,
	parentRefs []RouteParentReference,
	routeReporter reporter.RouteReporter,
) []agwir.AgwResource {
	allowed := map[string]struct{}{}
	for _, p := range FilteredReferences(parentRefs) {
		allowed[modelParentKey(p)] = struct{}{}
	}

	type parentAgg struct {
		anyAllowed bool
		parentRefs []RouteParentReference
	}
	agg := map[string]*parentAgg{}
	denied := map[string]*ParentError{}
	for _, parent := range parentRefs {
		statusKey := parentStatusKey(parent)
		if agg[statusKey] == nil {
			agg[statusKey] = &parentAgg{}
		}
		agg[statusKey].parentRefs = append(agg[statusKey].parentRefs, parent)
		if parent.DeniedReason != nil {
			denied[statusKey] = parent.DeniedReason
		}
	}

	var resources []agwir.AgwResource
	var conversionErr *reporter.RouteCondition
	for _, parent := range parentRefs {
		if _, ok := allowed[modelParentKey(parent)]; !ok {
			continue
		}
		if a := agg[parentStatusKey(parent)]; a != nil {
			a.anyAllowed = true
		}
		parentResources, err := convertAgentgatewayModel(ctx, model, parent)
		if err != nil {
			conversionErr = &reporter.RouteCondition{
				Type:    gwv1.RouteConditionResolvedRefs,
				Status:  "False",
				Reason:  "Invalid",
				Message: err.Error(),
			}
			continue
		}
		for _, resource := range parentResources {
			resources = append(resources, ToResourceForGateway(parent.ParentGateway, resource))
		}
	}

	resolvedOK := conversionErr == nil
	for statusKey, a := range agg {
		for _, parent := range a.parentRefs {
			prStatusRef := parent.OriginalReference
			prStatusRef.Kind = new(gwv1.Kind(parent.ParentKey.Kind))
			prStatusRef.Namespace = new(gwv1.Namespace(parent.ParentKey.Namespace))
			prStatusRef.Name = gwv1.ObjectName(parent.ParentKey.Name)
			prStatusRef.SectionName = nil

			pr := routeReporter.ParentRef(&prStatusRef)
			if a.anyAllowed {
				pr.SetCondition(reporter.RouteCondition{
					Type:    gwv1.RouteConditionAccepted,
					Status:  "True",
					Reason:  gwv1.RouteReasonAccepted,
					Message: reports.AgentgatewayModelAcceptedMessage,
				})
			} else {
				reason := gwv1.RouteReasonNoMatchingParent
				msg := "No listener matched the parent reference"
				if dr := denied[statusKey]; dr != nil {
					reason = gwv1.RouteConditionReason(dr.Reason)
					msg = dr.Message
				}
				pr.SetCondition(reporter.RouteCondition{
					Type:    gwv1.RouteConditionAccepted,
					Status:  "False",
					Reason:  reason,
					Message: msg,
				})
			}
			pr.SetCondition(reporter.RouteCondition{
				Type: gwv1.RouteConditionResolvedRefs,
				Status: func() metav1.ConditionStatus {
					if resolvedOK {
						return metav1.ConditionTrue
					}
					return metav1.ConditionFalse
				}(),
				Reason:  reasonResolvedRefs(conversionErr, resolvedOK),
				Message: routeConditionMessage(conversionErr),
			})
		}
	}
	return resources
}

func parentStatusKey(parent RouteParentReference) string {
	return fmt.Sprintf("%s/%s/%s", parent.ParentKey.Namespace, parent.ParentKey.Name, parent.ParentKey.Kind)
}

func modelParentKey(parent RouteParentReference) string {
	return fmt.Sprintf("%s/%s/%s/%s", parent.ParentKey.Namespace, parent.ParentKey.Name, parent.ParentKey.Kind, string(parent.ParentSection))
}

func convertAgentgatewayModel(ctx RouteContext, model *agentgateway.AgentgatewayModel, parent RouteParentReference) ([]*api.Resource, error) {
	key := modelRouteKey(model, parent)
	route := &api.ModelRoute{
		Key:         key,
		ListenerKey: parent.ListenerKey,
		Match:       &api.ModelRoute_Match{Model: effectiveModelName(model)},
	}
	var resources []*api.Resource

	if model.Spec.Provider != nil {
		backend, err := modelConcreteBackend(ctx, model, parent, nil)
		if err != nil {
			return nil, err
		}
		route.Kind = &api.ModelRoute_ConcreteModel_{
			ConcreteModel: &api.ModelRoute_ConcreteModel{
				ModelVisibility: translateModelVisibility(model.Spec.Visibility),
				Backend:         backendRef(backend.Key),
			},
		}
		resources = append(resources, backendResource(backend))
	} else if model.Spec.VirtualModel != nil {
		virtual, generated, err := translateVirtualModel(ctx, model, parent)
		if err != nil {
			return nil, err
		}
		route.Kind = &api.ModelRoute_VirtualModel_{VirtualModel: virtual}
		resources = append(resources, generated...)
	} else {
		return nil, fmt.Errorf("model must define provider or virtualModel")
	}

	resources = append(resources, &api.Resource{Kind: &api.Resource_ModelRoute{ModelRoute: route}})
	return resources, nil
}

func translateVirtualModel(ctx RouteContext, model *agentgateway.AgentgatewayModel, parent RouteParentReference) (*api.ModelRoute_VirtualModel, []*api.Resource, error) {
	vm := model.Spec.VirtualModel
	switch {
	case vm.Weighted != nil:
		targets := make([]*api.ModelRoute_VirtualModel_Weighted_Target, 0, len(vm.Weighted.Targets))
		for _, target := range vm.Weighted.Targets {
			modelName, err := resolveModelTargetName(ctx, model.Namespace, target.ModelTargetReference)
			if err != nil {
				return nil, nil, err
			}
			targets = append(targets, &api.ModelRoute_VirtualModel_Weighted_Target{
				Model:  modelName,
				Weight: uint32(target.Weight), //nolint:gosec // CEL constrains this to positive int32.
			})
		}
		return &api.ModelRoute_VirtualModel{
			Routing: &api.ModelRoute_VirtualModel_Weighted_{
				Weighted: &api.ModelRoute_VirtualModel_Weighted{Targets: targets},
			},
		}, nil, nil
	case vm.Conditional != nil:
		targets := make([]*api.ModelRoute_VirtualModel_Conditional_Target, 0, len(vm.Conditional.Targets))
		for _, target := range vm.Conditional.Targets {
			modelName, err := resolveModelTargetName(ctx, model.Namespace, target.ModelTargetReference)
			if err != nil {
				return nil, nil, err
			}
			var when *string
			if target.When != nil {
				when = new(string(*target.When))
			}
			targets = append(targets, &api.ModelRoute_VirtualModel_Conditional_Target{Model: modelName, When: when})
		}
		return &api.ModelRoute_VirtualModel{
			Routing: &api.ModelRoute_VirtualModel_Conditional_{
				Conditional: &api.ModelRoute_VirtualModel_Conditional{Targets: targets},
			},
		}, nil, nil
	case vm.Failover != nil:
		backend, err := modelFailoverBackend(ctx, model, parent)
		if err != nil {
			return nil, nil, err
		}
		return &api.ModelRoute_VirtualModel{
			Routing: &api.ModelRoute_VirtualModel_Failover_{
				Failover: &api.ModelRoute_VirtualModel_Failover{Backend: backendRef(backend.Key)},
			},
		}, []*api.Resource{backendResource(backend)}, nil
	default:
		return nil, nil, fmt.Errorf("virtualModel must define weighted, conditional, or failover")
	}
}

func modelConcreteBackend(ctx RouteContext, model *agentgateway.AgentgatewayModel, parent RouteParentReference, selectedModel *string) (*api.Backend, error) {
	provider, err := translateModelLLMProvider(ctx, model.Namespace, &model.Spec, utils.SingularLLMProviderSubBackendName, selectedModel)
	if err != nil {
		return nil, err
	}
	return &api.Backend{
		Key:  modelBackendKey(model, parent, "backend"),
		Name: plugins.ResourceName(model),
		Kind: &api.Backend_Ai{
			Ai: &api.AIBackend{
				ProviderGroups: []*api.AIBackend_ProviderGroup{{Providers: []*api.AIBackend_Provider{provider}}},
			},
		},
	}, nil
}

func modelFailoverBackend(ctx RouteContext, model *agentgateway.AgentgatewayModel, parent RouteParentReference) (*api.Backend, error) {
	groups := map[int32][]*api.AIBackend_Provider{}
	for _, target := range model.Spec.VirtualModel.Failover.Targets {
		refModel, modelName, err := resolveModelTarget(ctx, model.Namespace, target.ModelTargetReference)
		if err != nil {
			return nil, err
		}
		if refModel.Spec.Provider == nil {
			return nil, fmt.Errorf("failover target %s/%s is not a concrete provider model", model.Namespace, target.ModelRef.Name)
		}
		provider, err := translateModelLLMProvider(ctx, refModel.Namespace, &refModel.Spec, target.ModelRef.Name, new(modelName))
		if err != nil {
			return nil, err
		}
		groups[target.Priority] = append(groups[target.Priority], provider)
	}

	priorities := make([]int32, 0, len(groups))
	for p := range groups {
		priorities = append(priorities, p)
	}
	slices.Sort(priorities)

	backend := &api.AIBackend{}
	for _, priority := range priorities {
		providers := groups[priority]
		slices.SortFunc(providers, func(a, b *api.AIBackend_Provider) int {
			return strings.Compare(a.GetName(), b.GetName())
		})
		backend.ProviderGroups = append(backend.ProviderGroups, &api.AIBackend_ProviderGroup{Providers: providers})
	}

	return &api.Backend{
		Key:            modelBackendKey(model, parent, "failover"),
		Name:           plugins.ResourceName(model),
		Kind:           &api.Backend_Ai{Ai: backend},
		InlinePolicies: modelFailoverBackendPolicies(),
	}, nil
}

func modelFailoverBackendPolicies() []*api.BackendPolicySpec {
	consecutiveFailures := int32(3)
	restoreHealth := 1.0
	return []*api.BackendPolicySpec{{
		Kind: &api.BackendPolicySpec_Health_{
			Health: &api.BackendPolicySpec_Health{
				UnhealthyCondition: "response.code >= 500",
				Eviction: &api.BackendPolicySpec_Eviction{
					Duration:            durationpb.New(time.Minute),
					ConsecutiveFailures: &consecutiveFailures,
					RestoreHealth:       &restoreHealth,
				},
			},
		},
	}}
}

func translateModelLLMProvider(ctx RouteContext, namespace string, model *agentgateway.AgentgatewayModelSpec, providerName string, modelOverride *string) (*api.AIBackend_Provider, error) {
	if err := validateModelBaseURL(model); err != nil {
		return nil, err
	}
	if model.UpstreamOverrides != nil && model.UpstreamOverrides.Model != nil {
		modelOverride = new(string(*model.UpstreamOverrides.Model))
	}
	provider := &api.AIBackend_Provider{Name: providerName}
	if model.UpstreamOverrides != nil && model.UpstreamOverrides.BaseURL != nil {
		provider.BaseUrl = new(string(*model.UpstreamOverrides.BaseURL))
	}
	if model.Provider != nil {
		if preset, ok := modelProviderPreset(*model.Provider); ok {
			provider.ModelOverride = modelOverride
			provider.Provider = &api.AIBackend_Provider_ProviderPreset{ProviderPreset: preset}
			return provider, nil
		}
	}

	llm, err := modelLLMProvider(model)
	if err != nil {
		return nil, err
	}
	if llm == nil {
		return nil, fmt.Errorf("no LLM provider configured")
	}
	if provider.HostOverride == nil && llm.Host != "" {
		provider.HostOverride = &api.AIBackend_HostOverride{
			Host: string(llm.Host),
			Port: ptr.NonEmptyOrDefault(llm.Port, 443),
		}
	}
	if provider.PathOverride == nil && llm.Path != "" {
		provider.PathOverride = new(string(llm.Path))
	}
	if provider.PathPrefix == nil && llm.PathPrefix != "" {
		provider.PathPrefix = new(string(llm.PathPrefix))
	}

	switch {
	case llm.OpenAI != nil:
		provider.Provider = &api.AIBackend_Provider_Openai{Openai: &api.AIBackend_OpenAI{Model: providerModel(modelOverride, llm.OpenAI.Model)}}
	case llm.Azure != nil:
		resourceType := api.AIBackend_OPEN_AI
		if llm.Azure.ResourceType == agentgateway.AzureResourceTypeFoundry {
			resourceType = api.AIBackend_FOUNDRY
		}
		provider.Provider = &api.AIBackend_Provider_Azure{Azure: &api.AIBackend_Azure{
			ResourceName: string(llm.Azure.ResourceName),
			ResourceType: resourceType,
			Model:        providerModel(modelOverride, llm.Azure.Model),
			ApiVersion:   stringPtr(llm.Azure.ApiVersion),
			ProjectName:  stringPtr(llm.Azure.ProjectName),
		}}
	case llm.Anthropic != nil:
		provider.Provider = &api.AIBackend_Provider_Anthropic{Anthropic: &api.AIBackend_Anthropic{Model: providerModel(modelOverride, llm.Anthropic.Model)}}
	case llm.Gemini != nil:
		provider.Provider = &api.AIBackend_Provider_Gemini{Gemini: &api.AIBackend_Gemini{Model: providerModel(modelOverride, llm.Gemini.Model)}}
	case llm.VertexAI != nil:
		provider.Provider = &api.AIBackend_Provider_Vertex{Vertex: &api.AIBackend_Vertex{
			Region:    string(llm.VertexAI.Region),
			Model:     providerModel(modelOverride, llm.VertexAI.Model),
			ProjectId: string(llm.VertexAI.ProjectId),
		}}
	case llm.Bedrock != nil:
		var guardrailIdentifier, guardrailVersion *string
		if llm.Bedrock.Guardrail != nil {
			guardrailIdentifier = new(string(llm.Bedrock.Guardrail.GuardrailIdentifier))
			guardrailVersion = new(string(llm.Bedrock.Guardrail.GuardrailVersion))
		}
		provider.Provider = &api.AIBackend_Provider_Bedrock{Bedrock: &api.AIBackend_Bedrock{
			Model:               providerModel(modelOverride, llm.Bedrock.Model),
			Region:              llm.Bedrock.Region,
			GuardrailIdentifier: guardrailIdentifier,
			GuardrailVersion:    guardrailVersion,
		}}
	case llm.Custom != nil:
		formats, err := translateModelProviderFormats(llm.Custom.Formats)
		if err != nil {
			return nil, err
		}
		provider.Provider = &api.AIBackend_Provider_Custom{Custom: &api.AIBackend_Custom{
			Formats: formats,
			Model:   providerModel(modelOverride, llm.Custom.Model),
		}}
		if llm.Custom.BackendRef != nil {
			ref, err := translateModelCustomProviderBackendRef(ctx, namespace, *llm.Custom.BackendRef)
			if err != nil {
				return nil, err
			}
			provider.ProviderBackend = ref
		}
	default:
		return nil, fmt.Errorf("no supported LLM provider configured")
	}
	return provider, nil
}

func modelLLMProvider(model *agentgateway.AgentgatewayModelSpec) (*agentgateway.LLMProvider, error) {
	if model.Provider == nil {
		return nil, nil
	}
	provider := &agentgateway.LLMProvider{}
	switch *model.Provider {
	case agentgateway.ModelProviderOpenAI:
		provider.OpenAI = &agentgateway.OpenAIConfig{}
	case agentgateway.ModelProviderAzure:
		if model.Azure == nil {
			return nil, fmt.Errorf("azure provider requires azure configuration")
		}
		provider.Azure = &agentgateway.AzureConfig{AzureSettings: *model.Azure}
	case agentgateway.ModelProviderAnthropic:
		provider.Anthropic = &agentgateway.AnthropicConfig{}
	case agentgateway.ModelProviderGemini:
		provider.Gemini = &agentgateway.GeminiConfig{}
	case agentgateway.ModelProviderVertexAI:
		if model.VertexAI == nil {
			return nil, fmt.Errorf("vertexai provider requires vertexai configuration")
		}
		provider.VertexAI = &agentgateway.VertexAIConfig{VertexAISettings: *model.VertexAI}
	case agentgateway.ModelProviderBedrock:
		settings := agentgateway.BedrockSettings{Region: "us-east-1"}
		if model.Bedrock != nil {
			settings = *model.Bedrock
			if settings.Region == "" {
				settings.Region = "us-east-1"
			}
		}
		provider.Bedrock = &agentgateway.BedrockConfig{BedrockSettings: settings}
	case agentgateway.ModelProviderCustom:
		if model.Custom == nil {
			return nil, fmt.Errorf("custom provider requires custom configuration")
		}
		provider.Custom = &agentgateway.CustomProvider{CustomProviderSettings: *model.Custom}
	default:
		return nil, fmt.Errorf("unsupported model provider %q", *model.Provider)
	}
	return provider, nil
}

func validateModelBaseURL(model *agentgateway.AgentgatewayModelSpec) error {
	var baseURL string
	if model.UpstreamOverrides != nil && model.UpstreamOverrides.BaseURL != nil {
		baseURL = string(*model.UpstreamOverrides.BaseURL)
	}
	if model.Provider != nil && *model.Provider == agentgateway.ModelProviderOllama && baseURL == "" {
		return fmt.Errorf("ollama requires upstreamOverrides.baseURL")
	}
	if baseURL == "" {
		return nil
	}

	parsed, err := url.ParseRequestURI(baseURL)
	if err != nil || parsed.Scheme == "" || parsed.Host == "" {
		return fmt.Errorf("upstreamOverrides.baseURL must be an absolute URL")
	}
	if parsed.Scheme != "http" && parsed.Scheme != "https" {
		return fmt.Errorf("upstreamOverrides.baseURL must use http or https")
	}
	if parsed.User != nil || parsed.RawQuery != "" || parsed.Fragment != "" {
		return fmt.Errorf("upstreamOverrides.baseURL cannot include user info, query parameters, or a fragment")
	}

	host := parsed.Hostname()
	if host == "" {
		return fmt.Errorf("upstreamOverrides.baseURL must include a host")
	}
	if strings.EqualFold(host, "localhost") || strings.HasSuffix(strings.ToLower(host), ".localhost") {
		return fmt.Errorf("upstreamOverrides.baseURL cannot target localhost, loopback, or link-local addresses")
	}
	ipHost, _, _ := strings.Cut(host, "%")
	if addr, err := netip.ParseAddr(ipHost); err == nil {
		addr = addr.Unmap()
		if addr.IsLoopback() || addr.IsLinkLocalUnicast() {
			return fmt.Errorf("upstreamOverrides.baseURL cannot target localhost, loopback, or link-local addresses")
		}
	} else if strings.Trim(host, ".0123456789") == "" || strings.HasPrefix(strings.ToLower(host), "0x") {
		return fmt.Errorf("upstreamOverrides.baseURL cannot use an ambiguous IP address")
	}
	return nil
}

func modelProviderPreset(provider agentgateway.ModelProvider) (api.AIBackend_ProviderPreset, bool) {
	switch provider {
	case agentgateway.ModelProviderCohere:
		return api.AIBackend_PROVIDER_PRESET_COHERE, true
	case agentgateway.ModelProviderOllama:
		return api.AIBackend_PROVIDER_PRESET_OLLAMA, true
	case agentgateway.ModelProviderBaseten:
		return api.AIBackend_PROVIDER_PRESET_BASETEN, true
	case agentgateway.ModelProviderCerebras:
		return api.AIBackend_PROVIDER_PRESET_CEREBRAS, true
	case agentgateway.ModelProviderDeepinfra:
		return api.AIBackend_PROVIDER_PRESET_DEEPINFRA, true
	case agentgateway.ModelProviderDeepseek:
		return api.AIBackend_PROVIDER_PRESET_DEEPSEEK, true
	case agentgateway.ModelProviderGroq:
		return api.AIBackend_PROVIDER_PRESET_GROQ, true
	case agentgateway.ModelProviderHuggingface:
		return api.AIBackend_PROVIDER_PRESET_HUGGINGFACE, true
	case agentgateway.ModelProviderMistral:
		return api.AIBackend_PROVIDER_PRESET_MISTRAL, true
	case agentgateway.ModelProviderOpenrouter:
		return api.AIBackend_PROVIDER_PRESET_OPENROUTER, true
	case agentgateway.ModelProviderTogetherAI:
		return api.AIBackend_PROVIDER_PRESET_TOGETHERAI, true
	case agentgateway.ModelProviderXAI:
		return api.AIBackend_PROVIDER_PRESET_XAI, true
	case agentgateway.ModelProviderFireworks:
		return api.AIBackend_PROVIDER_PRESET_FIREWORKS, true
	default:
		return api.AIBackend_PROVIDER_PRESET_UNSPECIFIED, false
	}
}

func translateModelProviderFormats(formats []agentgateway.ProviderFormatConfig) ([]*api.AIBackend_ProviderFormatConfig, error) {
	out := make([]*api.AIBackend_ProviderFormatConfig, 0, len(formats))
	for _, format := range formats {
		protoFormat, err := translateModelProviderFormat(format.Type)
		if err != nil {
			return nil, err
		}
		var path *string
		if format.Path != "" {
			path = new(string(format.Path))
		}
		out = append(out, &api.AIBackend_ProviderFormatConfig{Format: protoFormat, Path: path})
	}
	return out, nil
}

func translateModelProviderFormat(format agentgateway.ProviderFormat) (api.AIBackend_ProviderFormat, error) {
	switch format {
	case agentgateway.ProviderFormatCompletions:
		return api.AIBackend_COMPLETIONS, nil
	case agentgateway.ProviderFormatMessages:
		return api.AIBackend_MESSAGES, nil
	case agentgateway.ProviderFormatResponses:
		return api.AIBackend_RESPONSES, nil
	case agentgateway.ProviderFormatEmbeddings:
		return api.AIBackend_EMBEDDINGS, nil
	case agentgateway.ProviderFormatAnthropicTokenCount:
		return api.AIBackend_ANTHROPIC_TOKEN_COUNT, nil
	case agentgateway.ProviderFormatRealtime:
		return api.AIBackend_REALTIME, nil
	case agentgateway.ProviderFormatRerank:
		return api.AIBackend_RERANK, nil
	default:
		return api.AIBackend_PROVIDER_FORMAT_UNSPECIFIED, fmt.Errorf("unsupported custom provider format %q", format)
	}
}

func translateModelCustomProviderBackendRef(ctx RouteContext, namespace string, ref agentgateway.LocalBackendObjectReference) (*api.BackendReference, error) {
	kind := ptr.OrDefault(ref.Kind, wellknown.ServiceKind)
	group := ""
	if ref.Group != nil {
		group = *ref.Group
	}
	gk := schema.GroupKind{Group: group, Kind: kind}
	switch gk {
	case wellknown.ServiceGVK.GroupKind(), wellknown.InferencePoolGVK.GroupKind():
	default:
		return nil, fmt.Errorf("custom provider backendRef may target only Service or InferencePool")
	}
	var port *gwv1.PortNumber
	if ref.Port != nil {
		port = new(gwv1.PortNumber(*ref.Port))
	}
	return ctx.References.RouteBackend(ctx.Krt, namespace, gk, gwv1.ObjectName(ref.Name), nil, port)
}

func resolveModelTargetName(ctx RouteContext, namespace string, target agentgateway.ModelTargetReference) (string, error) {
	_, modelName, err := resolveModelTarget(ctx, namespace, target)
	return modelName, err
}

func resolveModelTarget(ctx RouteContext, namespace string, target agentgateway.ModelTargetReference) (*agentgateway.AgentgatewayModel, string, error) {
	var ref *agentgateway.AgentgatewayModel
	for _, candidate := range krt.Fetch(ctx.Krt, ctx.Models, krt.FilterIndex(ctx.ModelsByNamespace, namespace)) {
		if candidate.Name == target.ModelRef.Name {
			ref = candidate
			break
		}
	}
	if ref == nil {
		return nil, "", fmt.Errorf("model target %s/%s not found", namespace, target.ModelRef.Name)
	}
	if target.Model != nil {
		return ref, string(*target.Model), nil
	}
	return ref, effectiveModelName(ref), nil
}

func routeConditionMessage(condition *reporter.RouteCondition) string {
	if condition == nil {
		return ""
	}
	return condition.Message
}

func providerModel[T ~string](override *string, configured *T) *string {
	if override != nil {
		return override
	}
	if configured == nil {
		return nil
	}
	return new(string(*configured))
}

func stringPtr[T ~string](v *T) *string {
	if v == nil {
		return nil
	}
	return new(string(*v))
}

func effectiveModelName(model *agentgateway.AgentgatewayModel) string {
	if model.Spec.Match != nil && model.Spec.Match.Model != nil {
		return string(*model.Spec.Match.Model)
	}
	return model.Name
}

func translateModelVisibility(visibility agentgateway.ModelVisibility) api.ModelRoute_ConcreteModel_ModelVisibility {
	if visibility == agentgateway.ModelVisibilityInternal {
		return api.ModelRoute_ConcreteModel_INTERNAL
	}
	return api.ModelRoute_ConcreteModel_PUBLIC
}

func modelRouteKey(model *agentgateway.AgentgatewayModel, parent RouteParentReference) string {
	return config.NamespacedName(model).String() + routeKeySuffix(parent)
}

func modelBackendKey(model *agentgateway.AgentgatewayModel, parent RouteParentReference, target string) string {
	return utils.InternalBackendKey(model.Namespace, model.Name, target+routeKeySuffix(parent))
}

func backendRef(key string) *api.BackendReference {
	return &api.BackendReference{Kind: &api.BackendReference_Backend{Backend: key}}
}

func backendResource(backend *api.Backend) *api.Resource {
	return &api.Resource{Kind: &api.Resource_Backend{Backend: backend}}
}

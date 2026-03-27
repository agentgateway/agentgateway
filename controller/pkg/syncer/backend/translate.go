package agentgatewaybackend

import (
	"errors"
	"fmt"

	"istio.io/istio/pilot/pkg/model/kstatus"
	"istio.io/istio/pkg/config"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	agwir "github.com/agentgateway/agentgateway/controller/pkg/agentgateway/ir"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/translator"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

var logger = logging.New("agentgateway/backend")

// NewBackendPlugin creates a new plugin for AgentgatewayBackends
func NewBackendPlugin(agw *plugins.AgwCollections) plugins.AgwPlugin {
	return plugins.AgwPlugin{
		ContributesBackends: map[schema.GroupKind]plugins.BackendPlugin{
			wellknown.AgentgatewayBackendGVK.GroupKind(): {
				BuildReferences: func() krt.Collection[*plugins.PolicyAttachment] {
					return krt.NewManyCollection(agw.Backends, func(ctx krt.HandlerContext, backend *agentgateway.AgentgatewayBackend) []*plugins.PolicyAttachment {
						return BuildAgwBackendReferences(backend)
					}, agw.KrtOpts.ToOptions("BackendPolicyAttachments")...)
				},
				Build: func(input plugins.PolicyPluginInput) (krt.StatusCollection[controllers.Object, any], krt.Collection[agwir.AgwResource]) {
					status, col := krt.NewStatusManyCollection(agw.Backends, func(ctx krt.HandlerContext, backend *agentgateway.AgentgatewayBackend) (
						*agentgateway.AgentgatewayBackendStatus,
						[]agwir.AgwResource,
					) {
						pc := plugins.PolicyCtx{
							Krt:         ctx,
							Collections: agw,
							References:  input.References,
						}
						return TranslateAgwBackend(pc, backend, input.References)
					}, agw.KrtOpts.ToOptions("Backends")...)
					return plugins.ConvertStatusCollection(status), col
				},
			},
		},
	}
}

func BuildAgwBackendReferences(
	backend *agentgateway.AgentgatewayBackend,
) []*plugins.PolicyAttachment {
	var attachments []*plugins.PolicyAttachment
	self := utils.TypedNamespacedName{
		NamespacedName: types.NamespacedName{Namespace: backend.Namespace, Name: backend.Name},
		Kind:           wellknown.AgentgatewayBackendGVK.Kind,
	}
	app := func(ref gwv1.BackendObjectReference) {
		attachments = append(attachments, &plugins.PolicyAttachment{
			Target: self,
			Backend: utils.TypedNamespacedName{
				NamespacedName: types.NamespacedName{Namespace: plugins.DefaultString(ref.Namespace, backend.Namespace), Name: string(ref.Name)},
				Kind:           plugins.DefaultString(ref.Kind, wellknown.ServiceKind),
			},
			Source: self,
		})
	}
	if backend.Spec.Policies != nil {
		plugins.BackendReferencesFromBackendPolicy(backend.Spec.Policies, app)
	}
	if ai := backend.Spec.AI; ai != nil {
		for _, r := range ai.PriorityGroups {
			for _, p := range r.Providers {
				if p.Policies != nil {
					plugins.BackendReferencesFromBackendPolicy(&agentgateway.BackendFull{
						BackendSimple: p.Policies.BackendSimple,
						AI:            p.Policies.AI,
						MCP:           nil,
					}, app)
				}
			}
		}
	}
	if mcp := backend.Spec.MCP; mcp != nil {
		for _, r := range mcp.Targets {
			if r.Static != nil && r.Static.Policies != nil {
				p := r.Static.Policies
				plugins.BackendReferencesFromBackendPolicy(&agentgateway.BackendFull{
					BackendSimple: p.BackendSimple,
					MCP:           p.MCP,
					AI:            nil,
				}, app)
			}
		}
	}
	return attachments
}

// BuildAgwBackend translates a Backend to an AgwBackend
func BuildAgwBackend(
	ctx plugins.PolicyCtx,
	backend *agentgateway.AgentgatewayBackend,
) ([]*api.Backend, error) {
	errs := []error{}
	pols, err := TranslateBackendPolicies(ctx, backend.Namespace, backend.Spec.Policies)
	if err != nil {
		errs = append(errs, err)
	}

	if b := backend.Spec.Static; b != nil {
		return []*api.Backend{{
			Key:  backend.Namespace + "/" + backend.Name,
			Name: plugins.ResourceName(backend),
			Kind: &api.Backend_Static{
				Static: &api.StaticBackend{
					Host: b.Host,
					Port: b.Port,
				},
			},
			InlinePolicies: pols,
		}}, errors.Join(errs...)
	}
	if b := backend.Spec.DynamicForwardProxy; b != nil {
		return []*api.Backend{{
			Key:  backend.Namespace + "/" + backend.Name,
			Name: plugins.ResourceName(backend),
			Kind: &api.Backend_Dynamic{
				Dynamic: &api.DynamicForwardProxy{},
			},
			InlinePolicies: pols,
		}}, errors.Join(errs...)
	}
	if b := backend.Spec.MCP; b != nil {
		be, err := TranslateMCPBackends(ctx, backend, pols)
		return be, errors.Join(append(errs, err)...)
	}
	if b := backend.Spec.AI; b != nil {
		be, err := translateAIBackends(ctx, backend, pols)
		if err != nil {
			return nil, errors.Join(append(errs, err)...)
		}
		return []*api.Backend{be}, errors.Join(errs...)
	}
	if b := backend.Spec.Aws; b != nil {
		be, err := translateAwsBackends(backend, pols)
		if err != nil {
			return nil, errors.Join(append(errs, err)...)
		}
		return be, errors.Join(errs...)
	}
	return nil, errors.Join(append(errs, errors.New("unknown backend"))...)
}

func TranslateAgwBackend(
	ctx plugins.PolicyCtx,
	backend *agentgateway.AgentgatewayBackend,
	references plugins.ReferenceIndex,
) (*agentgateway.AgentgatewayBackendStatus, []agwir.AgwResource) {
	var results []agwir.AgwResource
	backends, err := BuildAgwBackend(ctx, backend)
	if err != nil {
		logger.Error("failed to translate backend", "backend", backend.Name, "namespace", backend.Namespace, "err", err)
		return &agentgateway.AgentgatewayBackendStatus{
			Conditions: kstatus.UpdateConditionIfChanged(backend.Status.Conditions, metav1.Condition{
				Type:               "Accepted",
				Status:             metav1.ConditionFalse,
				Reason:             "TranslationError",
				Message:            fmt.Sprintf("failed to translate backend: %v", err),
				ObservedGeneration: backend.Generation,
				LastTransitionTime: metav1.Now(),
			}),
		}, results
	}

	gtws := references.LookupGatewaysForBackend(ctx.Krt, utils.TypedNamespacedName{
		NamespacedName: config.NamespacedName(backend),
		Kind:           wellknown.AgentgatewayBackendGVK.Kind,
	})
	// handle all backends created as an MCPBackend backend may create multiple backends
	for gateway := range gtws {
		for _, backend := range backends {
			logger.Debug("creating backend", "backend", backend.Name)
			resourceWrapper := translator.ToResourceForGateway(gateway, &api.Resource{
				Kind: &api.Resource_Backend{
					Backend: backend,
				},
			})
			results = append(results, resourceWrapper)
		}
	}

	return &agentgateway.AgentgatewayBackendStatus{
		Conditions: kstatus.UpdateConditionIfChanged(backend.Status.Conditions, metav1.Condition{
			Type:               "Accepted",
			Status:             metav1.ConditionTrue,
			Reason:             "Accepted",
			Message:            "Backend successfully accepted",
			ObservedGeneration: backend.Generation,
			LastTransitionTime: metav1.Now(),
		}),
	}, results
}

func TranslateMCPBackends(ctx plugins.PolicyCtx, be *agentgateway.AgentgatewayBackend, inlinePolicies []*api.BackendPolicySpec) ([]*api.Backend, error) {
	mcp := be.Spec.MCP
	var mcpTargets []*api.MCPTarget
	var backends []*api.Backend
	var errs []error
	for _, target := range mcp.Targets {
		if s := target.Static; s != nil {
			if s.BackendRef != nil {
				serviceHostname, err := ResolveMCPBackendRefHost(ctx, be.Namespace, s.BackendRef)
				if err != nil {
					return nil, err
				}
				mcpTarget := &api.MCPTarget{
					Name: string(target.Name),
					Backend: &api.BackendReference{
						Kind: &api.BackendReference_Service_{
							Service: &api.BackendReference_Service{
								Hostname:  serviceHostname,
								Namespace: be.Namespace,
							},
						},
						Port: uint32(s.Port), //nolint:gosec // G115: validated by the CRD schema
					},
					Path: ptr.OrEmpty(s.Path),
				}

				switch ptr.OrEmpty(s.Protocol) {
				case agentgateway.MCPProtocolSSE:
					mcpTarget.Protocol = api.MCPTarget_SSE
				case agentgateway.MCPProtocolStreamableHTTP:
					mcpTarget.Protocol = api.MCPTarget_STREAMABLE_HTTP
				}

				mcpTargets = append(mcpTargets, mcpTarget)
				continue
			}

			staticBackendRef := utils.InternalMCPStaticBackendName(be.Namespace, be.Name, string(target.Name))
			pol, err := TranslateMCPBackendPolicies(ctx, be.Namespace, s.Policies)
			if err != nil {
				logger.Error("failed to translate static MCP backend policies", "err", err)
				errs = append(errs, err)
			}
			staticBackend := &api.Backend{
				Key:  staticBackendRef,
				Name: plugins.ResourceName(be),
				Kind: &api.Backend_Static{
					Static: &api.StaticBackend{
						Host: ptr.OrEmpty(s.Host),
						Port: s.Port,
					},
				},
				InlinePolicies: pol,
			}
			backends = append(backends, staticBackend)

			mcpTarget := &api.MCPTarget{
				Name: string(target.Name),
				Backend: &api.BackendReference{
					Kind: &api.BackendReference_Backend{
						Backend: staticBackendRef,
					},
				},
				Path: ptr.OrEmpty(s.Path),
			}

			switch ptr.OrEmpty(s.Protocol) {
			case agentgateway.MCPProtocolSSE:
				mcpTarget.Protocol = api.MCPTarget_SSE
			case agentgateway.MCPProtocolStreamableHTTP:
				mcpTarget.Protocol = api.MCPTarget_STREAMABLE_HTTP
			}

			mcpTargets = append(mcpTargets, mcpTarget)
		} else if s := target.Selector; s != nil {
			targets, err := TranslateMCPSelectorTargets(ctx, be.Namespace, target.Selector)
			if err != nil {
				return nil, err
			}
			mcpTargets = append(mcpTargets, targets...)
		}
	}
	// defaults to stateful session routing
	sessionRouting := api.MCPBackend_STATEFUL
	if mcp.SessionRouting == agentgateway.Stateless {
		sessionRouting = api.MCPBackend_STATELESS
	}
	failureMode := api.MCPBackend_FAIL_CLOSED
	if mcp.FailureMode == agentgateway.FailOpen {
		failureMode = api.MCPBackend_FAIL_OPEN
	}
	mcpBackend := &api.Backend{
		Key:  be.Namespace + "/" + be.Name,
		Name: plugins.ResourceName(be),
		Kind: &api.Backend_Mcp{
			Mcp: &api.MCPBackend{
				Targets:      mcpTargets,
				StatefulMode: sessionRouting,
				FailureMode:  failureMode,
			},
		},
		InlinePolicies: inlinePolicies,
	}
	backends = append(backends, mcpBackend)
	return backends, errors.Join(errs...)
}

func translateAIBackends(ctx plugins.PolicyCtx, be *agentgateway.AgentgatewayBackend, inlinePolicies []*api.BackendPolicySpec) (*api.Backend, error) {
	ai := be.Spec.AI
	var errs []error

	aiBackend := &api.AIBackend{}
	if llm := ai.LLM; llm != nil {
		provider, err := translateLLMProvider(llm, utils.SingularLLMProviderSubBackendName)
		if err != nil {
			return nil, fmt.Errorf("failed to translate LLM provider: %w", err)
		}

		aiBackend.ProviderGroups = []*api.AIBackend_ProviderGroup{{
			Providers: []*api.AIBackend_Provider{provider},
		}}
	} else {
		for _, group := range ai.PriorityGroups {
			providerGroup := &api.AIBackend_ProviderGroup{}

			for _, provider := range group.Providers {
				tp, err := translateLLMProvider(&provider.LLMProvider, string(provider.Name))
				if err != nil {
					return nil, fmt.Errorf("failed to translate LLM provider: %w", err)
				}
				pol, err := translateAIBackendPolicies(ctx, be.Namespace, provider.Policies)
				if err != nil {
					// TODO: bubble this up to a status message without blocking the entire Backend
					logger.Warn("failed to translate AI backend policies", "err", err)
				}
				tp.InlinePolicies = pol

				providerGroup.Providers = append(providerGroup.Providers, tp)
			}
			if len(providerGroup.Providers) > 0 {
				aiBackend.ProviderGroups = append(aiBackend.ProviderGroups, providerGroup)
			}
		}
	}

	backendName := utils.InternalBackendKey(be.Namespace, be.Name, "")
	backend := &api.Backend{
		Key:  backendName,
		Name: plugins.ResourceName(be),
		Kind: &api.Backend_Ai{
			Ai: aiBackend,
		},
		InlinePolicies: inlinePolicies,
	}

	return backend, errors.Join(errs...)
}

func TranslateBackendPolicies(
	ctx plugins.PolicyCtx,
	namespace string,
	policies *agentgateway.BackendFull,
) ([]*api.BackendPolicySpec, error) {
	if policies == nil {
		return nil, nil
	}
	return plugins.TranslateInlineBackendPolicy(ctx, namespace, policies)
}

func TranslateMCPBackendPolicies(
	ctx plugins.PolicyCtx,
	namespace string, policies *agentgateway.BackendWithMCP,
) ([]*api.BackendPolicySpec, error) {
	if policies == nil {
		return nil, nil
	}
	return TranslateBackendPolicies(ctx, namespace, &agentgateway.BackendFull{
		BackendSimple: policies.BackendSimple,
		MCP:           policies.MCP,
	})
}

func translateAIBackendPolicies(
	ctx plugins.PolicyCtx,
	namespace string, policies *agentgateway.BackendWithAI,
) ([]*api.BackendPolicySpec, error) {
	if policies == nil {
		return nil, nil
	}
	return TranslateBackendPolicies(ctx, namespace, &agentgateway.BackendFull{
		BackendSimple: policies.BackendSimple,
		AI:            policies.AI,
	})
}

func translateLLMProvider(llm *agentgateway.LLMProvider, providerName string) (*api.AIBackend_Provider, error) {
	provider := &api.AIBackend_Provider{
		Name: providerName,
	}

	if llm.Host != "" {
		provider.HostOverride = &api.AIBackend_HostOverride{
			Host: llm.Host,
			Port: ptr.NonEmptyOrDefault(llm.Port, 443), // Port is required when Host is set (CEL validated)
		}
	}

	if llm.Path != "" {
		provider.PathOverride = &llm.Path
	}

	if llm.PathPrefix != "" {
		provider.PathPrefix = &llm.PathPrefix
	}

	// Extract auth token and model based on provider
	if llm.OpenAI != nil {
		provider.Provider = &api.AIBackend_Provider_Openai{
			Openai: &api.AIBackend_OpenAI{
				Model: llm.OpenAI.Model,
			},
		}
	} else if llm.AzureOpenAI != nil {
		provider.Provider = &api.AIBackend_Provider_Azureopenai{
			Azureopenai: &api.AIBackend_AzureOpenAI{
				Host:       llm.AzureOpenAI.Endpoint,
				Model:      llm.AzureOpenAI.DeploymentName,
				ApiVersion: llm.AzureOpenAI.ApiVersion,
			},
		}
	} else if llm.Anthropic != nil {
		provider.Provider = &api.AIBackend_Provider_Anthropic{
			Anthropic: &api.AIBackend_Anthropic{
				Model: llm.Anthropic.Model,
			},
		}
	} else if llm.Gemini != nil {
		provider.Provider = &api.AIBackend_Provider_Gemini{
			Gemini: &api.AIBackend_Gemini{
				Model: llm.Gemini.Model,
			},
		}
	} else if llm.VertexAI != nil {
		// TODO: publisher?
		provider.Provider = &api.AIBackend_Provider_Vertex{
			Vertex: &api.AIBackend_Vertex{
				Region:    llm.VertexAI.Region,
				Model:     llm.VertexAI.Model,
				ProjectId: llm.VertexAI.ProjectId,
			},
		}
	} else if llm.Bedrock != nil {
		region := llm.Bedrock.Region
		var guardrailIdentifier, guardrailVersion *string
		if llm.Bedrock.Guardrail != nil {
			guardrailIdentifier = &llm.Bedrock.Guardrail.GuardrailIdentifier
			guardrailVersion = &llm.Bedrock.Guardrail.GuardrailVersion
		}

		provider.Provider = &api.AIBackend_Provider_Bedrock{
			Bedrock: &api.AIBackend_Bedrock{
				Model:               llm.Bedrock.Model,
				Region:              region,
				GuardrailIdentifier: guardrailIdentifier,
				GuardrailVersion:    guardrailVersion,
			},
		}
	} else {
		return nil, fmt.Errorf("no supported LLM provider configured")
	}

	return provider, nil
}

func translateAwsBackends(
	be *agentgateway.AgentgatewayBackend,
	inlinePolicies []*api.BackendPolicySpec,
) ([]*api.Backend, error) {
	aws := be.Spec.Aws
	if aws == nil || aws.AgentCore == nil {
		return nil, errors.New("AwsBackend: agentCore is required")
	}
	ac := aws.AgentCore
	awsBackend := &api.AwsBackend{
		Service: &api.AwsBackend_AgentCore{
			AgentCore: &api.AwsAgentCoreBackend{
				AgentRuntimeArn: ac.AgentRuntimeArn,
				Qualifier:       ac.Qualifier,
			},
		},
	}
	return []*api.Backend{{
		Key:  be.Namespace + "/" + be.Name,
		Name: plugins.ResourceName(be),
		Kind: &api.Backend_Aws{
			Aws: awsBackend,
		},
		InlinePolicies: inlinePolicies,
	}}, nil
}

func toMCPProtocol(appProtocol string) api.MCPTarget_Protocol {
	switch appProtocol {
	case mcpProtocol, mcpProtocolLegacy:
		return api.MCPTarget_STREAMABLE_HTTP

	case mcpProtocolSSE, mcpProtocolSSELegacy:
		return api.MCPTarget_SSE

	default:
		// should never happen since this function is only invoked for valid MCPBackend protocols
		return api.MCPTarget_UNDEFINED
	}
}

package agentgatewaybackend

import (
	"errors"
	"fmt"

	"github.com/agentgateway/agentgateway/go/api"
	"istio.io/istio/pilot/pkg/model/kstatus"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/labels"

	apiannotations "github.com/kgateway-dev/kgateway/v2/api/annotations"
	"github.com/kgateway-dev/kgateway/v2/api/v1alpha1/agentgateway"
	agwir "github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/ir"
	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/plugins"
	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/translator"
	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/utils"
	"github.com/kgateway-dev/kgateway/v2/pkg/logging"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils"
)

var logger = logging.New("agentgateway/backend")

// BuildAgwBackend translates a Backend to an AgwBackend
func BuildAgwBackend(
	ctx plugins.PolicyCtx,
	backend *agentgateway.AgentgatewayBackend,
) ([]*api.Backend, error) {
	errs := []error{}
	pols, err := translateBackendPolicies(ctx, backend.Namespace, backend.Spec.Policies)
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
		be, err := translateMCPBackends(ctx, backend, pols)
		return be, errors.Join(append(errs, err)...)
	}
	if b := backend.Spec.AI; b != nil {
		be, err := translateAIBackends(ctx, backend, pols)
		if err != nil {
			return nil, errors.Join(append(errs, err)...)
		}
		return []*api.Backend{be}, errors.Join(errs...)
	}
	return nil, errors.Join(append(errs, errors.New("unknown backend"))...)
}

func TranslateAgwBackend(
	ctx plugins.PolicyCtx,
	backend *agentgateway.AgentgatewayBackend,
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

	// handle all backends created as an MCPBackend backend may create multiple backends
	for _, backend := range backends {
		logger.Debug("creating backend", "backend", backend.Name)
		resourceWrapper := translator.ToResourceGlobal(&api.Resource{
			Kind: &api.Resource_Backend{
				Backend: backend,
			},
		})
		results = append(results, resourceWrapper)
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

func translateMCPBackends(ctx plugins.PolicyCtx, be *agentgateway.AgentgatewayBackend, inlinePolicies []*api.BackendPolicySpec) ([]*api.Backend, error) {
	mcp := be.Spec.MCP
	var mcpTargets []*api.MCPTarget
	var backends []*api.Backend
	var errs []error
	for _, target := range mcp.Targets {
		if s := target.Static; s != nil {
			staticBackendRef := utils.InternalMCPStaticBackendName(be.Namespace, be.Name, string(target.Name))
			pol, err := translateMCPBackendPolicies(ctx, be.Namespace, s.Policies)
			if err != nil {
				logger.Error("failed to translate static MCP backend policies", "err", err)
				errs = append(errs, err)
			}
			staticBackend := &api.Backend{
				Key:  staticBackendRef,
				Name: plugins.ResourceName(be),
				Kind: &api.Backend_Static{
					Static: &api.StaticBackend{
						Host: target.Static.Host,
						Port: target.Static.Port,
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
				Path: ptr.OrEmpty(target.Static.Path),
			}

			switch ptr.OrEmpty(target.Static.Protocol) {
			case agentgateway.MCPProtocolSSE:
				mcpTarget.Protocol = api.MCPTarget_SSE
			case agentgateway.MCPProtocolStreamableHTTP:
				mcpTarget.Protocol = api.MCPTarget_STREAMABLE_HTTP
			}

			mcpTargets = append(mcpTargets, mcpTarget)
		} else if s := target.Selector; s != nil {
			// Krt only allows 1 filter per type, so we build a composite filter here
			generic := func(svc any) bool {
				return true
			}
			var nsFilter krt.FetchOption
			addFilter := func(nf func(svc any) bool) {
				og := generic
				generic = func(svc any) bool {
					return nf(svc) && og(svc)
				}
			}

			// Apply service filter
			if s.Service != nil {
				serviceSelector, err := metav1.LabelSelectorAsSelector(target.Selector.Service)
				if err != nil {
					return nil, fmt.Errorf("invalid service selector: %w", err)
				}
				if !serviceSelector.Empty() {
					addFilter(func(obj any) bool {
						service := obj.(*corev1.Service)
						return serviceSelector.Matches(labels.Set(service.Labels))
					})
				}
			}

			// Apply namespace selector
			if target.Selector.Namespace != nil {
				namespaceSelector, err := metav1.LabelSelectorAsSelector(target.Selector.Namespace)
				if err != nil {
					return nil, fmt.Errorf("invalid namespace selector: %w", err)
				}
				if !namespaceSelector.Empty() {
					// Get all namespaces and find those matching the selector
					allNamespaces := krt.Fetch(ctx.Krt, ctx.Collections.Namespaces)
					matchingNamespaces := make(map[string]bool)
					for _, ns := range allNamespaces {
						if namespaceSelector.Matches(labels.Set(ns.Labels)) {
							matchingNamespaces[ns.Name] = true
						}
					}
					// Filter services to only those in matching namespaces
					addFilter(func(obj any) bool {
						service := obj.(*corev1.Service)
						return matchingNamespaces[service.Namespace]
					})
				}
			} else {
				// If no namespace selector, limit to same namespace as backend
				nsFilter = krt.FilterIndex(ctx.Collections.ServicesByNamespace, be.Namespace)
			}

			opts := []krt.FetchOption{krt.FilterGeneric(generic)}
			if nsFilter != nil {
				opts = append(opts, nsFilter)
			}
			matchingServices := krt.Fetch(ctx.Krt, ctx.Collections.Services, opts...)
			for _, service := range matchingServices {
				for _, port := range service.Spec.Ports {
					appProtocol := ptr.OrEmpty(port.AppProtocol)
					if appProtocol != mcpProtocol && appProtocol != mcpProtocolSSE {
						// not a valid MCPBackend protocol
						continue
					}
					targetName := service.Name + fmt.Sprintf("-%d", port.Port)
					if port.Name != "" {
						targetName = service.Name + "-" + port.Name
					}

					svcHostname := kubeutils.ServiceFQDN(service.ObjectMeta)

					mcpTarget := &api.MCPTarget{
						Name: targetName,
						Backend: &api.BackendReference{
							Kind: &api.BackendReference_Service_{
								Service: &api.BackendReference_Service{
									Hostname:  svcHostname,
									Namespace: service.Namespace,
								},
							},
							Port: uint32(port.Port), //nolint:gosec // G115: Kubernetes service ports are always positive
						},
						Protocol: toMCPProtocol(appProtocol),
						Path:     service.Annotations[apiannotations.MCPServiceHTTPPath],
					}

					mcpTargets = append(mcpTargets, mcpTarget)
				}
			}
		}
	}
	// defaults to stateful session routing
	sessionRouting := api.MCPBackend_STATEFUL
	if mcp.SessionRouting == agentgateway.Stateless {
		sessionRouting = api.MCPBackend_STATELESS
	}
	mcpBackend := &api.Backend{
		Key:  be.Namespace + "/" + be.Name,
		Name: plugins.ResourceName(be),
		Kind: &api.Backend_Mcp{
			Mcp: &api.MCPBackend{
				Targets:      mcpTargets,
				StatefulMode: sessionRouting,
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

func translateBackendPolicies(
	ctx plugins.PolicyCtx,
	namespace string,
	policies *agentgateway.BackendFull) ([]*api.BackendPolicySpec, error) {
	if policies == nil {
		return nil, nil
	}
	return plugins.TranslateInlineBackendPolicy(ctx, namespace, policies)
}

func translateMCPBackendPolicies(
	ctx plugins.PolicyCtx,
	namespace string, policies *agentgateway.BackendWithMCP) ([]*api.BackendPolicySpec, error) {
	if policies == nil {
		return nil, nil
	}
	return translateBackendPolicies(ctx, namespace, &agentgateway.BackendFull{
		BackendSimple: policies.BackendSimple,
		MCP:           policies.MCP,
	})
}

func translateAIBackendPolicies(
	ctx plugins.PolicyCtx,
	namespace string, policies *agentgateway.BackendWithAI) ([]*api.BackendPolicySpec, error) {
	if policies == nil {
		return nil, nil
	}
	return translateBackendPolicies(ctx, namespace, &agentgateway.BackendFull{
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

func toMCPProtocol(appProtocol string) api.MCPTarget_Protocol {
	switch appProtocol {
	case mcpProtocol:
		return api.MCPTarget_STREAMABLE_HTTP

	case mcpProtocolSSE:
		return api.MCPTarget_SSE

	default:
		// should never happen since this function is only invoked for valid MCPBackend protocols
		return api.MCPTarget_UNDEFINED
	}
}

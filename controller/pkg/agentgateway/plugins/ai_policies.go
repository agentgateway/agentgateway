package plugins

import (
	"fmt"
	"log/slog"

	"github.com/agentgateway/agentgateway/go/api"
	"istio.io/istio/pkg/ptr"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/api/v1alpha1/agentgateway"
)

func processRequestGuard(ctx PolicyCtx, namespace string, reqs []agentgateway.PromptguardRequest) ([]*api.BackendPolicySpec_Ai_RequestGuard, error) {
	var res []*api.BackendPolicySpec_Ai_RequestGuard
	for _, req := range reqs {
		pgReq := &api.BackendPolicySpec_Ai_RequestGuard{}
		if req.Webhook != nil {
			wh, err := processWebhook(ctx, namespace, req.Webhook)
			if err != nil {
				return nil, err
			}
			pgReq.Kind = &api.BackendPolicySpec_Ai_RequestGuard_Webhook{
				Webhook: wh,
			}
		} else if req.Regex != nil {
			pgReq.Kind = &api.BackendPolicySpec_Ai_RequestGuard_Regex{
				Regex: processRegex(req.Regex),
			}
		} else if req.OpenAIModeration != nil {
			pgReq.Kind = &api.BackendPolicySpec_Ai_RequestGuard_OpenaiModeration{
				OpenaiModeration: processModeration(ctx, namespace, req.OpenAIModeration),
			}
		}

		if req.CustomResponse != nil {
			pgReq.Rejection = &api.BackendPolicySpec_Ai_RequestRejection{
				Body:   []byte(req.CustomResponse.Message),
				Status: uint32(req.CustomResponse.StatusCode), // nolint:gosec // G115: kubebuilder validation ensures safe for uint32
			}
		}
		res = append(res, pgReq)
	}

	return res, nil
}

func processResponseGuard(ctx PolicyCtx, namespace string, resps []agentgateway.PromptguardResponse) ([]*api.BackendPolicySpec_Ai_ResponseGuard, error) {
	var res []*api.BackendPolicySpec_Ai_ResponseGuard
	for _, req := range resps {
		pgReq := &api.BackendPolicySpec_Ai_ResponseGuard{}
		if req.Webhook != nil {
			wh, err := processWebhook(ctx, namespace, req.Webhook)
			if err != nil {
				return nil, err
			}
			pgReq.Kind = &api.BackendPolicySpec_Ai_ResponseGuard_Webhook{
				Webhook: wh,
			}
		} else if req.Regex != nil {
			pgReq.Kind = &api.BackendPolicySpec_Ai_ResponseGuard_Regex{
				Regex: processRegex(req.Regex),
			}
		}

		if req.CustomResponse != nil {
			pgReq.Rejection = &api.BackendPolicySpec_Ai_RequestRejection{
				Body:   []byte(req.CustomResponse.Message),
				Status: uint32(req.CustomResponse.StatusCode), // nolint:gosec // G115: kubebuilder validation ensures safe for uint32
			}
		}
		res = append(res, pgReq)
	}

	return res, nil
}

func processPromptEnrichment(enrichment *agentgateway.AIPromptEnrichment) *api.BackendPolicySpec_Ai_PromptEnrichment {
	pgPromptEnrichment := &api.BackendPolicySpec_Ai_PromptEnrichment{}

	// Add prepend messages
	for _, msg := range enrichment.Prepend {
		pgPromptEnrichment.Prepend = append(pgPromptEnrichment.Prepend, &api.BackendPolicySpec_Ai_Message{
			Role:    msg.Role,
			Content: msg.Content,
		})
	}

	// Add append messages
	for _, msg := range enrichment.Append {
		pgPromptEnrichment.Append = append(pgPromptEnrichment.Append, &api.BackendPolicySpec_Ai_Message{
			Role:    msg.Role,
			Content: msg.Content,
		})
	}

	return pgPromptEnrichment
}

func processWebhook(ctx PolicyCtx, namespace string, webhook *agentgateway.Webhook) (*api.BackendPolicySpec_Ai_Webhook, error) {
	if webhook == nil {
		return nil, nil
	}

	be, err := buildBackendRef(ctx, webhook.BackendRef, namespace)
	if err != nil {
		return nil, fmt.Errorf("failed to build webhook: %v", err)
	}

	w := &api.BackendPolicySpec_Ai_Webhook{
		Backend: be,
	}

	if len(webhook.ForwardHeaderMatches) > 0 {
		headers := make([]*api.HeaderMatch, 0, len(webhook.ForwardHeaderMatches))
		for _, match := range webhook.ForwardHeaderMatches {
			switch ptr.OrDefault(match.Type, gwv1.HeaderMatchExact) {
			case gwv1.HeaderMatchExact:
				headers = append(headers, &api.HeaderMatch{
					Name:  string(match.Name),
					Value: &api.HeaderMatch_Exact{Exact: match.Value},
				})

			case gwv1.HeaderMatchRegularExpression:
				headers = append(headers, &api.HeaderMatch{
					Name:  string(match.Name),
					Value: &api.HeaderMatch_Regex{Regex: match.Value},
				})
			}
		}
		w.ForwardHeaderMatches = headers
	}

	return w, nil
}

func processBuiltinRegexRule(builtin agentgateway.BuiltIn, logger *slog.Logger) *api.BackendPolicySpec_Ai_RegexRule {
	v := api.BackendPolicySpec_Ai_BUILTIN_UNSPECIFIED
	switch builtin {
	case agentgateway.SSN:
		v = api.BackendPolicySpec_Ai_SSN

	case agentgateway.CREDIT_CARD:
		v = api.BackendPolicySpec_Ai_CREDIT_CARD

	case agentgateway.PHONE_NUMBER:
		v = api.BackendPolicySpec_Ai_PHONE_NUMBER

	case agentgateway.EMAIL:
		v = api.BackendPolicySpec_Ai_EMAIL

	case agentgateway.CA_SIN:
		v = api.BackendPolicySpec_Ai_CA_SIN

	default:
		logger.Warn("unknown builtin regex rule", "builtin", builtin)
	}
	return &api.BackendPolicySpec_Ai_RegexRule{
		Kind: &api.BackendPolicySpec_Ai_RegexRule_Builtin{
			Builtin: v,
		},
	}
}

func processRegexRule(pattern string) *api.BackendPolicySpec_Ai_RegexRule {
	return &api.BackendPolicySpec_Ai_RegexRule{
		Kind: &api.BackendPolicySpec_Ai_RegexRule_Regex{
			Regex: pattern,
		},
	}
}

func processRegex(regex *agentgateway.Regex) *api.BackendPolicySpec_Ai_RegexRules {
	if regex == nil {
		return nil
	}

	rules := &api.BackendPolicySpec_Ai_RegexRules{}
	if regex.Action != nil {
		switch *regex.Action {
		case agentgateway.MASK:
			rules.Action = api.BackendPolicySpec_Ai_MASK
		case agentgateway.REJECT:
			rules.Action = api.BackendPolicySpec_Ai_REJECT
		default:
			logger.Warn("unsupported regex action", "action", *regex.Action)
		}
	}

	for _, match := range regex.Matches {
		rules.Rules = append(rules.Rules, processRegexRule(match))
	}

	for _, builtin := range regex.Builtins {
		rules.Rules = append(rules.Rules, processBuiltinRegexRule(builtin, logger))
	}

	return rules
}

func processModeration(ctx PolicyCtx, namespace string, moderation *agentgateway.OpenAIModeration) *api.BackendPolicySpec_Ai_Moderation {
	if moderation == nil {
		return nil
	}

	pgModeration := &api.BackendPolicySpec_Ai_Moderation{}
	pgModeration.Model = moderation.Model

	if moderation.Policies != nil {
		pol := &agentgateway.BackendFull{
			BackendSimple: *moderation.Policies,
		}
		pols, err := TranslateInlineBackendPolicy(ctx, namespace, pol)
		if err != nil {
			logger.Warn("failed to translate policy", "err", err)
		} else {
			pgModeration.InlinePolicies = pols
		}
	}

	return pgModeration
}

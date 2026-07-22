package plugins

import (
	"fmt"
	"log/slog"

	"istio.io/istio/pkg/ptr"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
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
		} else if req.BedrockGuardrails != nil {
			pgReq.Kind = &api.BackendPolicySpec_Ai_RequestGuard_BedrockGuardrails{
				BedrockGuardrails: processBedrockGuardrails(ctx, namespace, req.BedrockGuardrails),
			}
		} else if req.GoogleModelArmor != nil {
			pgReq.Kind = &api.BackendPolicySpec_Ai_RequestGuard_GoogleModelArmor{
				GoogleModelArmor: processGoogleModelArmor(ctx, namespace, req.GoogleModelArmor),
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
		} else if req.BedrockGuardrails != nil {
			pgReq.Kind = &api.BackendPolicySpec_Ai_ResponseGuard_BedrockGuardrails{
				BedrockGuardrails: processBedrockGuardrails(ctx, namespace, req.BedrockGuardrails),
			}
		} else if req.GoogleModelArmor != nil {
			pgReq.Kind = &api.BackendPolicySpec_Ai_ResponseGuard_GoogleModelArmor{
				GoogleModelArmor: processGoogleModelArmor(ctx, namespace, req.GoogleModelArmor),
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

	be, err := BuildBackendRef(ctx, webhook.BackendRef, namespace)
	if err != nil {
		return nil, fmt.Errorf("failed to build webhook: %v", err)
	}

	w := &api.BackendPolicySpec_Ai_Webhook{
		Backend:       be,
		FailureMode:   webhookFailureMode(webhook.FailureMode),
		MessageFormat: webhookMessageFormat(webhook.MessageFormat),
	}
	w.ForwardHeaderMatches = convertForwardHeaderMatches(webhook.ForwardHeaderMatches)

	return w, nil
}

func convertForwardHeaderMatches(matches []gwv1.HTTPHeaderMatch) []*api.HeaderMatch {
	if len(matches) == 0 {
		return nil
	}
	headers := make([]*api.HeaderMatch, 0, len(matches))
	for _, match := range matches {
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
	return headers
}

func webhookFailureMode(mode agentgateway.FailureMode) api.BackendPolicySpec_Ai_Webhook_FailureMode {
	if mode == agentgateway.FailOpen {
		return api.BackendPolicySpec_Ai_Webhook_FAIL_OPEN
	}
	return api.BackendPolicySpec_Ai_Webhook_FAIL_CLOSED
}

func webhookMessageFormat(format agentgateway.MessageFormat) api.BackendPolicySpec_Ai_Webhook_MessageFormat {
	if format == agentgateway.MessageFormatRaw {
		return api.BackendPolicySpec_Ai_Webhook_RAW
	}
	// Default (empty or Simplified) maps to SIMPLIFIED.
	return api.BackendPolicySpec_Ai_Webhook_SIMPLIFIED
}

func processContextCompression(ctx PolicyCtx, namespace string, cc *agentgateway.ContextCompressionConfig) (*api.BackendPolicySpec_Ai_ContextCompression, error) {
	if cc == nil {
		return nil, nil
	}

	be, err := BuildBackendRef(ctx, cc.BackendRef, namespace)
	if err != nil {
		return nil, fmt.Errorf("failed to build context compression backend: %v", err)
	}

	out := &api.BackendPolicySpec_Ai_ContextCompression{
		Backend:              be,
		FailureMode:          contextCompressionFailureMode(cc.FailureMode),
		ForwardHeaderMatches: convertForwardHeaderMatches(cc.ForwardHeaderMatches),
	}
	if cc.Path != "" {
		out.Path = new(cc.Path)
	}
	if cc.MinSizeBytes > 0 {
		out.MinSizeBytes = new(uint32(cc.MinSizeBytes)) //nolint:gosec // G115: MinSizeBytes is validated by kubebuilder to be >= 0
	}
	return out, nil
}

func contextCompressionFailureMode(mode agentgateway.FailureMode) api.BackendPolicySpec_Ai_ContextCompression_FailureMode {
	if mode == agentgateway.FailClosed {
		return api.BackendPolicySpec_Ai_ContextCompression_FAIL_CLOSED
	}
	// Default (empty or FailOpen) maps to FAIL_OPEN.
	return api.BackendPolicySpec_Ai_ContextCompression_FAIL_OPEN
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
		pols, err := translateAuxiliaryBackendPolicies(ctx, namespace, moderation.Policies)
		if err != nil {
			logger.Warn("failed to translate policy", "err", err)
		} else {
			pgModeration.InlinePolicies = pols
		}
	}

	return pgModeration
}

func processBedrockGuardrails(ctx PolicyCtx, namespace string, guardrails *agentgateway.BedrockGuardrails) *api.BackendPolicySpec_Ai_BedrockGuardrails {
	if guardrails == nil {
		return nil
	}

	pgGuardrails := &api.BackendPolicySpec_Ai_BedrockGuardrails{
		Identifier: guardrails.GuardrailIdentifier,
		Version:    guardrails.GuardrailVersion,
		Region:     guardrails.Region,
	}

	if guardrails.Policies != nil {
		pols, err := translateAuxiliaryBackendPolicies(ctx, namespace, guardrails.Policies)
		if err != nil {
			logger.Warn("failed to translate policy", "err", err)
		} else {
			pgGuardrails.InlinePolicies = pols
		}
	}

	return pgGuardrails
}

func processGoogleModelArmor(ctx PolicyCtx, namespace string, armor *agentgateway.GoogleModelArmor) *api.BackendPolicySpec_Ai_GoogleModelArmor {
	if armor == nil {
		return nil
	}

	pgArmor := &api.BackendPolicySpec_Ai_GoogleModelArmor{
		TemplateId: armor.TemplateID,
		ProjectId:  armor.ProjectID,
	}

	// Set location with default value if not specified
	if armor.Location != nil {
		pgArmor.Location = new(*armor.Location)
	} else {
		pgArmor.Location = new("us-central1")
	}

	if armor.Policies != nil {
		pols, err := translateAuxiliaryBackendPolicies(ctx, namespace, armor.Policies)
		if err != nil {
			logger.Warn("failed to translate policy", "err", err)
		} else {
			pgArmor.InlinePolicies = pols
		}
	}

	return pgArmor
}

type backendSimplePolicy interface {
	BackendSimple() *agentgateway.BackendSimple
}

func translateAuxiliaryBackendPolicies(ctx PolicyCtx, namespace string, policies backendSimplePolicy) ([]*api.BackendPolicySpec, error) {
	if policies == nil {
		return nil, nil
	}
	backend := policies.BackendSimple()
	if backend == nil {
		return nil, nil
	}
	pol := &agentgateway.BackendFull{
		BackendSimple: *backend,
	}
	return TranslateInlineBackendPolicy(ctx, namespace, pol)
}

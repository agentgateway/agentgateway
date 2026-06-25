package plugins

import (
	"bytes"
	"cmp"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"iter"
	"strconv"
	"strings"
	"time"

	"github.com/google/cel-go/cel"
	"google.golang.org/protobuf/types/known/durationpb"
	"google.golang.org/protobuf/types/known/structpb"
	"istio.io/istio/pkg/config"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/maps"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	"istio.io/istio/pkg/util/protomarshal"
	apiextensionsv1 "k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/jwks"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/reporter"
	"github.com/agentgateway/agentgateway/controller/pkg/reports"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

const (
	extauthPolicySuffix            = ":extauth"
	extprocPolicySuffix            = ":extproc"
	rbacPolicySuffix               = ":rbac"
	localRateLimitPolicySuffix     = ":rl-local"
	globalRateLimitPolicySuffix    = ":rl-global"
	transformationPolicySuffix     = ":transformation"
	csrfPolicySuffix               = ":csrf"
	corsPolicySuffix               = ":cors"
	headerModifierPolicySuffix     = ":header-modifier"
	respHeaderModifierPolicySuffix = ":resp-header-modifier"
	hostnameRewritePolicySuffix    = ":hostname-rewrite"
	retryPolicySuffix              = ":retry"
	timeoutPolicySuffix            = ":timeout"
	jwtPolicySuffix                = ":jwt"
	basicAuthPolicySuffix          = ":basicauth"
	apiKeyPolicySuffix             = ":apikeyauth" //nolint:gosec
	directResponseSuffix           = ":direct-response"
	bufferSuffix                   = ":buffer"
)

var logger = logging.New("agentgateway/plugins")

// Shared CEL environment for expression validation
var celEnv *cel.Env

func init() {
	var err error
	celEnv, err = cel.NewEnv()
	if err != nil {
		logger.Error("failed to create CEL environment", "error", err)
		// Optionally, set celEnv to a default or nil value
		celEnv = nil // or some default configuration
	}
}

// ConvertStatusCollection converts the specific TrafficPolicy status collection
// to the generic controllers.Object status collection expected by the interface
func ConvertStatusCollection[T controllers.Object, S any](
	col krt.Collection[krt.ObjectWithStatus[T, S]],
	toOptions func(string) []krt.CollectionOption,
	originalName string,
) krt.StatusCollection[controllers.Object, any] {
	return krt.MapCollection(col, func(item krt.ObjectWithStatus[T, S]) krt.ObjectWithStatus[controllers.Object, any] {
		return krt.ObjectWithStatus[controllers.Object, any]{
			Obj:    controllers.Object(item.Obj),
			Status: item.Status,
		}
	}, toOptions(originalName+"/mapped")...)
}

// NewAgentPlugin creates a new AgentgatewayPolicy plugin
func NewAgentPlugin(agw *AgwCollections, resolver remotehttp.Resolver, jwksLookup jwks.Lookup, credentialResolver kubeutils.CredentialResolver) AgwPlugin {
	return AgwPlugin{
		ContributesPolicies: map[schema.GroupKind]PolicyPlugin{
			wellknown.AgentgatewayPolicyGVK.GroupKind(): {
				Build: func(input PolicyPluginInput) (krt.StatusCollection[controllers.Object, any], krt.Collection[AgwPolicy]) {
					policyStatusCol, policyCol := krt.NewStatusManyCollection(agw.AgentgatewayPolicies, func(krtctx krt.HandlerContext, policyCR *agentgateway.AgentgatewayPolicy) (
						*gwv1.PolicyStatus,
						[]AgwPolicy,
					) {
						return TranslateAgentgatewayPolicy(krtctx, policyCR, agw, input.References, input.Grants, resolver, jwksLookup, credentialResolver)
					}, agw.KrtOpts.ToOptions("policies/Agentgateway")...)
					return ConvertStatusCollection(policyStatusCol, agw.KrtOpts.ToOptions, "policies/Agentgateway"), policyCol
				},
				BuildReferences: func(input PolicyPluginInput) krt.Collection[*PolicyAttachment] {
					return krt.NewManyCollection(agw.AgentgatewayPolicies, func(ctx krt.HandlerContext, policy *agentgateway.AgentgatewayPolicy) []*PolicyAttachment {
						return BackendReferencesFromPolicy(ctx, policy, input.References, agw, input.Grants)
					}, agw.KrtOpts.ToOptions("references/AgentgatewayPolicyAttachments")...)
				},
			},
		},
	}
}

type PolicyCtx struct {
	Krt         krt.HandlerContext
	Collections *AgwCollections
	References  ReferenceIndex
	Grants      ReferenceGrantChecker
	SourceGVK   schema.GroupVersionKind
	Resolver    remotehttp.Resolver
	JWKSLookup  jwks.Lookup

	// CredentialResolver resolves credential refs: the built-in Secret resolver
	// in OSS, or an injected resolver (which may itself be a chain). Access it
	// through ResolveCredentialRef, which is nil-safe.
	CredentialResolver kubeutils.CredentialResolver
}

// PolicySourceGVK returns the Kubernetes kind that should be used as the
// ReferenceGrant "from" kind for backend refs emitted while translating this
// policy.
func (ctx PolicyCtx) PolicySourceGVK() schema.GroupVersionKind {
	if ctx.SourceGVK == (schema.GroupVersionKind{}) {
		return wellknown.AgentgatewayPolicyGVK
	}
	return ctx.SourceGVK
}

// ResolveCredentialRef applies the context's credential resolver.
func (ctx PolicyCtx) ResolveCredentialRef(ref agentgateway.LocalSecretObjectRef, namespace string) (map[string][]byte, error) {
	if ctx.CredentialResolver == nil {
		return nil, errors.New("secret credential resolver is not configured")
	}
	return ctx.CredentialResolver.ResolveCredentialRef(ctx.Krt, ref, namespace)
}

type ResolvedTarget struct {
	AgentgatewayTarget *api.PolicyTarget
	GatewayTargets     []types.NamespacedName
	AncestorRefs       []gwv1.ParentReference
	AttachmentError    string
}

// TranslateAgentgatewayPolicy generates policies for a single traffic policy
func TranslateAgentgatewayPolicy(
	ctx krt.HandlerContext,
	policy *agentgateway.AgentgatewayPolicy,
	agw *AgwCollections,
	references ReferenceIndex,
	grants ReferenceGrantChecker,
	resolver remotehttp.Resolver,
	jwksLookup jwks.Lookup,
	credentialResolver kubeutils.CredentialResolver,
) (*gwv1.PolicyStatus, []AgwPolicy) {
	var agwPolicies []AgwPolicy
	existingStatus := policy.Status.DeepCopy()

	pctx := PolicyCtx{
		Krt:                ctx,
		Collections:        agw,
		References:         references,
		Grants:             grants,
		SourceGVK:          wellknown.AgentgatewayPolicyGVK,
		Resolver:           resolver,
		JWKSLookup:         jwksLookup,
		CredentialResolver: credentialResolver,
	}
	var ancestors []gwv1.PolicyAncestorStatus
	var attachmentErrors []string
	// TODO: add selectors
	baseTranslatedPolicies, baseErr := TranslatePolicyToAgw(pctx, policy)
	baseConds := PolicyConditionMap(baseErr, len(baseTranslatedPolicies) > 0)
	controller := gwv1.GatewayController(agw.ControllerName)

	processTarget := func(name gwv1.ObjectName, targetNamespace string, gk schema.GroupKind, policyTargets []*api.PolicyTarget, targetExists bool) {
		if len(policyTargets) == 0 {
			logger.Warn("unsupported target kind", "kind", gk.Kind, "policy", policy.Name)
			return
		}

		targetObject := utils.TypedNamespacedName{
			NamespacedName: types.NamespacedName{Namespace: targetNamespace, Name: string(name)},
			Kind:           gk.Kind,
		}

		for _, policyTarget := range policyTargets {
			// For backend-like targets, skip gateway resolution when the target doesn't exist.
			// A missing backend could still resolve via PolicyAttachments if another backend
			// chain happens to reference the same name, which would push config for a phantom target.
			// Gateway/route targets use direct lookup (no PolicyAttachments), so they're safe.
			var gatewayTargets []types.NamespacedName
			if !IsBackendLikeTarget(policyTarget) || targetExists {
				gatewayTargets = references.LookupGatewaysForPolicyTarget(ctx, targetObject, policyTarget).UnsortedList()
				translatedPolicies := ClonePoliciesForTarget(baseTranslatedPolicies, policyTarget)
				for _, translatedPolicy := range translatedPolicies {
					for _, gatewayTarget := range gatewayTargets {
						agwPolicies = append(agwPolicies, AgwPolicy{
							Gateway: new(gatewayTarget),
							Policy:  translatedPolicy,
						})
					}
				}
			}

			ancestorRefs, attachmentErr := resolvePolicyAncestorRefs(targetNamespace, targetObject, gatewayTargets, targetExists)
			if attachmentErr != "" {
				attachmentErrors = append(attachmentErrors, attachmentErr)
			}

			for _, ar := range ancestorRefs {
				// A policy should report at most one status per Gateway parent, even if multiple
				// targetRefs/targetSelectors resolve to the same Gateway.
				if slices.IndexFunc(ancestors, func(existing gwv1.PolicyAncestorStatus) bool {
					return existing.ControllerName == controller && ParentRefEquals(existing.AncestorRef, ar)
				}) != -1 {
					continue
				}
				ancestors = append(ancestors, SetAncestorStatus(ar, existingStatus, policy.Generation, baseConds, controller))
			}
		}
	}

	type targetKey struct {
		Group       string
		Kind        string
		Name        string
		Namespace   string
		SectionName string
	}
	seen := make(map[targetKey]struct{})
	tryProcessTarget := func(gk schema.GroupKind, name gwv1.ObjectName, sectionName *gwv1.SectionName, targetNamespace string) {
		section := ""
		if sectionName != nil {
			section = string(*sectionName)
		}
		key := targetKey{Group: gk.Group, Kind: gk.Kind, Name: string(name), Namespace: targetNamespace, SectionName: section}
		if _, ok := seen[key]; ok {
			return
		}
		seen[key] = struct{}{}
		policyTargets, targetExists := references.PolicyTarget(ctx, targetNamespace, name, gk, sectionName)
		processTarget(name, targetNamespace, gk, policyTargets, targetExists)
	}

	for _, target := range policy.Spec.TargetRefs {
		gk := schema.GroupKind{Group: string(target.Group), Kind: string(target.Kind)}
		tryProcessTarget(gk, target.Name, target.SectionName, policy.Namespace)
	}
	for _, selector := range policy.Spec.TargetSelectors {
		gk := schema.GroupKind{Group: string(selector.Group), Kind: string(selector.Kind)}
		targets := references.PolicyTargetsBySelector(ctx, policy.Namespace, selector)
		if len(targets) == 0 {
			attachmentErrors = append(attachmentErrors, fmt.Sprintf("Policy is not attached: no %s matching selector found in namespace %s", gk.Kind, policy.Namespace))
		}
		for _, target := range targets {
			processTarget(target.Name, target.Namespace, gk, target.PolicyTargets, true)
		}
	}

	if len(attachmentErrors) > 0 {
		logger.Warn("failed to resolve one or more ancestor refs", "errors", attachmentErrors)
		ancestors = append(ancestors, SetAncestorStatus(gwv1.ParentReference{
			Group: new(gwv1.Group(wellknown.AgentgatewayPolicyGVK.Group)),
			Name:  "StatusSummary",
		}, existingStatus, policy.Generation, attachmentErrorConditionMap(baseConds, attachmentErrors), controller))
	}

	// Build final status from accumulated ancestors
	status := gwv1.PolicyStatus{
		Ancestors: MergeAncestors(agw.ControllerName, existingStatus.Ancestors, ancestors),
	}

	// sort all parents for consistency with Equals and for Update
	// match sorting semantics of istio/istio, see:
	// https://github.com/istio/istio/blob/6dcaa0206bcaf20e3e3b4e45e9376f0f96365571/pilot/pkg/config/kube/gateway/conditions.go#L188-L193
	slices.SortStableFunc(status.Ancestors, func(a, b gwv1.PolicyAncestorStatus) int {
		return strings.Compare(reports.ParentString(a.AncestorRef), reports.ParentString(b.AncestorRef))
	})

	return &status, agwPolicies
}

func PolicyConditionMap(err error, hasTranslatedPolicies bool) map[string]*Condition {
	conds := map[string]*Condition{}
	if err != nil {
		// If we produced some policies alongside errors, treat as partial validity
		if hasTranslatedPolicies {
			conds[string(agentgateway.PolicyConditionAccepted)] = &Condition{
				Status:  metav1.ConditionTrue,
				Reason:  string(agentgateway.PolicyReasonPartiallyValid),
				Message: err.Error(),
			}
		} else {
			// No policies produced and error present -> invalid
			conds[string(agentgateway.PolicyConditionAccepted)] = &Condition{
				Status:  metav1.ConditionFalse,
				Reason:  string(agentgateway.PolicyReasonInvalid),
				Message: err.Error(),
			}
			conds[string(agentgateway.PolicyConditionAttached)] = &Condition{
				Status:  metav1.ConditionFalse,
				Reason:  string(agentgateway.PolicyReasonPending),
				Message: "Policy is not attached due to invalid status",
			}
		}
	} else {
		// Check for partial validity
		// Build success conditions per ancestor
		conds[string(agentgateway.PolicyConditionAccepted)] = &Condition{
			Status:  metav1.ConditionTrue,
			Reason:  string(agentgateway.PolicyReasonValid),
			Message: reporter.PolicyAcceptedMsg,
		}
		conds[string(agentgateway.PolicyConditionAttached)] = &Condition{
			Status:  metav1.ConditionTrue,
			Reason:  string(agentgateway.PolicyReasonAttached),
			Message: reporter.PolicyAttachedMsg,
		}
	}
	return conds
}

func attachmentErrorConditionMap(baseConds map[string]*Condition, attachmentErrors []string) map[string]*Condition {
	conds := maps.Clone(baseConds)
	conds[string(agentgateway.PolicyConditionAttached)] = &Condition{
		Status:  metav1.ConditionFalse,
		Reason:  string(agentgateway.PolicyReasonPending),
		Message: strings.Join(attachmentErrors, "\n"),
	}
	return conds
}

func resolvePolicyAncestorRefs(
	policyNamespace string,
	targetObject utils.TypedNamespacedName,
	gatewayTargets []types.NamespacedName,
	targetExists bool,
) ([]gwv1.ParentReference, string) {
	if !targetExists {
		return nil, fmt.Sprintf("Policy is not attached: %s %s/%s not found", targetObject.Kind, policyNamespace, targetObject.Name)
	}

	if len(gatewayTargets) == 0 {
		return nil, fmt.Sprintf("Policy is not attached: %s %s/%s is not attached to any Gateway", targetObject.Kind, policyNamespace, targetObject.Name)
	}

	refs := make([]gwv1.ParentReference, 0, len(gatewayTargets))
	for _, gatewayTarget := range gatewayTargets {
		refs = append(refs, gwv1.ParentReference{
			Name:      gwv1.ObjectName(gatewayTarget.Name),
			Namespace: new(gwv1.Namespace(gatewayTarget.Namespace)),
			Group:     new(gwv1.Group(wellknown.GatewayGVK.Group)),
			Kind:      new(gwv1.Kind(wellknown.GatewayGVK.Kind)),
		})
	}
	slices.SortStableFunc(refs, func(a, b gwv1.ParentReference) int {
		return strings.Compare(reports.ParentString(a), reports.ParentString(b))
	})
	return refs, ""
}

// TranslatePolicyToAgw converts a TrafficPolicy to agentgateway Policy resources
func TranslatePolicyToAgw(
	ctx PolicyCtx,
	policy *agentgateway.AgentgatewayPolicy,
) ([]*api.Policy, error) {
	agwPolicies := make([]*api.Policy, 0)
	var errs []error

	frontend, err := translateFrontendPolicyToAgw(ctx, policy)
	agwPolicies = append(agwPolicies, frontend...)
	if err != nil {
		errs = append(errs, err)
	}

	traffic, err := translateTrafficPolicyToAgw(ctx, policy)
	agwPolicies = append(agwPolicies, traffic...)
	if err != nil {
		errs = append(errs, err)
	}

	backend, err := translateBackendPolicyToAgw(ctx, policy)
	agwPolicies = append(agwPolicies, backend...)
	if err != nil {
		errs = append(errs, err)
	}

	return agwPolicies, errors.Join(errs...)
}

func ClonePoliciesForTarget(base []*api.Policy, policyTarget *api.PolicyTarget) []*api.Policy {
	if len(base) == 0 {
		return nil
	}
	out := make([]*api.Policy, 0, len(base))
	for _, p := range base {
		clone := protomarshal.ShallowClone(p)
		clone.Key += attachmentName(policyTarget)
		clone.Target = policyTarget
		out = append(out, clone)
	}
	return out
}

func translateTrafficPolicyToAgw(
	ctx PolicyCtx,
	policy *agentgateway.AgentgatewayPolicy,
) ([]*api.Policy, error) {
	traffic := policy.Spec.Traffic
	if traffic == nil {
		return nil, nil
	}

	agwPolicies := make([]*api.Policy, 0)
	var errs []error

	// Generate a base policy name from the TrafficPolicy reference
	basePolicyName := getTrafficPolicyName(policy.Namespace, policy.Name)
	policyName := config.NamespacedName(policy)
	inheritance := translatePolicyInheritance(policy.Spec.Strategy)

	appendPolicy := func(kind string) func(*api.Policy, error) {
		return func(p *api.Policy, err error) {
			if err != nil {
				name := fmt.Sprintf("%s %s", kind, policyName)
				logger.Error("error processing policy", "policy", name, "error", err)
				errs = append(errs, err)
			}
			if p != nil {
				p.Inheritance = inheritance
				agwPolicies = append(agwPolicies, p)
			}
		}
	}

	appendPolicies := func(kind string) func([]*api.Policy, error) {
		return func(policies []*api.Policy, err error) {
			if err != nil {
				name := fmt.Sprintf("%s %s", kind, policyName)
				logger.Error("error processing policy", "policy", name, "error", err)
				errs = append(errs, err)
			}
			for _, p := range policies {
				if p != nil {
					p.Inheritance = inheritance
				}
			}
			agwPolicies = append(agwPolicies, policies...)
		}
	}

	// Convert ExtAuth policy if present
	if traffic.ExtAuth != nil {
		appendPolicy("extAuth")(processConditional(
			traffic.ExtAuth,
			processExtAuthPolicy,
			extauthPolicySuffix,
			ctx,
			traffic.Phase,
			basePolicyName,
			policyName,
		))
	}

	// Convert ExtProc policy if present
	if traffic.ExtProc != nil {
		appendPolicy("extProc")(processConditional(
			traffic.ExtProc,
			processExtProcTraffic,
			extprocPolicySuffix,
			ctx,
			traffic.Phase,
			basePolicyName,
			policyName,
		))
	}

	// Convert Authorization policy if present
	if traffic.Authorization != nil {
		appendPolicy("authorization")(processAuthorizationPolicy(traffic.Authorization, traffic.Phase, basePolicyName, policyName))
	}

	// Process RateLimit policies if present
	if traffic.RateLimit != nil {
		appendPolicies("rateLimit")(processRateLimitPolicy(ctx, traffic.RateLimit, traffic.Phase, basePolicyName, policyName))
	}

	// Process transformation policies if present
	if traffic.Transformation != nil {
		appendPolicy("transformation")(processConditional(
			traffic.Transformation,
			processTransformationTraffic,
			transformationPolicySuffix,
			ctx,
			traffic.Phase,
			basePolicyName,
			policyName,
		))
	}

	// Process CSRF policies if present
	if traffic.Csrf != nil {
		appendPolicy("csrf")(processCSRFPolicy(traffic.Csrf, basePolicyName, policyName), nil)
	}

	if traffic.Cors != nil {
		appendPolicy("cors")(processCorsPolicy(traffic.Cors, traffic.Phase, basePolicyName, policyName), nil)
	}

	if traffic.HeaderModifiers != nil {
		appendPolicies("headerModifiers")(processHeaderModifierPolicy(traffic.HeaderModifiers, basePolicyName, policyName), nil)
	}

	if traffic.HostnameRewrite != nil {
		appendPolicy("hostnameRewrite")(processHostnameRewritePolicy(traffic.HostnameRewrite, basePolicyName, policyName), nil)
	}

	if traffic.Timeouts != nil {
		appendPolicy("timeouts")(processTimeoutPolicy(traffic.Timeouts, basePolicyName, policyName), nil)
	}

	if traffic.Retry != nil {
		appendPolicy("retry")(processRetriesPolicy(traffic.Retry, basePolicyName, policyName))
	}

	if traffic.DirectResponse != nil {
		appendPolicy("directResponse")(processConditional(
			traffic.DirectResponse,
			processDirectResponseTraffic,
			directResponseSuffix,
			ctx,
			traffic.Phase,
			basePolicyName,
			policyName,
		))
	}

	if traffic.Buffer != nil {
		appendPolicy("buffer")(processBufferPolicy(traffic.Buffer, basePolicyName, policyName))
	}

	if traffic.JWTAuthentication != nil {
		appendPolicy("jwtAuthentication")(processJWTAuthenticationPolicy(ctx, traffic.JWTAuthentication, traffic.Phase, basePolicyName, policyName))
	}

	if traffic.APIKeyAuthentication != nil {
		appendPolicy("apiKeyAuthentication")(processAPIKeyAuthenticationPolicy(ctx, traffic.APIKeyAuthentication, traffic.Phase, basePolicyName, policyName))
	}

	if traffic.BasicAuthentication != nil {
		appendPolicy("basicAuthentication")(processBasicAuthenticationPolicy(ctx, traffic.BasicAuthentication, traffic.Phase, basePolicyName, policyName))
	}
	return agwPolicies, errors.Join(errs...)
}

func bufferBodyOnOverflow(mode agentgateway.OverflowAction) api.TrafficPolicySpec_Buffer_OverflowAction {
	if mode == agentgateway.ContinueStreaming {
		return api.TrafficPolicySpec_Buffer_CONTINUE_STREAMING
	}
	return api.TrafficPolicySpec_Buffer_RETURN_ERROR
}

func translateBufferBody(b *agentgateway.BufferBody) *api.TrafficPolicySpec_Buffer_BufferBody {
	bBody := &api.TrafficPolicySpec_Buffer_BufferBody{}
	if b != nil {
		if v := b.MaxBytes; v != nil {
			bBody.MaxBytes = quantityUint32(v)
		}
		bBody.OnOverflow = bufferBodyOnOverflow(b.OnOverflow)
		return bBody
	}

	return nil
}

func processBufferPolicy(buffer *agentgateway.Buffer, basePolicyName string, policyName types.NamespacedName) (*api.Policy, error) {
	var errs []error
	translatedBuffer := &api.TrafficPolicySpec_Buffer{}
	translatedBuffer.Request = translateBufferBody(buffer.Request)
	translatedBuffer.Response = translateBufferBody(buffer.Response)

	bufferPolicy := &api.Policy{
		Key:  basePolicyName + bufferSuffix,
		Name: TypedResourceFromName(wellknown.AgentgatewayPolicyGVK.Kind, policyName),
		Kind: &api.Policy_Traffic{
			Traffic: &api.TrafficPolicySpec{
				Kind: &api.TrafficPolicySpec_Buffer_{
					Buffer: translatedBuffer,
				},
			},
		},
	}

	logger.Debug("generated Buffer policy",
		"policy", basePolicyName,
		"agentgateway_policy", bufferPolicy.Name)

	return bufferPolicy, errors.Join(errs...)
}

func translatePolicyInheritance(strategy *agentgateway.PolicyStrategy) api.Policy_Inheritance {
	if strategy == nil || strategy.Inheritance == nil {
		return api.Policy_DEFAULT
	}
	if *strategy.Inheritance == agentgateway.PolicyInheritanceOverride {
		return api.Policy_OVERRIDE
	}
	return api.Policy_DEFAULT
}

func processRetriesPolicy(retry *agentgateway.Retry, basePolicyName string, policy types.NamespacedName) (*api.Policy, error) {
	translatedRetry := &api.Retry{}
	var errs []error

	if retry.Codes != nil {
		for _, c := range retry.Codes {
			translatedRetry.RetryStatusCodes = append(translatedRetry.RetryStatusCodes, int32(c)) //nolint:gosec // G115: HTTP status codes are always positive integers (100-599)
		}
	}

	if retry.Backoff != nil {
		// This SHOULD be impossible due to CEL validation
		// In the unlikely event its not, we use no backoff
		d, err := time.ParseDuration(string(*retry.Backoff))
		if err != nil {
			errs = append(errs, fmt.Errorf("failed to parse retries backoff: %w", err))
		} else {
			translatedRetry.Backoff = durationpb.New(d)
		}
	}

	if a := retry.Attempts; a != nil {
		if *a < 0 {
			errs = append(errs, fmt.Errorf("failed to parse retry attempts should be positive int32 (%d)", *a))
		} else {
			// Agentgateway stores this as a u8 so has a max of 255
			translatedRetry.Attempts = int32(min(*retry.Attempts, 255)) //nolint:gosec // G115: max 255 so cannot fail
		}
	}

	if retry.Precondition != nil {
		translatedRetry.Precondition = string(*retry.Precondition)
	}

	if retry.Condition != nil {
		translatedRetry.Condition = string(*retry.Condition)
	}

	retryPolicy := &api.Policy{
		Key:  basePolicyName + retryPolicySuffix,
		Name: TypedResourceFromName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Traffic{
			Traffic: &api.TrafficPolicySpec{
				Kind: &api.TrafficPolicySpec_Retry{Retry: translatedRetry},
			},
		},
	}

	logger.Debug("generated Retry policy",
		"policy", basePolicyName,
		"agentgateway_policy", retryPolicy.Name)

	return retryPolicy, errors.Join(errs...)
}

func processDirectResponseTraffic(_ PolicyCtx, directResponse *agentgateway.DirectResponse, _ types.NamespacedName) (*api.Policy_Traffic, error) {
	if directResponse.StatusCode == nil {
		return nil, fmt.Errorf("failed to build directResponse: status is required")
	}
	var errs []error
	if directResponse.Body != nil && directResponse.BodyExpression != nil {
		errs = append(errs, fmt.Errorf("directResponse body and bodyExpression may not both be set"))
	}
	dr := &api.DirectResponse{
		Status: uint32(*directResponse.StatusCode), // nolint:gosec // G115: kubebuilder validation ensures safe for uint32
	}
	tp := &api.TrafficPolicySpec{
		Kind: &api.TrafficPolicySpec_DirectResponse{
			DirectResponse: dr,
		},
	}

	// Add body if specified
	if directResponse.Body != nil {
		dr.Body = []byte(*directResponse.Body)
	}
	if directResponse.BodyExpression != nil {
		if !isCEL(*directResponse.BodyExpression) {
			errs = append(errs, fmt.Errorf("directResponse bodyExpression is not a valid CEL expression: %s", *directResponse.BodyExpression))
		}
		dr.BodyExpression = string(*directResponse.BodyExpression)
	}
	for _, header := range directResponse.Headers {
		if !isCEL(header.Value) {
			errs = append(errs, fmt.Errorf("directResponse header %q is not a valid CEL expression: %s", header.Name, header.Value))
		}
		dr.Headers = append(dr.Headers, &api.ExpressionHeader{
			Name:       string(header.Name),
			Expression: string(header.Value),
		})
	}

	return &api.Policy_Traffic{Traffic: tp}, errors.Join(errs...)
}

func processJWTAuthenticationPolicy(ctx PolicyCtx, jwt *agentgateway.JWTAuthentication, policyPhase *agentgateway.PolicyPhase, basePolicyName string, policy types.NamespacedName) (*api.Policy, error) {
	p := &api.TrafficPolicySpec_JWT{}
	p.AuthorizationLocation = translateAuthorizationExtractionLocation(jwt.Location)

	switch jwt.Mode {
	case agentgateway.JWTAuthenticationModeOptional:
		p.Mode = api.TrafficPolicySpec_JWT_OPTIONAL
	case agentgateway.JWTAuthenticationModeStrict:
		p.Mode = api.TrafficPolicySpec_JWT_STRICT
	case agentgateway.JWTAuthenticationModePermissive:
		p.Mode = api.TrafficPolicySpec_JWT_PERMISSIVE
	}

	errs := make([]error, 0)
	if err := validateExtractionAuthorizationLocation(jwt.Location, "jwtAuthentication location"); err != nil {
		errs = append(errs, err)
	}
	for idx, pp := range jwt.Providers {
		jp := &api.TrafficPolicySpec_JWTProvider{
			Issuer:    pp.Issuer,
			Audiences: pp.Audiences,
		}
		if i := pp.JWKS.Inline; i != nil {
			jp.JwksSource = &api.TrafficPolicySpec_JWTProvider_Inline{Inline: *i}
			p.Providers = append(p.Providers, jp)
			continue
		}
		if r := pp.JWKS.Remote; r != nil {
			owner, ok := jwks.PolicyJWTProviderLookupOwner(policy.Namespace, policy.Name, idx, pp)
			if !ok {
				continue
			}
			inline, err := resolveJWKSInlineForOwner(ctx, owner)
			if err != nil {
				errs = append(errs, err)
				continue
			}
			jp.JwksSource = &api.TrafficPolicySpec_JWTProvider_Inline{Inline: inline}
			p.Providers = append(p.Providers, jp)
		}
	}

	if jwt.MCP != nil {
		if len(jwt.Providers) != 1 {
			errs = append(errs, fmt.Errorf("jwtAuthentication.mcp requires exactly one provider, found %d", len(jwt.Providers)))
		} else {
			mcp, err := translateJWTMCPConfig(jwt.MCP)
			if err != nil {
				errs = append(errs, err)
			} else {
				p.Mcp = mcp
			}
		}
	}

	jwtPolicy := &api.Policy{
		Key:  basePolicyName + jwtPolicySuffix,
		Name: TypedResourceFromName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Traffic{
			Traffic: &api.TrafficPolicySpec{
				Phase: phase(policyPhase),
				Kind:  &api.TrafficPolicySpec_Jwt{Jwt: p},
			},
		},
	}

	logger.Debug("generated jwt policy",
		"policy", basePolicyName,
		"agentgateway_policy", jwtPolicy.Name)

	return jwtPolicy, errors.Join(errs...)
}

func processBasicAuthenticationPolicy(
	ctx PolicyCtx,
	ba *agentgateway.BasicAuthentication,
	policyPhase *agentgateway.PolicyPhase,
	basePolicyName string,
	policy types.NamespacedName,
) (*api.Policy, error) {
	p := &api.TrafficPolicySpec_BasicAuthentication{}
	p.Realm = ba.Realm
	p.AuthorizationLocation = translateAuthorizationExtractionLocation(ba.Location)

	switch ba.Mode {
	case agentgateway.BasicAuthenticationModeOptional:
		p.Mode = api.TrafficPolicySpec_BasicAuthentication_OPTIONAL
	case agentgateway.BasicAuthenticationModeStrict:
		p.Mode = api.TrafficPolicySpec_BasicAuthentication_STRICT
	}

	var errs []error
	if err := validateExtractionAuthorizationLocation(ba.Location, "basicAuthentication location"); err != nil {
		errs = append(errs, err)
	}

	if s := ba.SecretRef; s != nil {
		data, err := ctx.ResolveCredentialRef(*s, policy.Namespace)
		if err != nil {
			errs = append(errs, err)
		} else {
			d, ok := data[".htaccess"]
			if !ok {
				errs = append(errs, fmt.Errorf("basic authentication secret %v found, but doesn't contain '.htaccess' key", s.Name))
			}
			p.HtpasswdContent = string(d)
		}
	}
	if len(ba.Users) > 0 {
		p.HtpasswdContent = strings.Join(ba.Users, "\n")
	}
	basicAuthPolicy := &api.Policy{
		Key:  basePolicyName + basicAuthPolicySuffix,
		Name: TypedResourceFromName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Traffic{
			Traffic: &api.TrafficPolicySpec{
				Phase: phase(policyPhase),
				Kind:  &api.TrafficPolicySpec_BasicAuth{BasicAuth: p},
			},
		},
	}

	logger.Debug("generated basic auth policy",
		"policy", basePolicyName,
		"agentgateway_policy", basicAuthPolicy.Name)

	return basicAuthPolicy, errors.Join(errs...)
}

type APIKeyEntry struct {
	Key      string          `json:"key"`
	KeyHash  string          `json:"keyHash"`
	Metadata json.RawMessage `json:"metadata"`
}

func validateAPIKeyHash(keyHash string) error {
	const prefix = "sha256:"
	if !strings.HasPrefix(keyHash, prefix) {
		return fmt.Errorf("keyHash must use the %s<hex> format", prefix)
	}
	decoded, err := hex.DecodeString(strings.TrimPrefix(keyHash, prefix))
	if err != nil {
		return fmt.Errorf("keyHash must contain a valid sha256 hex digest: %w", err)
	}
	if len(decoded) != 32 {
		return fmt.Errorf("keyHash sha256 digest must decode to 32 bytes")
	}
	return nil
}

func processAPIKeyAuthenticationPolicy(
	ctx PolicyCtx,
	ak *agentgateway.APIKeyAuthentication,
	policyPhase *agentgateway.PolicyPhase,
	basePolicyName string,
	policy types.NamespacedName,
) (*api.Policy, error) {
	p := &api.TrafficPolicySpec_APIKey{}
	p.AuthorizationLocation = translateAuthorizationExtractionLocation(ak.Location)

	switch ak.Mode {
	case agentgateway.APIKeyAuthenticationModeOptional:
		p.Mode = api.TrafficPolicySpec_APIKey_OPTIONAL
	case agentgateway.APIKeyAuthenticationModeStrict:
		p.Mode = api.TrafficPolicySpec_APIKey_STRICT
	case agentgateway.APIKeyAuthenticationModePermissive:
		p.Mode = api.TrafficPolicySpec_APIKey_PERMISSIVE
	}

	type apiKeyData struct {
		name string
		data map[string][]byte
	}
	var dataSets []apiKeyData
	var errs []error
	if err := validateExtractionAuthorizationLocation(ak.Location, "apiKeyAuthentication location"); err != nil {
		errs = append(errs, err)
	}
	if s := ak.SecretRef; s != nil {
		data, err := ctx.ResolveCredentialRef(*s, policy.Namespace)
		if err != nil {
			errs = append(errs, err)
		} else {
			dataSets = []apiKeyData{{name: string(s.Name), data: data}}
		}
	}
	if s := ak.SecretSelector; s != nil {
		dataSets = nil
		// Preserve existing precedence: secretSelector replaces secretRef, and
		// remains Secret-only. CredentialRef resolution is handled by secretRef.
		for _, secret := range krt.Fetch(ctx.Krt, ctx.Collections.Secrets, krt.FilterLabel(s.MatchLabels), krt.FilterIndex(ctx.Collections.SecretsByNamespace, policy.Namespace)) {
			dataSets = append(dataSets, apiKeyData{name: secret.Name, data: secret.Data})
		}
	}
	for _, s := range dataSets {
		for k, v := range s.data {
			trimmed := bytes.TrimSpace(v)
			if len(trimmed) == 0 {
				errs = append(errs, fmt.Errorf("secret %v contains invalid key %v: empty value", s.name, k))
				continue
			}
			var ke APIKeyEntry
			if trimmed[0] != '{' {
				// A raw key entry without metadata
				ke = APIKeyEntry{
					Key:      string(v),
					KeyHash:  "",
					Metadata: nil,
				}
			} else if err := json.Unmarshal(trimmed, &ke); err != nil {
				errs = append(errs, fmt.Errorf("secret %v contains invalid key %v: %w", s.name, k, err))
				continue
			}
			if (ke.Key == "") == (ke.KeyHash == "") {
				errs = append(errs, fmt.Errorf("secret %v contains invalid key %v: exactly one of key or keyHash must be set", s.name, k))
				continue
			}
			if ke.KeyHash != "" {
				if err := validateAPIKeyHash(ke.KeyHash); err != nil {
					errs = append(errs, fmt.Errorf("secret %v contains invalid key %v: %w", s.name, k, err))
					continue
				}
			}

			pbs, err := toStruct(ke.Metadata)
			if err != nil {
				errs = append(errs, fmt.Errorf("secret %v contains invalid key %v: %w", s.name, k, err))
				continue
			}
			p.ApiKeys = append(p.ApiKeys, &api.TrafficPolicySpec_APIKey_User{
				Key:      ke.Key,
				KeyHash:  ke.KeyHash,
				Metadata: pbs,
			})
		}
	}
	// Ensure deterministic ordering
	slices.SortFunc(p.ApiKeys, func(a, b *api.TrafficPolicySpec_APIKey_User) int {
		return cmp.Or(
			cmp.Compare(a.Key, b.Key),
			cmp.Compare(a.KeyHash, b.KeyHash),
		)
	})
	apiKeyPolicy := &api.Policy{
		Key:  basePolicyName + apiKeyPolicySuffix,
		Name: TypedResourceFromName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Traffic{
			Traffic: &api.TrafficPolicySpec{
				Phase: phase(policyPhase),
				Kind:  &api.TrafficPolicySpec_ApiKeyAuth{ApiKeyAuth: p},
			},
		},
	}

	logger.Debug("generated api key auth policy",
		"policy", basePolicyName,
		"agentgateway_policy", apiKeyPolicy.Name)

	return apiKeyPolicy, errors.Join(errs...)
}

func processTimeoutPolicy(timeout *agentgateway.Timeouts, basePolicyName string, policy types.NamespacedName) *api.Policy {
	timeoutPolicy := &api.Policy{
		Key:  basePolicyName + timeoutPolicySuffix,
		Name: TypedResourceFromName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Traffic{
			Traffic: &api.TrafficPolicySpec{
				Kind: &api.TrafficPolicySpec_Timeout{Timeout: &api.Timeout{
					Request: durationpb.New(timeout.Request.Duration),
				}},
			},
		},
	}

	logger.Debug("generated Timeout policy",
		"policy", basePolicyName,
		"agentgateway_policy", timeoutPolicy.Name)

	return timeoutPolicy
}

func processHostnameRewritePolicy(hnrw *agentgateway.HostnameRewrite, basePolicyName string, policy types.NamespacedName) *api.Policy {
	r := &api.TrafficPolicySpec_HostRewrite{}
	switch hnrw.Mode {
	case agentgateway.HostnameRewriteModeAuto:
		r.Mode = api.TrafficPolicySpec_HostRewrite_AUTO
	case agentgateway.HostnameRewriteModeNone:
		r.Mode = api.TrafficPolicySpec_HostRewrite_NONE
	}

	p := &api.Policy{
		Key:  basePolicyName + hostnameRewritePolicySuffix,
		Name: TypedResourceFromName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Traffic{
			Traffic: &api.TrafficPolicySpec{
				Kind: &api.TrafficPolicySpec_HostRewrite_{HostRewrite: r},
			},
		},
	}

	logger.Debug("generated HostnameRewrite policy",
		"policy", basePolicyName,
		"agentgateway_policy", p.Name)

	return p
}

func processHeaderModifierPolicy(headerModifier *agentgateway.HeaderModifiers, basePolicyName string, policy types.NamespacedName) []*api.Policy {
	var policies []*api.Policy

	var headerModifierPolicyRequest, headerModifierPolicyResponse *api.Policy
	if headerModifier.Request != nil {
		headerModifierPolicyRequest = &api.Policy{
			Key:  basePolicyName + headerModifierPolicySuffix,
			Name: TypedResourceFromName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
			Kind: &api.Policy_Traffic{
				Traffic: &api.TrafficPolicySpec{
					Kind: &api.TrafficPolicySpec_RequestHeaderModifier{RequestHeaderModifier: &api.HeaderModifier{
						Add:    headerListToAgw(headerModifier.Request.Add),
						Set:    headerListToAgw(headerModifier.Request.Set),
						Remove: headerModifier.Request.Remove,
					}},
				},
			},
		}
		logger.Debug("generated HeaderModifier policy",
			"policy", basePolicyName,
			"agentgateway_policy", headerModifierPolicyRequest.Name)
		policies = append(policies, headerModifierPolicyRequest)
	}

	if headerModifier.Response != nil {
		headerModifierPolicyResponse = &api.Policy{
			Key:  basePolicyName + respHeaderModifierPolicySuffix,
			Name: TypedResourceFromName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
			Kind: &api.Policy_Traffic{
				Traffic: &api.TrafficPolicySpec{
					Kind: &api.TrafficPolicySpec_ResponseHeaderModifier{ResponseHeaderModifier: &api.HeaderModifier{
						Add:    headerListToAgw(headerModifier.Response.Add),
						Set:    headerListToAgw(headerModifier.Response.Set),
						Remove: headerModifier.Response.Remove,
					}},
				},
			},
		}
		logger.Debug("generated HeaderModifier policy",
			"policy", basePolicyName,
			"agentgateway_policy", headerModifierPolicyResponse.Name)
		policies = append(policies, headerModifierPolicyResponse)
	}

	return policies
}

func processCorsPolicy(cors *agentgateway.CORS, policyPhase *agentgateway.PolicyPhase, basePolicyName string, policy types.NamespacedName) *api.Policy {
	corsPolicy := &api.Policy{
		Key:  basePolicyName + corsPolicySuffix,
		Name: TypedResourceFromName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Traffic{
			Traffic: &api.TrafficPolicySpec{
				Phase: phase(policyPhase),
				Kind: &api.TrafficPolicySpec_Cors{Cors: &api.CORS{
					AllowCredentials: ptr.OrEmpty(cors.AllowCredentials),
					AllowHeaders:     slices.Map(cors.AllowHeaders, func(h gwv1.HTTPHeaderName) string { return string(h) }),
					AllowMethods:     slices.Map(cors.AllowMethods, func(m gwv1.HTTPMethodWithWildcard) string { return string(m) }),
					AllowOrigins:     slices.Map(cors.AllowOrigins, func(o gwv1.CORSOrigin) string { return string(o) }),
					ExposeHeaders:    slices.Map(cors.ExposeHeaders, func(h gwv1.HTTPHeaderName) string { return string(h) }),
					MaxAge: &durationpb.Duration{
						Seconds: int64(cors.MaxAge),
					},
				}},
			},
		},
	}

	logger.Debug("generated Cors policy",
		"policy", basePolicyName,
		"agentgateway_policy", corsPolicy.Name)

	return corsPolicy
}

func processConditional[T any](
	condPol agentgateway.ConditionalPolicy[T],
	f func(ctx PolicyCtx, pol *T, polName types.NamespacedName) (*api.Policy_Traffic, error),
	suffix string,
	ctx PolicyCtx,
	policyPhase *agentgateway.PolicyPhase,
	basePolicyName string,
	policy types.NamespacedName,
) (*api.Policy, error) {
	concrete, conditional := condPol.ConditionalPolicy()
	if concrete != nil {
		base, err := f(ctx, concrete, policy)
		if base == nil {
			return nil, err
		}
		base.Traffic.Phase = phase(policyPhase)
		extauthPolicy := &api.Policy{
			Key:  basePolicyName + suffix,
			Name: TypedResourceFromName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
			Kind: base,
		}

		logger.Debug("generated policy",
			"kind", strings.TrimPrefix(suffix, ":"),
			"policy", basePolicyName,
			"agentgateway_policy", extauthPolicy.Name)

		return extauthPolicy, err
	}
	var entries []agentgateway.ConditionalPolicyEntry[T]
	for cond := range conditional {
		entries = append(entries, cond)
	}
	return processConditionalEntries(entries, f, suffix, ctx, policyPhase, basePolicyName, policy)
}

func processConditionalEntries[T any](
	entries []agentgateway.ConditionalPolicyEntry[T],
	f func(ctx PolicyCtx, pol *T, polName types.NamespacedName) (*api.Policy_Traffic, error),
	suffix string,
	ctx PolicyCtx,
	policyPhase *agentgateway.PolicyPhase,
	basePolicyName string,
	policy types.NamespacedName,
) (*api.Policy, error) {
	var errs []error
	conditionals := &api.ConditionalPolicies{}
	for _, cond := range entries {
		base, err := f(ctx, &cond.Policy, policy)
		if err != nil {
			errs = append(errs, err)
		}
		if base == nil {
			continue
		}
		base.Traffic.Phase = phase(policyPhase)
		c := &api.ConditionalPolicy{
			Kind: &api.ConditionalPolicy_Traffic{Traffic: base.Traffic},
		}
		if cond.Condition != "" {
			if !isCEL(cond.Condition) {
				errs = append(errs, fmt.Errorf("condition CEL expression is invalid: %s", cond.Condition))
			}
			c.Condition = new(string(cond.Condition))
		}
		conditionals.Policies = append(conditionals.Policies, c)
	}
	pol := &api.Policy{
		Key:  basePolicyName + suffix,
		Name: TypedResourceFromName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Conditional{Conditional: conditionals},
	}

	logger.Debug("generated policy",
		"kind", strings.TrimPrefix(suffix, ":"),
		"policy", basePolicyName,
		"agentgateway_policy", pol.Name)

	return pol, errors.Join(errs...)
}

// processExtAuthPolicy processes ExtAuth configuration and creates corresponding agentgateway policies
func processExtAuthPolicy(
	ctx PolicyCtx,
	extAuth *agentgateway.ExtAuth,
	policy types.NamespacedName,
) (*api.Policy_Traffic, error) {
	spec, err := buildExtAuthSpec(ctx, extAuth, policy)
	return &api.Policy_Traffic{
		Traffic: &api.TrafficPolicySpec{
			Kind: &api.TrafficPolicySpec_ExtAuthz{
				ExtAuthz: spec,
			},
		},
	}, err
}

func buildExtAuthSpec(
	ctx PolicyCtx,
	extAuth *agentgateway.ExtAuth,
	policy types.NamespacedName,
) (*api.TrafficPolicySpec_ExternalAuth, error) {
	var errs []error
	var be *api.BackendReference
	if extAuth.BackendRef == nil {
		errs = append(errs, fmt.Errorf("failed to build extAuth: backendRef is required"))
	} else {
		var err error
		be, err = BuildBackendRef(ctx, *extAuth.BackendRef, policy.Namespace)
		if err != nil {
			errs = append(errs, fmt.Errorf("failed to build extAuth: %v", err))
		}
	}

	spec := &api.TrafficPolicySpec_ExternalAuth{
		Target:      be,
		FailureMode: extAuthFailureMode(extAuth.FailureMode),
	}
	if g := extAuth.GRPC; g != nil {
		metadata := castCELMap(g.RequestMetadata, func(key string, expr agentgateway.CELExpression) {
			errs = append(errs, fmt.Errorf("extAuth grpc requestMetadata %q is not a valid CEL expression: %s", key, expr))
		})
		p := &api.TrafficPolicySpec_ExternalAuth_GRPCProtocol{
			Context:  g.ContextExtensions,
			Metadata: metadata,
		}
		spec.Protocol = &api.TrafficPolicySpec_ExternalAuth_Grpc{
			Grpc: p,
		}
	} else if h := extAuth.HTTP; h != nil {
		path := castCELPtr(h.Path, func(expr agentgateway.CELExpression) {
			errs = append(errs, fmt.Errorf("extAuth http path is not a valid CEL expression: %s", expr))
		})
		redirect := castCELPtr(h.Redirect, func(expr agentgateway.CELExpression) {
			errs = append(errs, fmt.Errorf("extAuth http redirect is not a valid CEL expression: %s", expr))
		})
		body := castCELPtr(h.Body, func(expr agentgateway.CELExpression) {
			errs = append(errs, fmt.Errorf("extAuth http body is not a valid CEL expression: %s", expr))
		})
		addRequestHeaders := castCELMap(h.AddRequestHeaders, func(key string, expr agentgateway.CELExpression) {
			errs = append(errs, fmt.Errorf("extAuth http addRequestHeaders %q is not a valid CEL expression: %s", key, expr))
		})
		metadata := castCELMap(h.ResponseMetadata, func(key string, expr agentgateway.CELExpression) {
			errs = append(errs, fmt.Errorf("extAuth http responseMetadata %q is not a valid CEL expression: %s", key, expr))
		})
		p := &api.TrafficPolicySpec_ExternalAuth_HTTPProtocol{
			Path:                   path,
			Redirect:               redirect,
			Body:                   body,
			IncludeResponseHeaders: h.AllowedResponseHeaders,
			AddRequestHeaders:      addRequestHeaders,
			Metadata:               metadata,
		}
		spec.IncludeRequestHeaders = h.AllowedRequestHeaders
		spec.Protocol = &api.TrafficPolicySpec_ExternalAuth_Http{
			Http: p,
		}
	}
	if b := extAuth.ForwardBody; b != nil {
		maxRequestBytes := uint32(8192)
		if v := b.MaxSize.ClampedValue(); v != nil {
			maxRequestBytes = *v
		}
		spec.IncludeRequestBody = &api.TrafficPolicySpec_ExternalAuth_BodyOptions{
			MaxRequestBytes: maxRequestBytes,
			// Currently the default, see https://github.com/kubernetes-sigs/gateway-api/issues/4198
			AllowPartialMessage: true,
			// TODO: should we allow config?
			PackAsBytes: false,
		}
	}
	if cache := extAuth.Cache; cache != nil {
		key := castCELSlice(cache.Key, func(expr agentgateway.CELExpression) {
			errs = append(errs, fmt.Errorf("extAuth cache key is not a valid CEL expression: %s", expr))
		})
		ttl := castExtAuthCacheTTL(cache.TTL, func(expr agentgateway.CELExpression) {
			errs = append(errs, fmt.Errorf("extAuth cache ttl is not a valid CEL expression: %s", expr))
		})
		spec.Cache = &api.TrafficPolicySpec_ExternalAuth_Cache{
			Key:        key,
			Ttl:        ttl,
			MaxEntries: ptr.OrDefault(cache.MaxEntries, 0),
		}
	}

	return spec, errors.Join(errs...)
}

// processExtProcPolicy processes ExtProc configuration and creates corresponding agentgateway policies
func processExtProcTraffic(
	ctx PolicyCtx,
	extProc *agentgateway.ExtProc,
	policy types.NamespacedName,
) (*api.Policy_Traffic, error) {
	var backendErr error
	var be *api.BackendReference
	if extProc.BackendRef == nil {
		backendErr = fmt.Errorf("failed to build extProc: backendRef is required")
	} else {
		var err error
		be, err = BuildBackendRef(ctx, *extProc.BackendRef, policy.Namespace)
		if err != nil {
			backendErr = fmt.Errorf("failed to build extProc: %v", err)
		}
	}

	spec := &api.TrafficPolicySpec_ExtProc{
		Target: be,
		// always use FAIL_CLOSED to prevent silent data loss when ExtProc is unavailable.
		FailureMode: api.TrafficPolicySpec_ExtProc_FAIL_CLOSED,
	}
	if extProc.ProcessingOptions != nil {
		spec.ProcessingOptions = &api.TrafficPolicySpec_ExtProc_ProcessingOptions{
			RequestBodyMode:   api.TrafficPolicySpec_ExtProc_FULL_DUPLEX_STREAMED,
			ResponseBodyMode:  api.TrafficPolicySpec_ExtProc_FULL_DUPLEX_STREAMED,
			AllowModeOverride: extProc.ProcessingOptions.AllowModeOverride,
		}
		if extProc.ProcessingOptions.RequestBodyMode != nil {
			spec.ProcessingOptions.RequestBodyMode = toBodySendMode(*extProc.ProcessingOptions.RequestBodyMode)
		}
		if extProc.ProcessingOptions.ResponseBodyMode != nil {
			spec.ProcessingOptions.ResponseBodyMode = toBodySendMode(*extProc.ProcessingOptions.ResponseBodyMode)
		}
		if extProc.ProcessingOptions.RequestHeaderMode != nil {
			spec.ProcessingOptions.RequestHeaderMode = toHeaderSendMode(*extProc.ProcessingOptions.RequestHeaderMode)
		}
		if extProc.ProcessingOptions.ResponseHeaderMode != nil {
			spec.ProcessingOptions.ResponseHeaderMode = toHeaderSendMode(*extProc.ProcessingOptions.ResponseHeaderMode)
		}
		if extProc.ProcessingOptions.RequestTrailerMode != nil {
			spec.ProcessingOptions.RequestTrailerMode = toTrailerSendMode(*extProc.ProcessingOptions.RequestTrailerMode)
		}
		if extProc.ProcessingOptions.ResponseTrailerMode != nil {
			spec.ProcessingOptions.ResponseTrailerMode = toTrailerSendMode(*extProc.ProcessingOptions.ResponseTrailerMode)
		}
	}

	return &api.Policy_Traffic{
		Traffic: &api.TrafficPolicySpec{
			Kind: &api.TrafficPolicySpec_ExtProc_{
				ExtProc: spec,
			},
		},
	}, backendErr
}

func toBodySendMode(mode agentgateway.BodySendMode) api.TrafficPolicySpec_ExtProc_BodySendMode {
	switch mode {
	case agentgateway.BodySendModeBuffered:
		return api.TrafficPolicySpec_ExtProc_BUFFERED
	case agentgateway.BodySendModeBufferedPartial:
		return api.TrafficPolicySpec_ExtProc_BUFFERED_PARTIAL
	case agentgateway.BodySendModeFullDuplexStreamed:
		return api.TrafficPolicySpec_ExtProc_FULL_DUPLEX_STREAMED
	default:
		return api.TrafficPolicySpec_ExtProc_NONE
	}
}

func toHeaderSendMode(mode agentgateway.HeaderSendMode) api.TrafficPolicySpec_ExtProc_HeaderTrailerSendMode {
	switch mode {
	case agentgateway.HeaderSendModeSkip:
		return api.TrafficPolicySpec_ExtProc_SKIP
	case agentgateway.HeaderSendModeSend:
		return api.TrafficPolicySpec_ExtProc_SEND
	default:
		return api.TrafficPolicySpec_ExtProc_SEND
	}
}

func toTrailerSendMode(mode agentgateway.TrailerSendMode) api.TrafficPolicySpec_ExtProc_HeaderTrailerSendMode {
	switch mode {
	case agentgateway.TrailerSendModeSend:
		return api.TrafficPolicySpec_ExtProc_SEND
	case agentgateway.TrailerSendModeSkip:
		return api.TrafficPolicySpec_ExtProc_SKIP
	default:
		return api.TrafficPolicySpec_ExtProc_SEND
	}
}

func phase(policyPhase *agentgateway.PolicyPhase) api.TrafficPolicySpec_PolicyPhase {
	var phase api.TrafficPolicySpec_PolicyPhase
	if policyPhase != nil {
		switch *policyPhase {
		case agentgateway.PolicyPhasePreRouting:
			phase = api.TrafficPolicySpec_GATEWAY
		case agentgateway.PolicyPhasePostRouting:
			phase = api.TrafficPolicySpec_ROUTE
		}
	}
	return phase
}

func cast[T ~string](items []T) []string {
	return slices.Map(items, func(item T) string {
		return string(item)
	})
}

func castCELSlice(items []agentgateway.CELExpression, invalid func(agentgateway.CELExpression)) []string {
	if items == nil {
		return nil
	}
	res := make([]string, 0, len(items))
	for _, item := range items {
		res = append(res, string(item))
		if !isCEL(item) {
			invalid(item)
		}
	}
	return res
}

func castCELMap(items map[string]agentgateway.CELExpression, invalid func(string, agentgateway.CELExpression)) map[string]string {
	if items == nil {
		return nil
	}
	res := make(map[string]string, len(items))
	for k, v := range maps.SeqStable(items) {
		res[k] = string(v)
		if !isCEL(v) {
			invalid(k, v)
		}
	}
	return res
}

func castCELPtr(item *agentgateway.CELExpression, invalid func(agentgateway.CELExpression)) *string {
	if item == nil {
		return nil
	}
	res := new(string(*item))
	if !isCEL(*item) {
		invalid(*item)
	}
	return res
}

func castCEL(item agentgateway.CELExpression, invalid func(agentgateway.CELExpression)) string {
	if !isCEL(item) {
		invalid(item)
	}
	return string(item)
}

func castExtAuthCacheTTL(item agentgateway.CELExpression, invalid func(agentgateway.CELExpression)) string {
	raw := string(item)
	if _, err := time.ParseDuration(raw); err == nil {
		return "duration(" + strconv.Quote(raw) + ")"
	}
	return castCEL(item, invalid)
}

// processAuthorizationPolicy processes Authorization configuration and creates corresponding Agw policies
func processAuthorizationPolicy(
	auth *agentgateway.Authorization,
	policyPhase *agentgateway.PolicyPhase,
	basePolicyName string,
	policy types.NamespacedName,
) (*api.Policy, error) {
	var errs []error
	var allowPolicies, denyPolicies, requirePolicies []string
	policies := castCELSlice(auth.Policy.MatchExpressions, func(expr agentgateway.CELExpression) {
		errs = append(errs, fmt.Errorf("authorization matchExpression is not a valid CEL expression: %s", expr))
	})
	if auth.Action == agentgateway.AuthorizationPolicyActionDeny {
		denyPolicies = append(denyPolicies, policies...)
	} else if auth.Action == agentgateway.AuthorizationPolicyActionRequire {
		requirePolicies = append(requirePolicies, policies...)
	} else {
		allowPolicies = append(allowPolicies, policies...)
	}

	pol := &api.Policy{
		Key:  basePolicyName + rbacPolicySuffix,
		Name: TypedResourceFromName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Traffic{
			Traffic: &api.TrafficPolicySpec{
				Phase: phase(policyPhase),
				Kind: &api.TrafficPolicySpec_Authorization{
					Authorization: &api.TrafficPolicySpec_RBAC{
						Allow:   allowPolicies,
						Deny:    denyPolicies,
						Require: requirePolicies,
					},
				},
			},
		},
	}

	logger.Debug("generated Authorization policy",
		"policy", basePolicyName,
		"agentgateway_policy", pol.Name)

	return pol, errors.Join(errs...)
}

func getFrontendPolicyName(trafficPolicyNs, trafficPolicyName string) string {
	return fmt.Sprintf("frontend/%s/%s", trafficPolicyNs, trafficPolicyName)
}

func getBackendPolicyName(trafficPolicyNs, trafficPolicyName string) string {
	return fmt.Sprintf("backend/%s/%s", trafficPolicyNs, trafficPolicyName)
}

func getTrafficPolicyName(trafficPolicyNs, trafficPolicyName string) string {
	return fmt.Sprintf("traffic/%s/%s", trafficPolicyNs, trafficPolicyName)
}

// processRateLimitPolicy processes RateLimit configuration and creates corresponding agentgateway policies
func processRateLimitPolicy(
	ctx PolicyCtx,
	rl *agentgateway.RateLimitsOrConditional,
	policyPhase *agentgateway.PolicyPhase,
	basePolicyName string,
	policy types.NamespacedName,
) ([]*api.Policy, error) {
	concrete, conditional := rl.ConditionalPolicy()
	if concrete != nil {
		return processConcreteRateLimitPolicy(ctx, concrete, policyPhase, basePolicyName, policy)
	}

	var localEntries []agentgateway.ConditionalPolicyEntry[[]agentgateway.LocalRateLimit]
	var globalEntries []agentgateway.ConditionalPolicyEntry[agentgateway.GlobalRateLimit]
	for cond := range conditional {
		if cond.Policy.Local != nil {
			localEntries = append(localEntries, agentgateway.ConditionalPolicyEntry[[]agentgateway.LocalRateLimit]{
				Condition: cond.Condition,
				Policy:    cond.Policy.Local,
			})
		}
		if cond.Policy.Global != nil {
			globalEntries = append(globalEntries, agentgateway.ConditionalPolicyEntry[agentgateway.GlobalRateLimit]{
				Condition: cond.Condition,
				Policy:    *cond.Policy.Global,
			})
		}
	}

	var agwPolicies []*api.Policy
	var errs []error
	if len(localEntries) > 0 {
		pol, err := processConditionalEntries(localEntries, processLocalRateLimitTraffic, localRateLimitPolicySuffix, ctx, policyPhase, basePolicyName, policy)
		if err != nil {
			errs = append(errs, err)
		}
		if pol != nil {
			agwPolicies = append(agwPolicies, pol)
		}
	}
	if len(globalEntries) > 0 {
		pol, err := processConditionalEntries(globalEntries, processGlobalRateLimitTraffic, globalRateLimitPolicySuffix, ctx, policyPhase, basePolicyName, policy)
		if err != nil {
			errs = append(errs, err)
		}
		if pol != nil {
			agwPolicies = append(agwPolicies, pol)
		}
	}

	return agwPolicies, errors.Join(errs...)
}

func processConcreteRateLimitPolicy(ctx PolicyCtx, rl *agentgateway.RateLimits, policyPhase *agentgateway.PolicyPhase, basePolicyName string, policy types.NamespacedName) ([]*api.Policy, error) {
	var agwPolicies []*api.Policy
	var errs []error

	// Process local rate limiting if present
	if rl.Local != nil {
		localPolicy := processLocalRateLimitPolicy(rl.Local, policyPhase, basePolicyName, policy)
		if localPolicy != nil {
			agwPolicies = append(agwPolicies, localPolicy)
		}
	}

	// Process global rate limiting if present
	if rl.Global != nil {
		globalPolicy, err := processGlobalRateLimitPolicy(ctx, *rl.Global, policyPhase, basePolicyName, policy)
		if err != nil {
			errs = append(errs, err)
		}
		if globalPolicy != nil {
			agwPolicies = append(agwPolicies, globalPolicy)
		}
	}

	return agwPolicies, errors.Join(errs...)
}

// processLocalRateLimitPolicy processes local rate limiting configuration
func processLocalRateLimitTraffic(_ PolicyCtx, limits *[]agentgateway.LocalRateLimit, _ types.NamespacedName) (*api.Policy_Traffic, error) {
	// TODO: support multiple
	limit := (*limits)[0]

	rule := &api.TrafficPolicySpec_LocalRateLimit{
		Type: api.TrafficPolicySpec_LocalRateLimit_REQUEST,
	}
	var capacity uint64
	if limit.Requests != nil {
		capacity = uint64(*limit.Requests) //nolint:gosec // G115: kubebuilder validation ensures non-negative, safe for uint64
		rule.Type = api.TrafficPolicySpec_LocalRateLimit_REQUEST
	} else {
		capacity = uint64(*limit.Tokens) //nolint:gosec // G115: kubebuilder validation ensures non-negative, safe for uint64
		rule.Type = api.TrafficPolicySpec_LocalRateLimit_TOKEN
	}
	rule.MaxTokens = capacity + uint64(ptr.OrEmpty(limit.Burst)) //nolint:gosec // G115: Burst is non-negative, safe for uint64
	rule.TokensPerFill = capacity
	switch limit.Unit {
	case agentgateway.LocalRateLimitUnitSeconds:
		rule.FillInterval = durationpb.New(time.Second)
	case agentgateway.LocalRateLimitUnitMinutes:
		rule.FillInterval = durationpb.New(time.Minute)
	case agentgateway.LocalRateLimitUnitHours:
		rule.FillInterval = durationpb.New(time.Hour)
	}

	return &api.Policy_Traffic{Traffic: &api.TrafficPolicySpec{
		Kind: &api.TrafficPolicySpec_LocalRateLimit_{
			LocalRateLimit: rule,
		},
	}}, nil
}

func processLocalRateLimitPolicy(limits []agentgateway.LocalRateLimit, policyPhase *agentgateway.PolicyPhase, basePolicyName string, policy types.NamespacedName) *api.Policy {
	tp, _ := processLocalRateLimitTraffic(PolicyCtx{}, &limits, policy)
	tp.Traffic.Phase = phase(policyPhase)
	return &api.Policy{
		Key:  basePolicyName + localRateLimitPolicySuffix,
		Name: TypedResourceFromName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: tp,
	}
}

func processGlobalRateLimitTraffic(ctx PolicyCtx, grl *agentgateway.GlobalRateLimit, policy types.NamespacedName) (*api.Policy_Traffic, error) {
	var errs []error
	be, err := BuildBackendRef(ctx, grl.BackendRef, policy.Namespace)
	if err != nil {
		errs = append(errs, fmt.Errorf("failed to build global rate limit: %v", err))
	}
	descriptors := make([]*api.TrafficPolicySpec_RemoteRateLimit_Descriptor, 0, len(grl.Descriptors))
	for _, d := range grl.Descriptors {
		agw, err := processRateLimitDescriptor(d)
		if err != nil {
			errs = append(errs, err)
		}
		if agw != nil {
			descriptors = append(descriptors, agw)
		}
	}

	return &api.Policy_Traffic{Traffic: &api.TrafficPolicySpec{
		Kind: &api.TrafficPolicySpec_RemoteRateLimit_{
			RemoteRateLimit: &api.TrafficPolicySpec_RemoteRateLimit{
				Domain:      grl.Domain,
				Target:      be,
				Descriptors: descriptors,
				FailureMode: remoteRateLimitFailureMode(grl.FailureMode),
			},
		},
	}}, errors.Join(errs...)
}

func processGlobalRateLimitPolicy(
	ctx PolicyCtx,
	grl agentgateway.GlobalRateLimit,
	policyPhase *agentgateway.PolicyPhase,
	basePolicyName string,
	policy types.NamespacedName,
) (*api.Policy, error) {
	tp, err := processGlobalRateLimitTraffic(ctx, &grl, policy)
	tp.Traffic.Phase = phase(policyPhase)

	// Build the RemoteRateLimit policy that agentgateway expects
	p := &api.Policy{
		Key:  basePolicyName + globalRateLimitPolicySuffix,
		Name: TypedResourceFromName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: tp,
	}

	return p, err
}

func processRateLimitDescriptor(descriptor agentgateway.RateLimitDescriptor) (*api.TrafficPolicySpec_RemoteRateLimit_Descriptor, error) {
	entries := make([]*api.TrafficPolicySpec_RemoteRateLimit_Entry, 0, len(descriptor.Entries))
	var errs []error

	for _, entry := range descriptor.Entries {
		if !isCEL(entry.Expression) {
			errs = append(errs, fmt.Errorf("rate limit descriptor entry %q is not a valid CEL expression: %s", entry.Name, entry.Expression))
		}
		entries = append(entries, &api.TrafficPolicySpec_RemoteRateLimit_Entry{
			Key:   entry.Name,
			Value: string(entry.Expression),
		})
	}

	rlType := api.TrafficPolicySpec_RemoteRateLimit_REQUESTS
	if descriptor.Unit != nil && *descriptor.Unit == agentgateway.RateLimitUnitTokens {
		rlType = api.TrafficPolicySpec_RemoteRateLimit_TOKENS
	}
	var cost *string
	if descriptor.Cost != nil {
		if !isCEL(*descriptor.Cost) {
			errs = append(errs, fmt.Errorf("rate limit descriptor cost is not a valid CEL expression: %s", *descriptor.Cost))
		}
		cost = new(string(*descriptor.Cost))
	}

	return &api.TrafficPolicySpec_RemoteRateLimit_Descriptor{
		Entries: entries,
		Type:    rlType,
		Cost:    cost,
	}, errors.Join(errs...)
}

func extAuthFailureMode(mode agentgateway.FailureMode) api.TrafficPolicySpec_ExternalAuth_FailureMode {
	if mode == agentgateway.FailOpen {
		return api.TrafficPolicySpec_ExternalAuth_ALLOW
	}
	return api.TrafficPolicySpec_ExternalAuth_DENY
}

func remoteRateLimitFailureMode(mode agentgateway.FailureMode) api.TrafficPolicySpec_RemoteRateLimit_FailureMode {
	if mode == agentgateway.FailOpen {
		return api.TrafficPolicySpec_RemoteRateLimit_FAIL_OPEN
	}
	return api.TrafficPolicySpec_RemoteRateLimit_FAIL_CLOSED
}

// BuildBackendRef constructs an agentgateway backend reference from a Gateway
// API backendRef and enforces any configured cross-namespace ReferenceGrant
// requirements for the policy source in ctx.
func BuildBackendRef(ctx PolicyCtx, ref gwv1.BackendObjectReference, defaultNS string) (*api.BackendReference, error) {
	kind := ptr.OrDefault(ref.Kind, wellknown.ServiceKind)
	group := ptr.OrDefault(ref.Group, "")
	gk := schema.GroupKind{
		Group: string(group),
		Kind:  string(kind),
	}
	if err := checkBackendRefGrant(ctx, ref, defaultNS, gk); err != nil {
		return nil, err
	}
	return ctx.References.PolicyBackend(ctx.Krt, defaultNS, gk, ref.Name, ref.Namespace, ref.Port)
}

func checkBackendRefGrant(ctx PolicyCtx, ref gwv1.BackendObjectReference, defaultNS string, gk schema.GroupKind) error {
	if ref.Namespace != nil &&
		string(*ref.Namespace) != defaultNS &&
		ctx.Collections.Settings.BackendRefGrantMode.RequirePolicyBackendGrant() {
		sourceGVK := ctx.PolicySourceGVK()
		if !ctx.Grants.BackendAllowed(
			ctx.Krt,
			sourceGVK,
			ref.Name,
			*ref.Namespace,
			defaultNS,
			gk,
			ctx.Collections.Settings.BackendRefGrantMode,
		) {
			article := "a"
			if sourceGVK.Kind != "" && strings.ContainsAny(strings.ToLower(sourceGVK.Kind[:1]), "aeiou") {
				article = "an"
			}
			return fmt.Errorf("backendRef %v/%v not accessible to %s %s in namespace %q (missing a ReferenceGrant?)", *ref.Namespace, ref.Name, article, sourceGVK.Kind, defaultNS)
		}
	}
	return nil
}

func toJSONValue(j apiextensionsv1.JSON) (string, error) {
	value := j.Raw
	if json.Valid(value) {
		return string(value), nil
	}

	if bytes.HasPrefix(value, []byte("{")) || bytes.HasPrefix(value, []byte("[")) {
		return "", fmt.Errorf("invalid JSON value: %s", string(value))
	}

	// Treat this as an unquoted string and marshal it to JSON
	marshaled, err := json.Marshal(value)
	if err != nil {
		return "", err
	}
	return string(marshaled), nil
}

func processCSRFPolicy(csrf *agentgateway.CSRF, basePolicyName string, policy types.NamespacedName) *api.Policy {
	csrfPolicy := &api.Policy{
		Key:  basePolicyName + csrfPolicySuffix,
		Name: TypedResourceFromName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Traffic{
			Traffic: &api.TrafficPolicySpec{
				Kind: &api.TrafficPolicySpec_Csrf{
					Csrf: &api.TrafficPolicySpec_CSRF{
						AdditionalOrigins: csrf.AdditionalOrigins,
					},
				},
			},
		},
	}

	return csrfPolicy
}

// processTransformationPolicy processes transformation configuration and creates corresponding Agw policies
func processTransformationTraffic(
	_ PolicyCtx,
	transformation *agentgateway.Transformation,
	_ types.NamespacedName,
) (*api.Policy_Traffic, error) {
	var errs []error
	convertedReq, err := convertTransformSpec(transformation.Request)
	if err != nil {
		errs = append(errs, err)
	}
	convertedResp, err := convertTransformSpec(transformation.Response)
	if err != nil {
		errs = append(errs, err)
	}

	if convertedResp != nil || convertedReq != nil {
		return &api.Policy_Traffic{
			Traffic: &api.TrafficPolicySpec{
				Kind: &api.TrafficPolicySpec_Transformation{
					Transformation: &api.TrafficPolicySpec_TransformationPolicy{
						Request:  convertedReq,
						Response: convertedResp,
					},
				},
			},
		}, errors.Join(errs...)
	}
	return nil, errors.Join(errs...)
}

// convertTransformSpec converts transformation specs to agentgateway format
func convertTransformSpec(spec *agentgateway.Transform) (*api.TrafficPolicySpec_TransformationPolicy_Transform, error) {
	if spec == nil {
		return nil, nil
	}
	var errs []error
	var transform *api.TrafficPolicySpec_TransformationPolicy_Transform

	for _, header := range spec.Set {
		headerValue := header.Value
		if !isCEL(headerValue) {
			errs = append(errs, fmt.Errorf("header value is not a valid CEL expression: %s", headerValue))
		}
		if transform == nil {
			transform = &api.TrafficPolicySpec_TransformationPolicy_Transform{}
		}
		transform.Set = append(transform.Set, &api.TrafficPolicySpec_HeaderTransformation{
			Name:       string(header.Name),
			Expression: string(header.Value),
		})
	}

	for _, header := range spec.Add {
		headerValue := header.Value
		if !isCEL(headerValue) {
			errs = append(errs, fmt.Errorf("invalid header value: %s", headerValue))
		}
		if transform == nil {
			transform = &api.TrafficPolicySpec_TransformationPolicy_Transform{}
		}
		transform.Add = append(transform.Add, &api.TrafficPolicySpec_HeaderTransformation{
			Name:       string(header.Name),
			Expression: string(header.Value),
		})
	}

	if spec.Remove != nil {
		if transform == nil {
			transform = &api.TrafficPolicySpec_TransformationPolicy_Transform{}
		}
		transform.Remove = cast(spec.Remove)
	}

	if spec.Body != nil {
		// Handle body transformation if present
		bodyValue := *spec.Body
		if !isCEL(bodyValue) {
			errs = append(errs, fmt.Errorf("body value is not a valid CEL expression: %s", bodyValue))
		}
		if transform == nil {
			transform = &api.TrafficPolicySpec_TransformationPolicy_Transform{}
		}
		transform.Body = &api.TrafficPolicySpec_BodyTransformation{
			Expression: string(bodyValue),
		}
	}

	if len(spec.Metadata) > 0 {
		if transform == nil {
			transform = &api.TrafficPolicySpec_TransformationPolicy_Transform{}
		}
		transform.Metadata = make(map[string]string, len(spec.Metadata))
		for key, value := range spec.Metadata {
			if !isCEL(value) {
				errs = append(errs, fmt.Errorf("metadata value is not a valid CEL expression: %s", value))
			}
			transform.Metadata[key] = string(value)
		}
	}

	return transform, errors.Join(errs...)
}

// Checks if the expression is a valid CEL expression
func isCEL(expr agentgateway.CELExpression) bool {
	_, iss := celEnv.Parse(string(expr))
	return iss.Err() == nil
}

func attachmentName(target *api.PolicyTarget) string {
	if target == nil {
		return ""
	}
	switch v := target.Kind.(type) {
	case *api.PolicyTarget_Gateway:
		b := ":" + v.Gateway.Namespace + "/" + v.Gateway.Name
		if v.Gateway.Listener != nil {
			b += "/" + *v.Gateway.Listener
		}
		return b
	case *api.PolicyTarget_Route:
		b := ":" + v.Route.Namespace + "/" + v.Route.Name
		if v.Route.RouteRule != nil {
			b += "/" + *v.Route.RouteRule
		}
		return b
	case *api.PolicyTarget_Backend:
		b := ":" + v.Backend.Namespace + "/" + v.Backend.Name
		if v.Backend.Section != nil {
			b += "/" + *v.Backend.Section
		}
		return b
	case *api.PolicyTarget_Service:
		b := ":" + v.Service.Namespace + "/" + v.Service.Hostname
		if v.Service.Port != nil {
			b += "/" + strconv.Itoa(int(*v.Service.Port))
		}
		return b
	case *api.PolicyTarget_ListenerSet:
		b := ":" + v.ListenerSet.Namespace + "/" + v.ListenerSet.Name
		if v.ListenerSet.Section != nil {
			b += "/" + *v.ListenerSet.Section
		}
		return b
	default:
		panic(fmt.Sprintf("unknown target kind %T", target))
	}
}

func headerListToAgw(hl []gwv1.HTTPHeader) []*api.Header {
	return slices.Map(hl, func(hl gwv1.HTTPHeader) *api.Header {
		return &api.Header{
			Name:  string(hl.Name),
			Value: hl.Value,
		}
	})
}

func toStruct(rm json.RawMessage) (*structpb.Struct, error) {
	j, err := json.Marshal(rm)
	if err != nil {
		return nil, err
	}

	pbs := &structpb.Struct{}
	if err := protomarshal.Unmarshal(j, pbs); err != nil {
		return nil, err
	}

	return pbs, nil
}

func DefaultString[T ~string](s *T, def string) string {
	if s == nil {
		return def
	}
	return string(*s)
}

// BackendReferencesFromPolicy only emits attachments for existing, unsectioned targets
// to prevent phantom chains and section-scoped over-attachment.
func BackendReferencesFromPolicy(
	ctx krt.HandlerContext,
	policy *agentgateway.AgentgatewayPolicy,
	references ReferenceIndex,
	agw *AgwCollections,
	grants ReferenceGrantChecker,
) []*PolicyAttachment {
	return BackendReferencesFromPolicyForSource(ctx, policy, references, agw, grants, wellknown.AgentgatewayPolicyGVK)
}

// BackendReferencesFromPolicyForSource emits policy backend attachments using
// sourceGVK as the policy kind for ReferenceGrant checks and attachment source
// metadata.
func BackendReferencesFromPolicyForSource(
	ctx krt.HandlerContext,
	policy *agentgateway.AgentgatewayPolicy,
	references ReferenceIndex,
	agw *AgwCollections,
	grants ReferenceGrantChecker,
	sourceGVK schema.GroupVersionKind,
) []*PolicyAttachment {
	if sourceGVK == (schema.GroupVersionKind{}) {
		sourceGVK = wellknown.AgentgatewayPolicyGVK
	}
	s := policy.Spec
	self := utils.TypedNamespacedName{
		NamespacedName: types.NamespacedName{Namespace: policy.Namespace, Name: policy.Name},
		Kind:           sourceGVK.Kind,
	}

	seenTargets := make(map[utils.TypedNamespacedName]struct{})
	var existingTargets []utils.TypedNamespacedName
	addTarget := func(tnn utils.TypedNamespacedName) {
		if _, ok := seenTargets[tnn]; ok {
			return
		}
		seenTargets[tnn] = struct{}{}
		existingTargets = append(existingTargets, tnn)
	}
	for _, tgt := range s.TargetRefs {
		gk := schema.GroupKind{Group: string(tgt.Group), Kind: string(tgt.Kind)}
		policyTarget, targetExists := references.PolicyTarget(ctx, policy.Namespace, tgt.Name, gk, tgt.SectionName)
		if policyTarget == nil || !targetExists {
			continue
		}
		addTarget(utils.TypedNamespacedName{
			NamespacedName: types.NamespacedName{Namespace: policy.Namespace, Name: string(tgt.Name)},
			Kind:           string(tgt.Kind),
		})
	}
	for _, selector := range s.TargetSelectors {
		for _, target := range references.PolicyTargetsBySelector(ctx, policy.Namespace, selector) {
			if len(target.PolicyTargets) == 0 {
				continue
			}
			addTarget(utils.TypedNamespacedName{
				NamespacedName: types.NamespacedName{Namespace: target.Namespace, Name: string(target.Name)},
				Kind:           string(selector.Kind),
			})
		}
	}
	if len(existingTargets) == 0 {
		return nil
	}

	backends := referencedBackendsFromPolicy(ctx, policy, agw, grants, sourceGVK)
	if len(backends) == 0 {
		return nil
	}

	attachments := make([]*PolicyAttachment, 0, len(existingTargets)*len(backends))
	for _, backend := range backends {
		for _, tgt := range existingTargets {
			attachments = append(attachments, &PolicyAttachment{
				Target:  tgt,
				Backend: backend,
				Source:  self,
			})
		}
	}
	return attachments
}

// PolicyOrConditionalSeq iterates over all concrete policy objects whether conditional or explicit.
func PolicyOrConditionalSeq[T any, P interface {
	agentgateway.ConditionalPolicy[T]
	comparable
}](p P) iter.Seq[T] {
	var zero P
	if p == zero {
		return func(yield func(T) bool) {}
	}
	explicit, cond := p.ConditionalPolicy()
	if explicit != nil {
		return func(yield func(T) bool) {
			yield(*explicit)
		}
	}

	return func(yield func(T) bool) {
		for e := range cond {
			yield(e.Policy)
		}
	}
}

func referencedBackendsFromPolicy(ctx krt.HandlerContext, policy *agentgateway.AgentgatewayPolicy, agw *AgwCollections, grants ReferenceGrantChecker, sourceGVK schema.GroupVersionKind) []utils.TypedNamespacedName {
	var backends []utils.TypedNamespacedName
	for _, ref := range referencedBackendRefsFromPolicy(policy) {
		kind := ptr.OrDefault(ref.Kind, wellknown.ServiceKind)
		group := ptr.OrDefault(ref.Group, "")
		gk := schema.GroupKind{Group: string(group), Kind: string(kind)}
		if err := checkBackendRefGrant(PolicyCtx{Krt: ctx, Collections: agw, Grants: grants, SourceGVK: sourceGVK}, ref, policy.Namespace, gk); err != nil {
			continue
		}
		backends = append(backends, utils.TypedNamespacedName{
			NamespacedName: types.NamespacedName{Namespace: DefaultString(ref.Namespace, policy.Namespace), Name: string(ref.Name)},
			Kind:           DefaultString(ref.Kind, wellknown.ServiceKind),
		})
	}
	return backends
}

func referencedBackendRefsFromPolicy(policy *agentgateway.AgentgatewayPolicy) []gwv1.BackendObjectReference {
	var backends []gwv1.BackendObjectReference
	app := func(ref gwv1.BackendObjectReference) {
		backends = append(backends, ref)
	}

	s := policy.Spec
	if s.Traffic != nil {
		for p := range PolicyOrConditionalSeq(s.Traffic.ExtAuth) {
			if p.BackendRef != nil {
				app(*p.BackendRef)
			}
		}
		for p := range PolicyOrConditionalSeq(s.Traffic.ExtProc) {
			if p.BackendRef != nil {
				app(*p.BackendRef)
			}
		}
		for p := range PolicyOrConditionalSeq(s.Traffic.RateLimit) {
			if p.Global != nil {
				app(p.Global.BackendRef)
			}
		}
		if s.Traffic.JWTAuthentication != nil {
			for _, p := range s.Traffic.JWTAuthentication.Providers {
				if p.JWKS.Remote != nil {
					app(p.JWKS.Remote.BackendRef)
				}
			}
		}
	}
	if s.Frontend != nil {
		if s.Frontend.Tracing != nil {
			app(s.Frontend.Tracing.BackendRef)
		}
		if s.Frontend.AccessLog != nil && s.Frontend.AccessLog.Otlp != nil {
			app(s.Frontend.AccessLog.Otlp.BackendRef)
		}
	}
	if s.Backend != nil {
		BackendReferencesFromBackendPolicy(s.Backend, app)
	}
	return backends
}

func BackendReferencesFromBackendPolicy(s *agentgateway.BackendFull, app func(ref gwv1.BackendObjectReference)) {
	appTunnel := func(backend *agentgateway.BackendSimple) {
		if backend != nil && backend.Tunnel != nil {
			app(backend.Tunnel.BackendRef)
		}
	}
	appTunnel(&s.BackendSimple)
	if s.ExtAuth != nil && s.ExtAuth.BackendRef != nil {
		app(*s.ExtAuth.BackendRef)
	}
	if s.MCP != nil && s.MCP.Authentication != nil {
		app(s.MCP.Authentication.JWKS.BackendRef)
	}
	if s.MCP != nil && s.MCP.Guardrails != nil {
		for _, p := range s.MCP.Guardrails.Processors {
			if p.Remote != nil {
				app(p.Remote.BackendRef)
			}
		}
	}
	if s.AI != nil && s.AI.PromptGuard != nil {
		for _, p := range s.AI.PromptGuard.Request {
			if p.Webhook != nil {
				app(p.Webhook.BackendRef)
			}
			if p.OpenAIModeration != nil {
				appTunnel(p.OpenAIModeration.Policies)
			}
			if p.GoogleModelArmor != nil {
				appTunnel(p.GoogleModelArmor.Policies)
			}
			if p.BedrockGuardrails != nil {
				appTunnel(p.BedrockGuardrails.Policies)
			}
		}
		for _, p := range s.AI.PromptGuard.Response {
			if p.Webhook != nil {
				app(p.Webhook.BackendRef)
			}
			if p.GoogleModelArmor != nil {
				appTunnel(p.GoogleModelArmor.Policies)
			}
			if p.BedrockGuardrails != nil {
				appTunnel(p.BedrockGuardrails.Policies)
			}
		}
	}
}

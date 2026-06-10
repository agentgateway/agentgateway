package agentgateway

// Configures CEL-based authorization.
type Authorization struct {
	// The authorization rule to evaluate.
	//
	// * `Allow`: any matching allow rule allows the request.
	// * `Require`: every require rule must match for the request to be allowed.
	// * `Deny`: any matching deny rule denies the request.
	//
	// A CEL expression that fails to evaluate does not match. Prefer `Require`
	// for deny-by-default behavior.
	//
	// If at least one `Allow` rule is configured, requests are denied unless at
	// least one allow rule matches.
	// +required
	Policy AuthorizationPolicy `json:"policy"`

	// The effect of this rule when it matches.
	// If unspecified, defaults to `Allow`.
	// `Require` rules are cumulative: all require rules must match.
	// +kubebuilder:default=Allow
	// +optional
	Action AuthorizationPolicyAction `json:"action,omitempty"`
}

// A Common Expression Language (CEL) expression.
// +kubebuilder:validation:MinLength=1
// +kubebuilder:validation:MaxLength=16384
// +k8s:deepcopy-gen=false
type CELExpression string

// Defines CEL expressions for a single authorization rule.
type AuthorizationPolicy struct {
	// CEL expressions that must all evaluate to true for the rule to match.
	//
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=256
	// +required
	MatchExpressions []CELExpression `json:"matchExpressions"`
}

// AuthorizationPolicyAction defines the action to take when the
// `RBACPolicies` matches.
// +k8s:enum
type AuthorizationPolicyAction string

const (
	// AuthorizationPolicyActionAllow defines the action to take when the
	// `RBACPolicies` matches.
	AuthorizationPolicyActionAllow AuthorizationPolicyAction = "Allow"
	// AuthorizationPolicyActionDeny denies the action to take when the
	// `RBACPolicies` matches.
	AuthorizationPolicyActionDeny AuthorizationPolicyAction = "Deny"
	// AuthorizationPolicyActionRequire requires the action to take when the RBACPolicies matches.
	AuthorizationPolicyActionRequire AuthorizationPolicyAction = "Require"
)

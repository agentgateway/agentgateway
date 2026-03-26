package shared

// Authorization defines the configuration for role-based access control.
type Authorization struct {
	// `policy` specifies the authorization rule to evaluate.
	//
	// * For `Allow` rules: any policy allows the request.
	// * For `Require` rules: all policies must match for the request to be allowed.
	// * For `Deny` rules: any matching policy denies the request. Note: a CEL expression that fails to evaluate is not
	// considered to match, making this a risky policy; prefer to use `Require`.
	//
	// The presence of at least one `Allow` rule triggers a deny-by-default policy, requiring at least 1 match to allow.
	// With no rules, all requires are allowed.
	Policy AuthorizationPolicy `json:"policy"`

	// `action` defines whether the rule allows, denies, or requires the request if
	// matched. If unspecified, the default is `Allow`.
	// Require policies are conjunctive across merged policies: all require policies must match.
	// +kubebuilder:validation:Enum=Allow;Deny;Require
	// +kubebuilder:default=Allow
	// +optional
	Action AuthorizationPolicyAction `json:"action,omitempty"`
}

// CELExpression represents a Common Expression Language (CEL) expression.
// +kubebuilder:validation:MinLength=1
// +kubebuilder:validation:MaxLength=16384
// +k8s:deepcopy-gen=false
type CELExpression string

// AuthorizationPolicy defines a single authorization rule.
type AuthorizationPolicy struct {
	// MatchExpressions defines a set of conditions that must be satisfied for the rule to match.
	// These expressions should be in the form of a Common Expression Language
	// (`CEL`) expression.
	//
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=256
	// +required
	MatchExpressions []CELExpression `json:"matchExpressions"`
}

// AuthorizationPolicyAction defines the action to take when the
// `RBACPolicies` matches.
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

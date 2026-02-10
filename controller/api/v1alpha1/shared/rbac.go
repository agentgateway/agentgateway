package shared

// Authorization defines the configuration for role-based access control.
type Authorization struct {
	// Policy specifies the Authorization rule to evaluate.
	// A policy matches when **any** of the conditions evaluates to true.
	// +required
	Policy AuthorizationPolicy `json:"policy"`

	// Action defines whether the rule allows or denies the request if matched.
	// If unspecified, the default is "Allow".
	// +kubebuilder:validation:Enum=Allow;Deny
	// +kubebuilder:default=Allow
	// +optional
	Action AuthorizationPolicyAction `json:"action,omitempty"`
}

// CELExpression represents a Common Expression Language (CEL) expression.
// +kubebuilder:validation:MinLength=1
// +kubebuilder:validation:MaxLength=16384
// +k8s:deepcopy-gen=false
type CELExpression string

// AuthorizationPolicy defines a single Authorization rule.
type AuthorizationPolicy struct {
	// MatchExpressions defines a set of conditions that must be satisfied for the rule to match.
	// These expression should be in the form of a Common Expression Language (CEL) expression.
	//
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=256
	// +required
	MatchExpressions []CELExpression `json:"matchExpressions"`
}

// AuthorizationPolicyAction defines the action to take when the RBACPolicies matches.
type AuthorizationPolicyAction string

const (
	// AuthorizationPolicyActionAllow defines the action to take when the RBACPolicies matches.
	AuthorizationPolicyActionAllow AuthorizationPolicyAction = "Allow"
	// AuthorizationPolicyActionDeny denies the action to take when the RBACPolicies matches.
	AuthorizationPolicyActionDeny AuthorizationPolicyAction = "Deny"
)

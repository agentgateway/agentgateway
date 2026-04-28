// +kubebuilder:object:generate=true
package embeddedgeneric

import "github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"

// List of conditionals
// +kubebuilder:object:generate=false
type ConditionalPolicyList[T any] = []Conditional[T]

// +kubebuilder:object:generate=false
type Conditional[T any] struct {
	// `condition` must evaluate to true for this policy to execute.
	// +required
	Condition shared.CELExpression `json:"condition"`
	// `policy` definition.
	// +required
	Policy T `json:"policy"`
}

// +kubebuilder:validation:ExactlyOneFieldSet
type MyPolicyOrConditional struct {
	MyPolicy
	Conditions []Conditional[MyPolicy]
}

type MyPolicy struct {
	Field string `json:"field"`
}

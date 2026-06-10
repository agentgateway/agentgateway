package shared

import metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

type Timeouts struct {
	// Timeout for an individual request from the gateway to a backend.
	// The timeout starts after the full downstream request has been received
	// and ends when the backend response has been fully processed.
	// A value of `0` effectively disables the timeout.
	// Specify this as a sequence of decimal numbers, each with optional
	// fraction and a unit suffix, such as `1s` or `500ms`.
	// +optional
	//
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	Request *metav1.Duration `json:"request,omitempty"`

	// Timeout for idle request or response streams.
	// A value of `0` effectively disables the timeout.
	// Specify this as a sequence of decimal numbers, each with optional
	// fraction and a unit suffix, such as `1s` or `500ms`.
	// +optional
	//
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	StreamIdle *metav1.Duration `json:"streamIdle,omitempty"`
}

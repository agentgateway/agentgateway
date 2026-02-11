package shared

import metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

type Timeouts struct {
	// Request specifies a timeout for an individual request from the gateway to a backend.
	// This spans between the point at which the entire downstream request (i.e. end-of-stream) has been
	// processed and when the backend response has been completely processed.
	// A value of 0 effectively disables the timeout.
	// It is specified as a sequence of decimal numbers, each with optional fraction and a unit suffix, such as "1s" or "500ms".
	// +optional
	//
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	Request *metav1.Duration `json:"request,omitempty"`

	// StreamIdle specifies a timeout for a requests' idle streams.
	// A value of 0 effectively disables the timeout.
	// +optional
	//
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	StreamIdle *metav1.Duration `json:"streamIdle,omitempty"`
}

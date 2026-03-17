package root

import "github.com/agentgateway/agentgateway/controller/hack/crdgen/testdata/embedded"

// +kubebuilder:validation:AtLeastOneFieldSet
type Wrapper struct {
	embedded.InlineFields `json:",inline"`
	Baz                   *string `json:"baz,omitempty"`
}

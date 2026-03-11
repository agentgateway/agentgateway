package v1

import (
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	embedded "github.com/agentgateway/agentgateway/controller/hack/crdgen/testdata/ifthenembedded/embedded"
)

// +kubebuilder:object:root=true
// +kubebuilder:resource:path=widgets,scope=Namespaced
type Widget struct {
	metav1.TypeMeta   `json:",inline"`
	metav1.ObjectMeta `json:"metadata,omitempty"`

	Spec WidgetSpec `json:"spec,omitempty"`
}

type WidgetSpec struct {
	Traffic Traffic `json:"traffic,omitempty"`
}

// +kubebuilder:validation:IfThenOnlyFields:if="has(self.baz)",fields=foo;baz,message="when baz is set only foo and baz may be set"
type Traffic struct {
	embedded.InlineFields `json:",inline"`
	Baz                   *string `json:"baz,omitempty"`
}

package v1

import (
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/hack/crdgen/testdata/overrideembedded/upstream"
)

// +kubebuilder:object:root=true
// +kubebuilder:resource:path=widgets,scope=Namespaced
type Widget struct {
	metav1.TypeMeta   `json:",inline"`
	metav1.ObjectMeta `json:"metadata,omitempty"`

	Spec WidgetSpec `json:"spec,omitempty"`
}

type WidgetSpec struct {
	Policies *LocalPolicies `json:"policies,omitempty"`
}

// +kubebuilder:validation:AtLeastOneFieldSet
// +kubebuilder:validation:OverrideXValidation:messageContains="at least one of the fields in [ai health]"
type LocalPolicies struct {
	upstream.FullPolicy `json:",inline"`

	Token *string `json:"token,omitempty"`
}

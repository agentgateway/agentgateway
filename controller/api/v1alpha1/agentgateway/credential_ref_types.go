package agentgateway

// LocalCredentialRef references a same-namespace credential.
// A name-only ref preserves the previous Secret reference wire shape.
//
// +structType=atomic
// +kubebuilder:validation:XValidation:rule="(!has(self.group) || size(self.group) == 0) ? (!has(self.kind) || size(self.kind) == 0 || self.kind == 'Secret') : (has(self.kind) && size(self.kind) > 0)",message="custom credential refs must set both group and kind"
type LocalCredentialRef struct {
	// Name of the referenced credential.
	// +optional
	// +default=""
	// +kubebuilder:default=""
	Name string `json:"name,omitempty"`

	// Group of the referenced credential. Empty selects the core API group.
	// +optional
	Group string `json:"group,omitempty"`

	// Kind of the referenced credential. Empty defaults to Secret.
	// +optional
	Kind string `json:"kind,omitempty"`
}

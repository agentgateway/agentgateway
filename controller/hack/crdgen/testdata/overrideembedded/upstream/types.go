package upstream

type SimplePolicy struct {
	Auth *string `json:"auth,omitempty"`
	HTTP *string `json:"http,omitempty"`
}

// +kubebuilder:validation:AtLeastOneFieldSet
type FullPolicy struct {
	SimplePolicy `json:",inline"`

	AI     *string `json:"ai,omitempty"`
	Health *string `json:"health,omitempty"`
}

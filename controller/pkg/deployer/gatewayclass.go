package deployer

import (
	"cmp"
	"slices"

	"k8s.io/apimachinery/pkg/util/sets"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	"sigs.k8s.io/gateway-api/pkg/features"
)

// GatewayClassInfo describes the desired configuration for a GatewayClass.
type GatewayClassInfo struct {
	// Description is a human-readable description of the GatewayClass.
	Description string
	// Labels are the labels to be added to the GatewayClass.
	Labels map[string]string
	// Annotations are the annotations to be added to the GatewayClass.
	Annotations map[string]string
	// ParametersRef is the reference to the GatewayParameters object.
	ParametersRef *gwv1.ParametersReference
	// ControllerName is the name of the controller that is managing the GatewayClass.
	ControllerName string
	// SupportedFeatures is the list of Gateway API features supported by this GatewayClass.
	// This will be populated in the GatewayClass status.supportedFeatures field.
	SupportedFeatures []gwv1.SupportedFeature
}

// GetSupportedFeaturesForAgentGateway returns the supported features for the agent Gateway class.
// Agent gateways support additional features beyond the standard gateway class.
func GetSupportedFeaturesForAgentGateway() []gwv1.SupportedFeature {
	exemptFeatures := GetCommonExemptFeatures()
	return getSupportedFeatures(exemptFeatures)
}

// GetCommonExemptFeatures returns the set of features that are commonly unsupported across all gateway classes.
// Exported for use in the conformance test suite.
func GetCommonExemptFeatures() sets.Set[features.Feature] {
	exemptFeatures := sets.New[features.Feature]()
	// we don't support any mesh features at all.
	for _, feature := range features.MeshCoreFeatures.UnsortedList() {
		exemptFeatures.Insert(feature)
	}
	for _, feature := range features.MeshExtendedFeatures.UnsortedList() {
		exemptFeatures.Insert(feature)
	}
	// we don't support the BackendTLSPolicy feature at all.
	for _, feature := range features.BackendTLSPolicyCoreFeatures.UnsortedList() {
		exemptFeatures.Insert(feature)
	}
	for _, feature := range features.BackendTLSPolicyExtendedFeatures.UnsortedList() {
		exemptFeatures.Insert(feature)
	}
	return exemptFeatures
}

// getSupportedFeatures builds a sorted list of supported features, excluding the provided exempt features.
func getSupportedFeatures(exemptFeatures sets.Set[features.Feature]) []gwv1.SupportedFeature {
	var allSupportedFeatures []gwv1.SupportedFeature
	for _, feature := range features.AllFeatures.UnsortedList() {
		if exemptFeatures.Has(feature) {
			continue
		}
		allSupportedFeatures = append(allSupportedFeatures, gwv1.SupportedFeature{
			Name: gwv1.FeatureName(feature.Name),
		})
	}
	slices.SortFunc(allSupportedFeatures, func(a, b gwv1.SupportedFeature) int {
		return cmp.Compare(a.Name, b.Name)
	})
	return allSupportedFeatures
}

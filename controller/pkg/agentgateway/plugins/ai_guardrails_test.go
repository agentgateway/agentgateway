package plugins

import (
	"strings"
	"testing"

	"istio.io/istio/pkg/kube/krt"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	apisettings "github.com/agentgateway/agentgateway/controller/api/settings"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

// The inline (deprecated) path needs no reference resolution, so an empty context is fine.
func TestProcessBedrockGuardrailsInline(t *testing.T) {
	got, err := processBedrockGuardrails(PolicyCtx{}, "ns", &agentgateway.BedrockGuardrails{
		GuardrailIdentifier: "gid",
		GuardrailVersion:    "DRAFT",
		Region:              "us-west-2",
	})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if got.BackendRef != nil {
		t.Fatalf("expected no backendRef on the inline path, got %v", got.BackendRef)
	}
	// This test exists to verify the deprecated inline fields are still emitted on the
	// back-compat path, so referencing them here is intentional.
	//nolint:staticcheck // asserting deprecated inline fields are populated
	if got.Identifier != "gid" || got.Version != "DRAFT" || got.Region != "us-west-2" {
		t.Fatalf("unexpected inline fields: %+v", got)
	}
}

func TestProcessGoogleModelArmorInlineDefaultsLocation(t *testing.T) {
	got, err := processGoogleModelArmor(PolicyCtx{}, "ns", &agentgateway.GoogleModelArmor{
		TemplateID: "tid",
		ProjectID:  "pid",
	})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if got.GetLocation() != "us-central1" {
		t.Fatalf("expected default location us-central1, got %q", got.GetLocation())
	}
}

// When backendRef is set the inline fields must be skipped and resolution must run
// through BuildBackendRef. We use a grant checker that denies the reference, proving the
// backendRef branch is taken (it errors at resolution rather than emitting inline fields).
func TestProcessBedrockGuardrailsBackendRefRoutesThroughResolution(t *testing.T) {
	ns := gwv1.Namespace("backend-ns")
	_, err := processBedrockGuardrails(PolicyCtx{
		Krt: krt.TestingDummyContext{},
		Collections: &AgwCollections{
			Settings: apisettings.Settings{
				BackendRefGrantMode: apisettings.BackendRefGrantModeRouteAndPolicy,
			},
		},
		Grants: &recordingGrantChecker{},
	}, "policy-ns", &agentgateway.BedrockGuardrails{
		BackendRef: &gwv1.BackendObjectReference{
			Name:      "my-guard",
			Namespace: &ns,
		},
	})
	if err == nil {
		t.Fatal("expected backendRef resolution to error under a denying grant checker")
	}
	if !strings.Contains(err.Error(), "backendRef") {
		t.Fatalf("error %q should mention backendRef", err)
	}
}

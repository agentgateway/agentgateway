package plugins

import (
	"strings"
	"testing"

	"istio.io/istio/pkg/kube/krt"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwv1b1 "sigs.k8s.io/gateway-api/apis/v1beta1"

	apisettings "github.com/agentgateway/agentgateway/controller/api/settings"
)

type recordingGrantChecker struct {
	source schema.GroupVersionKind
}

func (r *recordingGrantChecker) SecretAllowed(krt.HandlerContext, schema.GroupVersionKind, types.NamespacedName, string) bool {
	return false
}

func (r *recordingGrantChecker) BackendAllowed(
	_ krt.HandlerContext,
	source schema.GroupVersionKind,
	_ gwv1b1.ObjectName,
	_ gwv1b1.Namespace,
	_ string,
	_ schema.GroupKind,
	_ apisettings.BackendRefGrantMode,
) bool {
	r.source = source
	return false
}

func TestBuildBackendRefUsesPolicySourceGVKForReferenceGrant(t *testing.T) {
	source := schema.GroupVersionKind{
		Group:   "example.io",
		Version: "v1",
		Kind:    "CustomPolicy",
	}
	grants := &recordingGrantChecker{}
	namespace := new(gwv1.Namespace)
	*namespace = "backend-ns"

	_, err := BuildBackendRef(PolicyCtx{
		Krt: krt.TestingDummyContext{},
		Collections: &AgwCollections{
			Settings: apisettings.Settings{
				BackendRefGrantMode: apisettings.BackendRefGrantModeRouteAndPolicy,
			},
		},
		Grants:    grants,
		SourceGVK: source,
	}, gwv1.BackendObjectReference{
		Name:      "backend",
		Namespace: namespace,
	}, "policy-ns")

	if err == nil {
		t.Fatal("expected ReferenceGrant error")
	}
	if grants.source != source {
		t.Fatalf("source GVK = %v, want %v", grants.source, source)
	}
	if !strings.Contains(err.Error(), "CustomPolicy") {
		t.Fatalf("error %q does not include source kind", err)
	}
}

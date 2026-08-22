package jwks_test

import (
	"testing"

	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwv1b1 "sigs.k8s.io/gateway-api/apis/v1beta1"

	apisettings "github.com/agentgateway/agentgateway/controller/api/settings"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/jwks"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

func TestResolverBackendRefGrant(t *testing.T) {
	const (
		sourceNS = "source-ns"
		targetNS = "target-ns"
	)
	targetNamespace := gwv1.Namespace(targetNS)
	targetName := gwv1b1.ObjectName("jwks")
	otherName := gwv1b1.ObjectName("other")

	tests := []struct {
		name         string
		mode         apisettings.BackendRefGrantMode
		ownerKind    jwks.OwnerKind
		ownerNS      string
		refNamespace *gwv1.Namespace
		grant        *gwv1b1.ReferenceGrant
		wantErr      bool
	}{
		{
			name:         "route mode does not require policy grants",
			mode:         apisettings.BackendRefGrantModeRoute,
			ownerKind:    jwks.OwnerKindPolicy,
			ownerNS:      sourceNS,
			refNamespace: &targetNamespace,
		},
		{
			name:         "none mode does not require grants",
			mode:         apisettings.BackendRefGrantModeNone,
			ownerKind:    jwks.OwnerKindPolicy,
			ownerNS:      sourceNS,
			refNamespace: &targetNamespace,
		},
		{
			name:         "same namespace does not require a grant",
			mode:         apisettings.BackendRefGrantModeRouteAndPolicy,
			ownerKind:    jwks.OwnerKindPolicy,
			ownerNS:      targetNS,
			refNamespace: &targetNamespace,
		},
		{
			name:         "missing grant rejects cross namespace reference",
			mode:         apisettings.BackendRefGrantModeRouteAndPolicy,
			ownerKind:    jwks.OwnerKindPolicy,
			ownerNS:      sourceNS,
			refNamespace: &targetNamespace,
			wantErr:      true,
		},
		{
			name:         "matching policy grant allows reference",
			mode:         apisettings.BackendRefGrantModeRouteAndPolicy,
			ownerKind:    jwks.OwnerKindPolicy,
			ownerNS:      sourceNS,
			refNamespace: &targetNamespace,
			grant:        referenceGrant(targetNS, wellknown.AgentgatewayPolicyGVK.GroupKind(), sourceNS, &targetName),
		},
		{
			name:         "matching backend grant uses backend source kind",
			mode:         apisettings.BackendRefGrantModeRouteAndPolicy,
			ownerKind:    jwks.OwnerKindBackend,
			ownerNS:      sourceNS,
			refNamespace: &targetNamespace,
			grant:        referenceGrant(targetNS, wellknown.AgentgatewayBackendGVK.GroupKind(), sourceNS, nil),
		},
		{
			name:         "grant for a different target is rejected",
			mode:         apisettings.BackendRefGrantModeRouteAndPolicy,
			ownerKind:    jwks.OwnerKindPolicy,
			ownerNS:      sourceNS,
			refNamespace: &targetNamespace,
			grant:        referenceGrant(targetNS, wellknown.AgentgatewayPolicyGVK.GroupKind(), sourceNS, &otherName),
			wantErr:      true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			inputs := []any{&corev1.Service{
				Name: string(targetName), Namespace: targetNS,
				Spec: corev1.ServiceSpec{Ports: []corev1.ServicePort{{Port: 80}}},
			}}
			if tt.grant != nil {
				inputs = append(inputs, tt.grant)
			}
			collections := testutils.BuildMockCollection(t, inputs)
			resolver := jwks.NewResolver(
				testutils.BuildRemoteHTTPResolver(collections),
				testutils.BuildReferenceGrants(collections),
				tt.mode,
			)

			_, err := resolver.ResolveOwner(krt.TestingDummyContext{}, jwks.RemoteJwksOwner{
				ID: jwks.JwksOwnerID{
					Kind:      tt.ownerKind,
					Namespace: tt.ownerNS,
					Name:      "owner",
				},
				DefaultNamespace: tt.ownerNS,
				Remote: agentgateway.RemoteJWKS{
					JwksPath: ptr.Of(agentgateway.LongString("keys")),
					PolicyBackendEndpoint: agentgateway.PolicyBackendEndpoint{
						BackendRef: &gwv1.BackendObjectReference{
							Name:      gwv1.ObjectName(targetName),
							Namespace: tt.refNamespace,
							Port:      new(gwv1.PortNumber(80)),
						},
					},
				},
			})

			if tt.wantErr {
				require.ErrorContains(t, err, "missing a ReferenceGrant?")
			} else {
				require.NoError(t, err)
			}
		})
	}
}

func referenceGrant(namespace string, source schema.GroupKind, sourceNamespace string, targetName *gwv1b1.ObjectName) *gwv1b1.ReferenceGrant {
	return &gwv1b1.ReferenceGrant{
		Name: "allow-jwks", Namespace: namespace,
		Spec: gwv1b1.ReferenceGrantSpec{
			From: []gwv1b1.ReferenceGrantFrom{{
				Group:     gwv1b1.Group(source.Group),
				Kind:      gwv1b1.Kind(source.Kind),
				Namespace: gwv1b1.Namespace(sourceNamespace),
			}},
			To: []gwv1b1.ReferenceGrantTo{{
				Group: gwv1b1.Group(wellknown.ServiceGVK.Group),
				Kind:  gwv1b1.Kind(wellknown.ServiceGVK.Kind),
				Name:  targetName,
			}},
		},
	}
}

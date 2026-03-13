package agentjwksstore

import (
	"testing"

	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/krt"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/identity/jwks"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

func TestCollectPolicyJWKSSourcesUsesBackendScopedMCPAuthentication(t *testing.T) {
	opts := krtutil.NewKrtOptions(t.Context().Done(), nil)
	backendCol := krt.NewStaticCollection[*agentgateway.AgentgatewayBackend](
		nil,
		[]*agentgateway.AgentgatewayBackend{
			{
				ObjectMeta: metav1.ObjectMeta{
					Namespace: "default",
					Name:      "backend-mcp",
				},
				Spec: agentgateway.AgentgatewayBackendSpec{
					MCP: &agentgateway.MCPBackend{},
					Policies: &agentgateway.BackendFull{
						MCP: &agentgateway.BackendMCP{
							Authentication: &agentgateway.MCPAuthentication{
								JWKS: agentgateway.RemoteJWKS{
									JwksPath: "/backend-jwks",
									BackendRef: gwv1.BackendObjectReference{
										Name: gwv1.ObjectName("backend-service"),
									},
								},
							},
						},
					},
				},
			},
		},
		opts.ToOptions("Backends")...,
	)
	policy := &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      "gw-policy",
		},
		Spec: agentgateway.AgentgatewayPolicySpec{
			Backend: &agentgateway.BackendFull{
				MCP: &agentgateway.BackendMCP{
					Authentication: &agentgateway.MCPAuthentication{
						JWKS: agentgateway.RemoteJWKS{
							JwksPath: "/policy-jwks",
							BackendRef: gwv1.BackendObjectReference{
								Name: gwv1.ObjectName("policy-service"),
							},
						},
					},
				},
			},
		},
	}

	got := collectPolicyJWKSSources(
		krt.TestingDummyContext{},
		policy,
		plugins.PolicyCtx{Krt: krt.TestingDummyContext{}},
		backendCol,
		func(_ krt.HandlerContext, _, _ string, remoteProvider *agentgateway.RemoteJWKS) *jwks.JwksSource {
			return &jwks.JwksSource{
				JwksURL: remoteProvider.JwksPath + ":" + string(remoteProvider.BackendRef.Name),
			}
		},
	)

	require.Len(t, got, 2)
	require.ElementsMatch(t, []string{
		"/policy-jwks:policy-service",
		"/backend-jwks:backend-service",
	}, []string{got[0].JwksURL, got[1].JwksURL})
}

func TestCollectPolicyJWKSSourcesSkipsBackendJWKSWhenBackendHasNoMCP(t *testing.T) {
	opts := krtutil.NewKrtOptions(t.Context().Done(), nil)
	backendCol := krt.NewStaticCollection[*agentgateway.AgentgatewayBackend](
		nil,
		[]*agentgateway.AgentgatewayBackend{
			{
				ObjectMeta: metav1.ObjectMeta{
					Namespace: "default",
					Name:      "non-mcp-backend",
				},
				Spec: agentgateway.AgentgatewayBackendSpec{
					Policies: &agentgateway.BackendFull{
						MCP: &agentgateway.BackendMCP{
							Authentication: &agentgateway.MCPAuthentication{
								JWKS: agentgateway.RemoteJWKS{
									JwksPath: "/backend-jwks",
									BackendRef: gwv1.BackendObjectReference{
										Name: gwv1.ObjectName("backend-service"),
									},
								},
							},
						},
					},
				},
			},
		},
		opts.ToOptions("Backends")...,
	)

	got := collectPolicyJWKSSources(
		krt.TestingDummyContext{},
		&agentgateway.AgentgatewayPolicy{
			ObjectMeta: metav1.ObjectMeta{
				Namespace: "default",
				Name:      "gw-policy",
			},
		},
		plugins.PolicyCtx{Krt: krt.TestingDummyContext{}},
		backendCol,
		func(_ krt.HandlerContext, _, _ string, remoteProvider *agentgateway.RemoteJWKS) *jwks.JwksSource {
			return &jwks.JwksSource{JwksURL: remoteProvider.JwksPath}
		},
	)

	require.Empty(t, got)
}

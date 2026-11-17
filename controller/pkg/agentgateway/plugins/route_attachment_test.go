package plugins

import (
	"testing"

	"github.com/stretchr/testify/require"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
)

func TestRouteAttachmentEquality(t *testing.T) {
	a := RouteAttachment{
		To:      utils.TypedNamespacedName{Kind: "TestListenerResource", Namespace: "default", Name: "parent"},
		Gateway: types.NamespacedName{Namespace: "default", Name: "gateway"},
	}
	b := a
	require.True(t, a.Equals(&b))
	b.To.Name = "other-parent"
	require.False(t, a.Equals(&b))
	require.NotEqual(t, a.ResourceName(), b.ResourceName())
	b = a
	b.Gateway.Name = "other-gateway"
	require.False(t, a.Equals(&b))
	require.NotEqual(t, a.ResourceName(), b.ResourceName())
}

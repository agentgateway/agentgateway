package plugins

import (
	"testing"

	"github.com/stretchr/testify/require"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
)

func TestRouteAttachmentParentRefEquality(t *testing.T) {
	a := RouteAttachment{
		To: utils.TypedNamespacedName{Kind: "Gateway", Namespace: "default", Name: "gateway"},
		ParentRef: RouteAttachmentParent{
			GroupKind:      schema.GroupKind{Group: "first.example", Kind: "Gateway"},
			NamespacedName: types.NamespacedName{Namespace: "default", Name: "parent"},
		},
	}
	b := a
	require.True(t, a.Equals(&b))
	b.ParentRef.Group = "second.example"
	require.False(t, a.Equals(&b))
	// Parent metadata must not change the existing attachment identity.
	require.Equal(t, a.ResourceName(), b.ResourceName())
}

package plugins

import (
	"testing"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
)

// TestAttachmentNameGatewayPortListenerNoCollision guards against a numeric listener
// name (SectionName allows e.g. "443") producing the same attachment key suffix as a
// port of the same value. attachmentName feeds clone.Key in ClonePoliciesForTarget,
// so a collision would silently drop one attachment.
func TestAttachmentNameGatewayPortListenerNoCollision(t *testing.T) {
	listener := "443"
	port := int32(443)

	listenerName := attachmentName(&api.PolicyTarget{Kind: utils.GatewayTarget("ns", "gw", &listener, nil)})
	portName := attachmentName(&api.PolicyTarget{Kind: utils.GatewayTarget[string]("ns", "gw", nil, &port)})

	if listenerName == portName {
		t.Fatalf("attachmentName collision: listener %q and port %d both produced %q", listener, port, listenerName)
	}
}

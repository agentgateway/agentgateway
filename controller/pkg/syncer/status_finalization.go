package syncer

import (
	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/translator"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/syncer/status"
)

// FinalStatusCollectionsConfig provides reactive inputs for extension-owned status.
// The callback runs once during graph construction, not after collection sync.
// Build leaf collections with krt.Fetch dependencies on these inputs and any early
// extension collections; do not feed final status back into listener or route inputs.
// Register only final status, not both initial and final writers for the same object.
type FinalStatusCollectionsConfig struct {
	ControllerName string
	// GatewayListeners is the unfiltered arbitration output, including losers.
	GatewayListeners krt.Collection[*translator.GatewayListener]
	// RejectedListenerSets contains contribution admission verdicts, not conflicts.
	RejectedListenerSets krt.Collection[RejectedListenerSet]
	// RouteAttachments accounts for logical attachments, not emitted proxy routes.
	RouteAttachments  krt.Collection[*plugins.RouteAttachment]
	StatusCollections *status.StatusCollections
	KrtOpts           krtutil.KrtOptions
}

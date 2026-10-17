package proxyfeedback

import (
	"testing"

	discovery "github.com/envoyproxy/go-control-plane/envoy/service/discovery/v3"
	"google.golang.org/protobuf/types/known/anypb"
	"istio.io/istio/pilot/pkg/util/protoconv"
	"istio.io/istio/pkg/test/util/assert"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/api"
)

const testType = "type.googleapis.com/agentgateway.dev.resource.Resource"

var (
	testGateway = types.NamespacedName{Namespace: "default", Name: "gw"}
	testSource  = Source{Kind: "AgentgatewayPolicy", Namespace: "default", Name: "auth"}
)

func policyLookup() func(typeURL, name string) *anypb.Any {
	resource := protoconv.MessageToAny(&api.Resource{Kind: &api.Resource_Policy{Policy: &api.Policy{
		Key:  "auth",
		Name: &api.TypedResourceName{Kind: testSource.Kind, Namespace: testSource.Namespace, Name: testSource.Name},
	}}})
	return func(typeURL, name string) *anypb.Any {
		if name == "policy/auth" {
			return resource
		}
		return nil
	}
}

func resources(names ...string) []*discovery.Resource {
	out := make([]*discovery.Resource, 0, len(names))
	for _, name := range names {
		out = append(out, &discovery.Resource{Name: name})
	}
	return out
}

func badKey() Report {
	return Report{Resource: "policy/auth", Severity: SeverityWarning, Message: "bad key"}
}

func TestConnectionReportsUntilResent(t *testing.T) {
	tracker := NewTracker()
	conn := tracker.NewConnection(testGateway, policyLookup())

	conn.Sent(testType, "n1", resources("policy/auth", "policy/other"), nil)
	conn.Replied(testType, "n1", []Report{badKey()})
	assert.Equal(t, tracker.Snapshot(), []Diagnostic{{
		Gateway:  testGateway,
		Source:   testSource,
		Resource: "policy/auth",
		Severity: SeverityWarning,
		Message:  "bad key",
	}})

	// A response without the resource leaves its report in place.
	conn.Sent(testType, "n2", resources("policy/other"), nil)
	conn.Replied(testType, "n2", nil)
	assert.Equal(t, len(tracker.Snapshot()), 1)

	// Sending the resource again without a report clears it.
	conn.Sent(testType, "n3", resources("policy/auth"), nil)
	conn.Replied(testType, "n3", nil)
	assert.Equal(t, len(tracker.Snapshot()), 0)
}

func TestConnectionReplyToOlderResponse(t *testing.T) {
	tracker := NewTracker()
	conn := tracker.NewConnection(testGateway, policyLookup())

	// The fix is sent before the proxy replies to the broken version.
	conn.Sent(testType, "n1", resources("policy/auth"), nil)
	conn.Sent(testType, "n2", resources("policy/auth"), nil)
	conn.Replied(testType, "n1", []Report{badKey()})
	assert.Equal(t, len(tracker.Snapshot()), 1)
	conn.Replied(testType, "n2", nil)
	assert.Equal(t, len(tracker.Snapshot()), 0)
}

func TestConnectionRemovedResource(t *testing.T) {
	tracker := NewTracker()
	conn := tracker.NewConnection(testGateway, policyLookup())

	conn.Sent(testType, "n1", resources("policy/auth"), nil)
	conn.Replied(testType, "n1", []Report{badKey()})
	conn.Sent(testType, "n2", nil, []string{"policy/auth"})
	conn.Replied(testType, "n2", nil)
	assert.Equal(t, len(tracker.Snapshot()), 0)
}

func TestConnectionIgnoresUnknownNonce(t *testing.T) {
	tracker := NewTracker()
	conn := tracker.NewConnection(testGateway, policyLookup())

	// A reply to a response sent on an earlier connection says nothing about this one.
	conn.Replied(testType, "previous", []Report{badKey()})
	assert.Equal(t, len(tracker.Snapshot()), 0)
}

func TestTrackerCountsConnections(t *testing.T) {
	tracker := NewTracker()
	first := tracker.NewConnection(testGateway, policyLookup())
	second := tracker.NewConnection(testGateway, policyLookup())
	for _, conn := range []*Connection{first, second} {
		conn.Sent(testType, "n1", resources("policy/auth"), nil)
		conn.Replied(testType, "n1", []Report{badKey()})
	}
	assert.Equal(t, len(tracker.Snapshot()), 1)
	<-tracker.Changed()

	first.Close()
	assert.Equal(t, len(tracker.Snapshot()), 1)
	select {
	case <-tracker.Changed():
		t.Fatal("closing one of two reporting connections must not change the set")
	default:
	}

	second.Close()
	assert.Equal(t, len(tracker.Snapshot()), 0)
	<-tracker.Changed()
}

func TestConnectionUnknownSource(t *testing.T) {
	tracker := NewTracker()
	conn := tracker.NewConnection(testGateway, policyLookup())

	conn.Sent(testType, "n1", resources("backend/default/b"), nil)
	conn.Replied(testType, "n1", []Report{{Resource: "backend/default/b", Severity: SeverityError, Message: "bad"}})
	snapshot := tracker.Snapshot()
	assert.Equal(t, len(snapshot), 1)
	assert.Equal(t, snapshot[0].Source, Source{})
}

func TestTruncate(t *testing.T) {
	assert.Equal(t, truncate("short", 10), "short")
	// Cutting inside the two-byte "é" moves back to the rune start.
	assert.Equal(t, truncate("abcé", 4), "abc... (truncated)")
}

//go:build e2e

package a2a

import (
	"path/filepath"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/fsutils"
	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/tests/base"
)

type testingSuite struct {
	*base.BaseTestingSuite
}

// A2AMessage represents a message in the A2A protocol
type A2AMessage struct {
	Kind      string `json:"kind"`
	MessageID string `json:"messageId"`
	Parts     []struct {
		Kind string `json:"kind"`
		Text string `json:"text"`
	} `json:"parts"`
	Role string `json:"role"`
}
type A2ATaskResponse struct {
	JSONRPC string `json:"jsonrpc"`
	ID      string `json:"id"`
	Result  *struct {
		ContextID string       `json:"contextId"`
		History   []A2AMessage `json:"history"`
		ID        string       `json:"id"`
		Kind      string       `json:"kind"`
		Status    struct {
			Message   A2AMessage `json:"message"`
			State     string     `json:"state"`
			Timestamp string     `json:"timestamp"`
		} `json:"status"`
	} `json:"result,omitempty"`
	Error *struct {
		Code    int    `json:"code"`
		Message string `json:"message"`
	} `json:"error,omitempty"`
}
type A2AMessageResponse struct {
	JSONRPC string `json:"jsonrpc"`
	ID      string `json:"id"`
	Result  *struct {
		Kind      string `json:"kind"`
		MessageID string `json:"messageId"`
		Parts     []struct {
			Kind string `json:"kind"`
			Text string `json:"text"`
		} `json:"parts"`
		Role string `json:"role"`
	} `json:"result,omitempty"`
	Error *struct {
		Code    int    `json:"code"`
		Message string `json:"message"`
	} `json:"error,omitempty"`
}

// A2AAgentCard models the agent card discovery response
type A2AAgentCard struct {
	Name                              string   `json:"name"`
	Version                           string   `json:"version"`
	Description                       string   `json:"description"`
	ProtocolVersion                   string   `json:"protocolVersion"`
	PreferredTransport                string   `json:"preferredTransport"`
	URL                               string   `json:"url"`
	DefaultInputModes                 []string `json:"defaultInputModes"`
	DefaultOutputModes                []string `json:"defaultOutputModes"`
	SupportsAuthenticatedExtendedCard bool     `json:"supportsAuthenticatedExtendedCard"`
	Capabilities                      struct {
		Streaming bool `json:"streaming"`
	} `json:"capabilities"`
	Skills []struct {
		ID          string   `json:"id"`
		Name        string   `json:"name"`
		Description string   `json:"description"`
		Examples    []string `json:"examples"`
		Tags        []string `json:"tags"`
	} `json:"skills"`
}

const (
	// test namespace for proxy resources
	namespace = "agentgateway-base"
)

var (
	_ e2e.NewSuiteFunc = NewTestingSuite

	gatewayName = "gateway"

	setupManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "common.yaml")

	setup = base.TestCase{
		Manifests: []string{setupManifest},
	}
)

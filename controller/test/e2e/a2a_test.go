//go:build e2e

package e2e_test

import (
	"encoding/json"
	"fmt"
	"io"
	"strings"
	"testing"

	"github.com/google/uuid"
	"istio.io/istio/pkg/test/util/assert"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

func TestA2A(tt *testing.T) {
	t := New(tt)
	t.Apply(manifest("a2a", "common.yaml"))

	t.Run("AgentCard", func(t base.Test) {
		testA2AAgentCard(t)
	})
	t.Run("MessageSend", func(t base.Test) {
		testA2AMessageSend(t)
	})
	t.Run("HelloWorld", func(t base.Test) {
		testA2AHelloWorld(t)
	})
}

func testA2AAgentCard(t base.Test) {
	out, err := execCurlA2A(t, "/agent-card", a2aHeaders(), "")
	assert.NoError(t, err)

	var card a2aAgentCard
	assert.NoError(t, json.Unmarshal([]byte(strings.TrimSpace(out)), &card))

	assert.Equal(t, "Example A2A Agent", card.Name)
	assert.Equal(t, "1.0.0", card.Version)
	assert.Equal(t, "An example A2A agent using the a2a-protocol crate", card.Description)
	if len(card.Skills) < 1 {
		t.Fatal("expected at least one skill")
	}
}

func testA2AMessageSend(t base.Test) {
	request := buildMessageSendRequest("hello", "test-123")
	out, err := execCurlA2A(t, "/", a2aHeaders(), request)
	assert.NoError(t, err)

	var resp a2aTaskResponse
	assert.NoError(t, json.Unmarshal([]byte(strings.TrimSpace(out)), &resp))

	if resp.Error != nil {
		t.Fatalf("unexpected error in response: %+v", resp.Error)
	}
	if resp.Result == nil {
		t.Fatal("missing result")
	}
	assert.Equal(t, "task", resp.Result.Kind)
	assert.Equal(t, "working", resp.Result.Status.State)
	if len(resp.Result.History) < 1 {
		t.Fatal("expected at least one history item")
	}

	agentMessage := findAgentMessage(resp.Result.History)
	if agentMessage == nil {
		t.Fatal("expected agent response in history")
	}
	if len(agentMessage.Parts) < 1 {
		t.Fatal("expected at least one agent message part")
	}
}

func testA2AHelloWorld(t base.Test) {
	request := buildMessageSendRequest("hello world", "test-hello")
	out, err := execCurlA2A(t, "/", a2aHeaders(), request)
	assert.NoError(t, err)

	var resp a2aTaskResponse
	assert.NoError(t, json.Unmarshal([]byte(strings.TrimSpace(out)), &resp))

	if resp.Error != nil {
		t.Fatalf("unexpected error in response: %+v", resp.Error)
	}
	if resp.Result == nil {
		t.Fatal("missing result")
	}
	assert.Equal(t, "task", resp.Result.Kind)
	assert.Equal(t, "working", resp.Result.Status.State)

	agentMessage := findAgentMessage(resp.Result.History)
	if agentMessage == nil {
		t.Fatal("expected agent response in history")
	}
	if len(agentMessage.Parts) < 1 {
		t.Fatal("expected at least one agent message part")
	}
	if !strings.Contains(agentMessage.Parts[0].Text, "Echo") {
		t.Fatalf("expected Echo in response, got %q", agentMessage.Parts[0].Text)
	}
}

type a2aMessage struct {
	Kind      string `json:"kind"`
	MessageID string `json:"messageId"`
	Parts     []struct {
		Kind string `json:"kind"`
		Text string `json:"text"`
	} `json:"parts"`
	Role string `json:"role"`
}

type a2aTaskResponse struct {
	JSONRPC string `json:"jsonrpc"`
	ID      string `json:"id"`
	Result  *struct {
		ContextID string       `json:"contextId"`
		History   []a2aMessage `json:"history"`
		ID        string       `json:"id"`
		Kind      string       `json:"kind"`
		Status    struct {
			Message   a2aMessage `json:"message"`
			State     string     `json:"state"`
			Timestamp string     `json:"timestamp"`
		} `json:"status"`
	} `json:"result,omitempty"`
	Error *struct {
		Code    int    `json:"code"`
		Message string `json:"message"`
	} `json:"error,omitempty"`
}

type a2aAgentCard struct {
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

func buildMessageSendRequest(text string, id string) string {
	if id == "" {
		id = uuid.New().String()
	}
	messageID := uuid.New().String()
	taskID := fmt.Sprintf("task-%s", uuid.New().String())

	return fmt.Sprintf(`{
		"jsonrpc": "2.0",
		"id": "%s",
		"method": "tasks/send",
		"params": {
			"id": "%s",
			"message": {
				"kind": "message",
				"messageId": "%s",
				"role": "user",
				"parts": [
					{
						"kind": "text",
						"text": "%s"
					}
				]
			}
		}
	}`, id, taskID, messageID, text)
}

func a2aHeaders() map[string]string {
	return map[string]string{
		"Content-Type":  "application/json",
		"Accept":        "application/json",
		"Authorization": "Bearer secret-token",
	}
}

func execCurlA2A(t base.Test, path string, headers map[string]string, body string) (string, error) {
	curlOpts := []curl.Option{
		curl.WithPath(path),
	}
	for k, v := range headers {
		curlOpts = append(curlOpts, curl.WithHeader(k, v))
	}
	if body != "" {
		curlOpts = append(curlOpts, curl.WithBody(body))
	}

	resp := base.BaseGateway.SendWithResponse(t, &matchers.HttpResponse{
		StatusCode: 200,
	}, curlOpts...)
	defer resp.Body.Close()

	bodyBytes, err := io.ReadAll(resp.Body)
	if err != nil {
		t.Logf("read body error: %v", err)
		return "", err
	}
	return string(bodyBytes), nil
}

func findAgentMessage(history []a2aMessage) *a2aMessage {
	for _, msg := range history {
		if msg.Role == "agent" {
			return &msg
		}
	}
	return nil
}

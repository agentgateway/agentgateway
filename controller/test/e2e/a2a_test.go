//go:build e2e

package e2e_test

import (
	"encoding/json"
	"fmt"
	"io"
	"strings"
	"testing"

	"github.com/google/uuid"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

func TestA2A(t *testing.T) {
	agw := New(t)
	agw.Apply(manifest("a2a", "common.yaml"))

	agw.Run("AgentCard", func() {
		testA2AAgentCard(agw)
	})
	agw.Run("MessageSend", func() {
		testA2AMessageSend(agw)
	})
	agw.Run("HelloWorld", func() {
		testA2AHelloWorld(agw)
	})
}

func testA2AAgentCard(agw *base.BaseTestingSuite) {
	out, err := execCurlA2A(agw, "/agent-card", a2aHeaders(), "")
	agw.Require().NoError(err, "agent card curl failed")

	var card a2aAgentCard
	agw.Require().NoError(json.Unmarshal([]byte(strings.TrimSpace(out)), &card), "failed to parse agent card")

	agw.Require().Equal("Example A2A Agent", card.Name)
	agw.Require().Equal("1.0.0", card.Version)
	agw.Require().Equal("An example A2A agent using the a2a-protocol crate", card.Description)
	agw.Require().GreaterOrEqual(len(card.Skills), 1, "expected at least one skill")
}

func testA2AMessageSend(agw *base.BaseTestingSuite) {
	request := buildMessageSendRequest("hello", "test-123")
	out, err := execCurlA2A(agw, "/", a2aHeaders(), request)
	agw.Require().NoError(err, "tasks/send curl failed")

	var resp a2aTaskResponse
	agw.Require().NoError(json.Unmarshal([]byte(strings.TrimSpace(out)), &resp), "failed to parse response")

	agw.Require().Nil(resp.Error, "unexpected error in response")
	agw.Require().NotNil(resp.Result, "missing result")
	agw.Require().Equal("task", resp.Result.Kind)
	agw.Require().Equal("working", resp.Result.Status.State)
	agw.Require().GreaterOrEqual(len(resp.Result.History), 1)

	agentMessage := findAgentMessage(resp.Result.History)
	agw.Require().NotNil(agentMessage, "expected agent response in history")
	agw.Require().GreaterOrEqual(len(agentMessage.Parts), 1)
}

func testA2AHelloWorld(agw *base.BaseTestingSuite) {
	request := buildMessageSendRequest("hello world", "test-hello")
	out, err := execCurlA2A(agw, "/", a2aHeaders(), request)
	agw.Require().NoError(err, "hello world curl failed")

	var resp a2aTaskResponse
	agw.Require().NoError(json.Unmarshal([]byte(strings.TrimSpace(out)), &resp), "failed to parse response")

	agw.Require().Nil(resp.Error)
	agw.Require().NotNil(resp.Result)
	agw.Require().Equal("task", resp.Result.Kind)
	agw.Require().Equal("working", resp.Result.Status.State)

	agentMessage := findAgentMessage(resp.Result.History)
	agw.Require().NotNil(agentMessage, "expected agent response in history")
	agw.Require().GreaterOrEqual(len(agentMessage.Parts), 1)
	agw.Require().Contains(agentMessage.Parts[0].Text, "Echo", "expected Echo in response")
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

func execCurlA2A(t *base.BaseTestingSuite, path string, headers map[string]string, body string) (string, error) {
	curlOpts := []curl.Option{
		curl.WithPath(path),
	}
	for k, v := range headers {
		curlOpts = append(curlOpts, curl.WithHeader(k, v))
	}
	if body != "" {
		curlOpts = append(curlOpts, curl.WithBody(body))
	}

	resp := base.BaseGateway.SendWithResponse(t.T(), &matchers.HttpResponse{
		StatusCode: 200,
	}, curlOpts...)
	defer resp.Body.Close()

	bodyBytes, err := io.ReadAll(resp.Body)
	if err != nil {
		t.T().Logf("read body error: %v", err)
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

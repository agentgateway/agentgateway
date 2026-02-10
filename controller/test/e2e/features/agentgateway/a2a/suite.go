//go:build e2e

package a2a

import (
	"context"
	"encoding/json"
	"strings"

	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/tests/base"
)

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	return &testingSuite{
		BaseTestingSuite: base.NewBaseTestingSuite(ctx, testInst, setup, map[string]*base.TestCase{}),
	}
}

func (s *testingSuite) TestA2AAgentCard() {
	s.T().Log("Testing A2A agent card discovery")
	s.waitA2AEnvironmentReady()

	headers := a2aHeaders()
	out, err := s.execCurlA2A("/agent-card", headers, "")
	s.Require().NoError(err, "agent card curl failed")

	// Without -v, output is just the JSON response
	var card A2AAgentCard
	s.Require().NoError(json.Unmarshal([]byte(strings.TrimSpace(out)), &card), "failed to parse agent card")

	s.Require().Equal("Example A2A Agent", card.Name)
	s.Require().Equal("1.0.0", card.Version)
	s.Require().Equal("An example A2A agent using the a2a-protocol crate", card.Description)
	s.Require().GreaterOrEqual(len(card.Skills), 1, "expected at least one skill")

	s.T().Logf("Agent card: %s v%s with %d skills", card.Name, card.Version, len(card.Skills))
}

func (s *testingSuite) TestA2AMessageSend() {
	s.T().Log("Testing A2A tasks/send")
	s.waitA2AEnvironmentReady()

	request := buildMessageSendRequest("hello", "test-123")
	headers := a2aHeaders()

	out, err := s.execCurlA2A("/", headers, request)
	s.Require().NoError(err, "tasks/send curl failed")

	var resp A2ATaskResponse
	s.Require().NoError(json.Unmarshal([]byte(strings.TrimSpace(out)), &resp), "failed to parse response")

	s.Require().Nil(resp.Error, "unexpected error in response")
	s.Require().NotNil(resp.Result, "missing result")
	s.Require().Equal("task", resp.Result.Kind)
	s.Require().Equal("working", resp.Result.Status.State)
	s.Require().GreaterOrEqual(len(resp.Result.History), 1)

	// Find the agent response in history
	var agentMessage *A2AMessage
	for _, msg := range resp.Result.History {
		if msg.Role == "agent" {
			agentMessage = &msg
			break
		}
	}
	s.Require().NotNil(agentMessage, "expected agent response in history")
	s.Require().GreaterOrEqual(len(agentMessage.Parts), 1)

	s.T().Logf("Response: %s", agentMessage.Parts[0].Text)
}

func (s *testingSuite) TestA2AHelloWorld() {
	s.T().Log("Testing A2A Hello World skill")
	s.waitA2AEnvironmentReady()

	request := buildMessageSendRequest("hello world", "test-hello")
	headers := a2aHeaders()

	out, err := s.execCurlA2A("/", headers, request)
	s.Require().NoError(err, "hello world curl failed")

	var resp A2ATaskResponse
	s.Require().NoError(json.Unmarshal([]byte(strings.TrimSpace(out)), &resp), "failed to parse response")

	s.Require().Nil(resp.Error)
	s.Require().NotNil(resp.Result)
	s.Require().Equal("task", resp.Result.Kind)
	s.Require().Equal("working", resp.Result.Status.State)

	// Find the agent response in history
	var agentMessage *A2AMessage
	for _, msg := range resp.Result.History {
		if msg.Role == "agent" {
			agentMessage = &msg
			break
		}
	}
	s.Require().NotNil(agentMessage, "expected agent response in history")
	s.Require().GreaterOrEqual(len(agentMessage.Parts), 1)
	s.Require().Contains(agentMessage.Parts[0].Text, "Echo", "expected Echo in response")

	s.T().Logf("Response: %s", agentMessage.Parts[0].Text)
}

func (s *testingSuite) waitA2AEnvironmentReady() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyPodsRunning(
		s.Ctx, namespace,
		metav1.ListOptions{LabelSelector: "app=a2a-helloworld"},
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayCondition(
		s.Ctx, gatewayName, namespace,
		gwv1.GatewayConditionProgrammed, metav1.ConditionTrue,
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyPodsRunning(
		s.Ctx, namespace,
		metav1.ListOptions{LabelSelector: "app.kubernetes.io/name=" + gatewayName},
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx, "a2a-route", namespace,
		gwv1.RouteConditionAccepted, metav1.ConditionTrue,
	)
}

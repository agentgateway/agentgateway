package plugins

import (
	"testing"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

func TestContextCompressionFailureMode(t *testing.T) {
	cases := []struct {
		name string
		in   agentgateway.FailureMode
		want api.BackendPolicySpec_Ai_ContextCompression_FailureMode
	}{
		// Empty (unset) must default to FAIL_OPEN, matching the CRD default and the data-plane
		// default. FAIL_OPEN is proto 0, so an absent field also decodes to open downstream.
		{"empty defaults to open", "", api.BackendPolicySpec_Ai_ContextCompression_FAIL_OPEN},
		{"explicit open", agentgateway.FailOpen, api.BackendPolicySpec_Ai_ContextCompression_FAIL_OPEN},
		{"explicit closed", agentgateway.FailClosed, api.BackendPolicySpec_Ai_ContextCompression_FAIL_CLOSED},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := contextCompressionFailureMode(tc.in); got != tc.want {
				t.Fatalf("contextCompressionFailureMode(%q) = %v, want %v", tc.in, got, tc.want)
			}
		})
	}
}

func TestWebhookMessageFormat(t *testing.T) {
	cases := []struct {
		name string
		in   agentgateway.MessageFormat
		want api.BackendPolicySpec_Ai_Webhook_MessageFormat
	}{
		// Empty (unset) must default to SIMPLIFIED so existing webhook servers see the same
		// payload. SIMPLIFIED is proto 0, so an absent field also decodes to simplified.
		{"empty defaults to simplified", "", api.BackendPolicySpec_Ai_Webhook_SIMPLIFIED},
		{"explicit simplified", agentgateway.MessageFormatSimplified, api.BackendPolicySpec_Ai_Webhook_SIMPLIFIED},
		{"explicit raw", agentgateway.MessageFormatRaw, api.BackendPolicySpec_Ai_Webhook_RAW},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := webhookMessageFormat(tc.in); got != tc.want {
				t.Fatalf("webhookMessageFormat(%q) = %v, want %v", tc.in, got, tc.want)
			}
		})
	}
}

func TestProcessContextCompressionNil(t *testing.T) {
	cc, err := processContextCompression(PolicyCtx{}, "ns", nil)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if cc != nil {
		t.Fatalf("expected nil result for nil config, got %v", cc)
	}
}

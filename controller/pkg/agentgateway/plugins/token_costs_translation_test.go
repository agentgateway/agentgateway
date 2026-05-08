package plugins

import (
	"testing"

	"github.com/agentgateway/agentgateway/api"
	agwv1alpha1 "github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

// TestStringPtrToFloat64OrDefault verifies the helper used by translateBackendAI.
func TestStringPtrToFloat64OrDefault(t *testing.T) {
	t.Run("nil pointer returns default", func(t *testing.T) {
		got := stringPtrToFloat64OrDefault(nil)
		if got != 1.0 {
			t.Errorf("expected 1.0, got %v", got)
		}
	})

	t.Run("valid string returns parsed value", func(t *testing.T) {
		s := "5.25"
		got := stringPtrToFloat64OrDefault(&s)
		if got != 5.25 {
			t.Errorf("expected 5.25, got %v", got)
		}
	})

	t.Run("zero value returns default (must be positive)", func(t *testing.T) {
		s := "0.0"
		got := stringPtrToFloat64OrDefault(&s)
		if got != 1.0 {
			t.Errorf("expected 1.0 for non-positive, got %v", got)
		}
	})

	t.Run("negative value returns default", func(t *testing.T) {
		s := "-1.5"
		got := stringPtrToFloat64OrDefault(&s)
		if got != 1.0 {
			t.Errorf("expected 1.0 for negative, got %v", got)
		}
	})

	t.Run("unparseable string returns default", func(t *testing.T) {
		s := "not-a-number"
		got := stringPtrToFloat64OrDefault(&s)
		if got != 1.0 {
			t.Errorf("expected 1.0 for unparseable, got %v", got)
		}
	})
}

// TestTranslateTokenCosts verifies translation of TokenCosts from CRD fields
// to the proto BackendPolicySpec_Ai_TokenCosts struct.
func TestTranslateTokenCosts(t *testing.T) {
	t.Run("all fields set translate correctly", func(t *testing.T) {
		tc := &agwv1alpha1.TokenCosts{
			Input:      new("1.0"),
			Output:     new("5.0"),
			CacheWrite: new("1.25"),
			CacheRead:  new("0.1"),
		}

		got := &api.BackendPolicySpec_Ai_TokenCosts{
			Input:      stringPtrToFloat64OrDefault(tc.Input),
			Output:     stringPtrToFloat64OrDefault(tc.Output),
			CacheWrite: stringPtrToFloat64OrDefault(tc.CacheWrite),
			CacheRead:  stringPtrToFloat64OrDefault(tc.CacheRead),
		}

		if got.Input != 1.0 {
			t.Errorf("Input: expected 1.0, got %v", got.Input)
		}
		if got.Output != 5.0 {
			t.Errorf("Output: expected 5.0, got %v", got.Output)
		}
		if got.CacheWrite != 1.25 {
			t.Errorf("CacheWrite: expected 1.25, got %v", got.CacheWrite)
		}
		if got.CacheRead != 0.1 {
			t.Errorf("CacheRead: expected 0.1, got %v", got.CacheRead)
		}
	})

	t.Run("partial fields default to 1.0", func(t *testing.T) {
		// Only input is set; all others should default to 1.0.
		tc := &agwv1alpha1.TokenCosts{
			Input: new("3.0"),
			// Output, CacheWrite, CacheRead all nil
		}

		got := &api.BackendPolicySpec_Ai_TokenCosts{
			Input:      stringPtrToFloat64OrDefault(tc.Input),
			Output:     stringPtrToFloat64OrDefault(tc.Output),
			CacheWrite: stringPtrToFloat64OrDefault(tc.CacheWrite),
			CacheRead:  stringPtrToFloat64OrDefault(tc.CacheRead),
		}

		if got.Input != 3.0 {
			t.Errorf("Input: expected 3.0, got %v", got.Input)
		}
		if got.Output != 1.0 {
			t.Errorf("Output: expected default 1.0, got %v", got.Output)
		}
		if got.CacheWrite != 1.0 {
			t.Errorf("CacheWrite: expected default 1.0, got %v", got.CacheWrite)
		}
		if got.CacheRead != 1.0 {
			t.Errorf("CacheRead: expected default 1.0, got %v", got.CacheRead)
		}
	})

	t.Run("absent TokenCosts produces no proto struct (nil guard)", func(t *testing.T) {
		// Mirrors the guard in translateBackendAI: when aiSpec.TokenCosts is nil,
		// the translation block is skipped and the proto field remains unset.
		aiSpec := &agwv1alpha1.BackendAI{
			// TokenCosts intentionally left nil
		}

		var translatedTokenCosts *api.BackendPolicySpec_Ai_TokenCosts
		if aiSpec.TokenCosts != nil {
			translatedTokenCosts = &api.BackendPolicySpec_Ai_TokenCosts{
				Input:      stringPtrToFloat64OrDefault(aiSpec.TokenCosts.Input),
				Output:     stringPtrToFloat64OrDefault(aiSpec.TokenCosts.Output),
				CacheWrite: stringPtrToFloat64OrDefault(aiSpec.TokenCosts.CacheWrite),
				CacheRead:  stringPtrToFloat64OrDefault(aiSpec.TokenCosts.CacheRead),
			}
		}

		if translatedTokenCosts != nil {
			t.Error("expected nil TokenCosts to produce nil proto — translation must guard against nil")
		}
	})
}

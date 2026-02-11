package matchers_test

import (
	"strings"
	"testing"

	"istio.io/istio/pkg/test/util/assert"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

func TestHaveCondition(t *testing.T) {
	conditions := []metav1.Condition{
		{
			Type:   "Ready",
			Status: metav1.ConditionTrue,
			Reason: "AllGood",
		},
		{
			Type:   "Accepted",
			Status: metav1.ConditionTrue,
			Reason: "Accepted",
		},
		{
			Type:   "Degraded",
			Status: metav1.ConditionFalse,
			Reason: "NotDegraded",
		},
	}

	t.Run("matches when condition type and status match", func(t *testing.T) {
		mustMatch(t, matchers.HaveCondition("Ready", metav1.ConditionTrue), conditions)
	})
	t.Run("matches when condition is False", func(t *testing.T) {
		mustMatch(t, matchers.HaveCondition("Degraded", metav1.ConditionFalse), conditions)
	})
	t.Run("does not match when condition type is not found", func(t *testing.T) {
		mustNotMatch(t, matchers.HaveCondition("NonExistent", metav1.ConditionTrue), conditions)
	})
	t.Run("does not match when status differs", func(t *testing.T) {
		mustNotMatch(t, matchers.HaveCondition("Ready", metav1.ConditionFalse), conditions)
	})
	t.Run("does not match when conditions slice is empty", func(t *testing.T) {
		mustNotMatch(t, matchers.HaveCondition("Ready", metav1.ConditionTrue), []metav1.Condition{})
	})
	t.Run("provides informative failure message", func(t *testing.T) {
		matcher := matchers.HaveCondition("NonExistent", metav1.ConditionTrue)
		ok, err := matcher.Match(conditions)
		assert.NoError(t, err)
		assert.Equal(t, false, ok)
		msg := matcher.FailureMessage(conditions)
		assert.Equal(t, true, strings.Contains(msg, "NonExistent"))
		assert.Equal(t, true, strings.Contains(msg, "Ready"))
	})
}

func TestHaveAnyParentCondition(t *testing.T) {
	parentConditions := [][]metav1.Condition{
		{
			{Type: "Accepted", Status: metav1.ConditionTrue, Reason: "Accepted"},
			{Type: "ResolvedRefs", Status: metav1.ConditionTrue, Reason: "ResolvedRefs"},
		},
		{
			{Type: "Accepted", Status: metav1.ConditionFalse, Reason: "Rejected"},
			{Type: "ResolvedRefs", Status: metav1.ConditionFalse, Reason: "RefNotFound"},
		},
	}

	t.Run("matches when any parent has matching condition", func(t *testing.T) {
		mustMatch(t, matchers.HaveAnyParentCondition("Accepted", metav1.ConditionTrue), parentConditions)
	})
	t.Run("matches when second parent has matching condition", func(t *testing.T) {
		mustMatch(t, matchers.HaveAnyParentCondition("Accepted", metav1.ConditionFalse), parentConditions)
	})
	t.Run("does not match when no parent has matching condition", func(t *testing.T) {
		mustNotMatch(t, matchers.HaveAnyParentCondition("NonExistent", metav1.ConditionTrue), parentConditions)
	})
	t.Run("does not match when status does not match any parent", func(t *testing.T) {
		mustNotMatch(t, matchers.HaveAnyParentCondition("ResolvedRefs", metav1.ConditionUnknown), parentConditions)
	})
	t.Run("does not match when parents slice is empty", func(t *testing.T) {
		mustNotMatch(t, matchers.HaveAnyParentCondition("Accepted", metav1.ConditionTrue), [][]metav1.Condition{})
	})
}

func TestHaveAnyAncestorCondition(t *testing.T) {
	ancestorConditions := [][]metav1.Condition{
		{
			{Type: "Accepted", Status: metav1.ConditionTrue, Reason: "PolicyAccepted"},
		},
	}

	t.Run("matches when any ancestor has matching condition", func(t *testing.T) {
		mustMatch(t, matchers.HaveAnyAncestorCondition("Accepted", metav1.ConditionTrue), ancestorConditions)
	})
	t.Run("does not match when no ancestor has matching condition", func(t *testing.T) {
		mustNotMatch(t, matchers.HaveAnyAncestorCondition("NonExistent", metav1.ConditionTrue), ancestorConditions)
	})
}

func mustMatch(t *testing.T, matcher interface {
	Match(actual any) (bool, error)
	FailureMessage(actual any) string
}, actual any) {
	t.Helper()
	ok, err := matcher.Match(actual)
	assert.NoError(t, err)
	assert.Equal(t, true, ok, matcher.FailureMessage(actual))
}

func mustNotMatch(t *testing.T, matcher interface {
	Match(actual any) (bool, error)
	NegatedFailureMessage(actual any) string
}, actual any) {
	t.Helper()
	ok, err := matcher.Match(actual)
	assert.NoError(t, err)
	assert.Equal(t, false, ok, matcher.NegatedFailureMessage(actual))
}

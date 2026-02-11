package matchers

import (
	"fmt"
	"strings"

	"github.com/onsi/gomega/types"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
)

// ExpectedCondition defines the expected condition to match.
type ExpectedCondition struct {
	// Type is the condition type to look for.
	Type string

	// Status is the expected condition status.
	Status metav1.ConditionStatus

	// Reason is the expected condition reason. Optional.
	Reason string
}

// HaveCondition returns a matcher that checks if a slice of conditions contains
// a condition with the specified type and status.
func HaveCondition(conditionType string, status metav1.ConditionStatus) types.GomegaMatcher {
	return &conditionMatcher{
		expected: ExpectedCondition{
			Type:   conditionType,
			Status: status,
		},
	}
}

type conditionMatcher struct {
	expected   ExpectedCondition
	conditions []metav1.Condition
}

func (m *conditionMatcher) Match(actual any) (bool, error) {
	conditions, ok := actual.([]metav1.Condition)
	if !ok {
		return false, fmt.Errorf("HaveCondition matcher expects []metav1.Condition, got %T", actual)
	}
	m.conditions = conditions

	for i := range conditions {
		if conditions[i].Type == m.expected.Type {
			if conditions[i].Status != m.expected.Status {
				return false, nil
			}
			if m.expected.Reason != "" && conditions[i].Reason != m.expected.Reason {
				return false, nil
			}
			return true, nil
		}
	}
	return false, nil
}

func (m *conditionMatcher) FailureMessage(actual any) string {
	return fmt.Sprintf("Expected conditions to have condition type %q with status %q%s.\nActual conditions:\n%s",
		m.expected.Type, m.expected.Status, m.reasonSuffix(), m.formatConditions())
}

func (m *conditionMatcher) NegatedFailureMessage(actual any) string {
	return fmt.Sprintf("Expected conditions NOT to have condition type %q with status %q%s.\nActual conditions:\n%s",
		m.expected.Type, m.expected.Status, m.reasonSuffix(), m.formatConditions())
}

func (m *conditionMatcher) reasonSuffix() string {
	if m.expected.Reason != "" {
		return fmt.Sprintf(" and reason %q", m.expected.Reason)
	}
	return ""
}

func (m *conditionMatcher) formatConditions() string {
	if len(m.conditions) == 0 {
		return "  (no conditions)"
	}
	var sb strings.Builder
	for _, c := range m.conditions {
		sb.WriteString(fmt.Sprintf("  - Type: %s, Status: %s, Reason: %s\n", c.Type, c.Status, c.Reason))
	}
	return sb.String()
}

// ParentConditionsAccessor is an interface for objects that have parent status with conditions.
// This covers route types like HTTPRoute, TCPRoute, TLSRoute, GRPCRoute.
type ParentConditionsAccessor interface {
	GetParentConditions() [][]metav1.Condition
}

// HaveAnyParentCondition returns a matcher that checks if any parent status contains
// a condition with the specified type and status. This is useful for route resources
// that have Status.Parents[].Conditions.
func HaveAnyParentCondition(conditionType string, status metav1.ConditionStatus) types.GomegaMatcher {
	return &parentConditionMatcher{
		expected: ExpectedCondition{
			Type:   conditionType,
			Status: status,
		},
	}
}

type parentConditionMatcher struct {
	expected         ExpectedCondition
	parentConditions [][]metav1.Condition
}

func (m *parentConditionMatcher) Match(actual any) (bool, error) {
	parentConditions, ok := actual.([][]metav1.Condition)
	if !ok {
		return false, fmt.Errorf("HaveAnyParentCondition matcher expects [][]metav1.Condition, got %T", actual)
	}
	m.parentConditions = parentConditions

	for _, conditions := range parentConditions {
		for i := range conditions {
			if conditions[i].Type == m.expected.Type {
				if conditions[i].Status == m.expected.Status {
					if m.expected.Reason == "" || conditions[i].Reason == m.expected.Reason {
						return true, nil
					}
				}
			}
		}
	}
	return false, nil
}

func (m *parentConditionMatcher) FailureMessage(actual any) string {
	return fmt.Sprintf("Expected at least one parent to have condition type %q with status %q%s.\nParent conditions:\n%s",
		m.expected.Type, m.expected.Status, m.parentReasonSuffix(), m.formatParentConditions())
}

func (m *parentConditionMatcher) NegatedFailureMessage(actual any) string {
	return fmt.Sprintf("Expected NO parent to have condition type %q with status %q%s.\nParent conditions:\n%s",
		m.expected.Type, m.expected.Status, m.parentReasonSuffix(), m.formatParentConditions())
}

func (m *parentConditionMatcher) parentReasonSuffix() string {
	if m.expected.Reason != "" {
		return fmt.Sprintf(" and reason %q", m.expected.Reason)
	}
	return ""
}

func (m *parentConditionMatcher) formatParentConditions() string {
	if len(m.parentConditions) == 0 {
		return "  (no parents)"
	}
	var sb strings.Builder
	for i, conditions := range m.parentConditions {
		sb.WriteString(fmt.Sprintf("  Parent %d:\n", i))
		if len(conditions) == 0 {
			sb.WriteString("    (no conditions)\n")
			continue
		}
		for _, c := range conditions {
			sb.WriteString(fmt.Sprintf("    - Type: %s, Status: %s, Reason: %s\n", c.Type, c.Status, c.Reason))
		}
	}
	return sb.String()
}

// HaveAnyAncestorCondition returns a matcher that checks if any ancestor status contains
// a condition with the specified type and status. This is useful for policy resources
// that have Status.Ancestors[].Conditions.
func HaveAnyAncestorCondition(conditionType string, status metav1.ConditionStatus) types.GomegaMatcher {
	// Ancestors and Parents have the same structure, so we can reuse the same matcher
	return &ancestorConditionMatcher{
		expected: ExpectedCondition{
			Type:   conditionType,
			Status: status,
		},
	}
}

type ancestorConditionMatcher struct {
	expected           ExpectedCondition
	ancestorConditions [][]metav1.Condition
}

func (m *ancestorConditionMatcher) Match(actual any) (bool, error) {
	ancestorConditions, ok := actual.([][]metav1.Condition)
	if !ok {
		return false, fmt.Errorf("HaveAnyAncestorCondition matcher expects [][]metav1.Condition, got %T", actual)
	}
	m.ancestorConditions = ancestorConditions

	for _, conditions := range ancestorConditions {
		for i := range conditions {
			if conditions[i].Type == m.expected.Type {
				if conditions[i].Status == m.expected.Status {
					if m.expected.Reason == "" || conditions[i].Reason == m.expected.Reason {
						return true, nil
					}
				}
			}
		}
	}
	return false, nil
}

func (m *ancestorConditionMatcher) FailureMessage(actual any) string {
	return fmt.Sprintf("Expected at least one ancestor to have condition type %q with status %q%s.\nAncestor conditions:\n%s",
		m.expected.Type, m.expected.Status, m.ancestorReasonSuffix(), m.formatAncestorConditions())
}

func (m *ancestorConditionMatcher) NegatedFailureMessage(actual any) string {
	return fmt.Sprintf("Expected NO ancestor to have condition type %q with status %q%s.\nAncestor conditions:\n%s",
		m.expected.Type, m.expected.Status, m.ancestorReasonSuffix(), m.formatAncestorConditions())
}

func (m *ancestorConditionMatcher) ancestorReasonSuffix() string {
	if m.expected.Reason != "" {
		return fmt.Sprintf(" and reason %q", m.expected.Reason)
	}
	return ""
}

func (m *ancestorConditionMatcher) formatAncestorConditions() string {
	if len(m.ancestorConditions) == 0 {
		return "  (no ancestors)"
	}
	var sb strings.Builder
	for i, conditions := range m.ancestorConditions {
		sb.WriteString(fmt.Sprintf("  Ancestor %d:\n", i))
		if len(conditions) == 0 {
			sb.WriteString("    (no conditions)\n")
			continue
		}
		for _, c := range conditions {
			sb.WriteString(fmt.Sprintf("    - Type: %s, Status: %s, Reason: %s\n", c.Type, c.Status, c.Reason))
		}
	}
	return sb.String()
}

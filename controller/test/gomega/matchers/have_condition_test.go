package matchers_test

import (
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"

	"github.com/kgateway-dev/kgateway/v2/test/gomega/matchers"
)

var _ = Describe("HaveCondition", func() {
	var conditions []metav1.Condition

	BeforeEach(func() {
		conditions = []metav1.Condition{
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
	})

	Describe("matching conditions", func() {
		It("matches when condition type and status match", func() {
			Expect(conditions).To(matchers.HaveCondition("Ready", metav1.ConditionTrue))
		})

		It("matches when condition is False", func() {
			Expect(conditions).To(matchers.HaveCondition("Degraded", metav1.ConditionFalse))
		})

		It("does not match when condition type is not found", func() {
			Expect(conditions).ToNot(matchers.HaveCondition("NonExistent", metav1.ConditionTrue))
		})

		It("does not match when status differs", func() {
			Expect(conditions).ToNot(matchers.HaveCondition("Ready", metav1.ConditionFalse))
		})
	})

	Describe("empty conditions", func() {
		It("does not match when conditions slice is empty", func() {
			Expect([]metav1.Condition{}).ToNot(matchers.HaveCondition("Ready", metav1.ConditionTrue))
		})
	})

	Describe("failure messages", func() {
		It("provides informative failure message", func() {
			matcher := matchers.HaveCondition("NonExistent", metav1.ConditionTrue)
			success, _ := matcher.Match(conditions)
			Expect(success).To(BeFalse())
			Expect(matcher.FailureMessage(conditions)).To(ContainSubstring("NonExistent"))
			Expect(matcher.FailureMessage(conditions)).To(ContainSubstring("Ready"))
		})
	})
})

var _ = Describe("HaveAnyParentCondition", func() {
	var parentConditions [][]metav1.Condition

	BeforeEach(func() {
		parentConditions = [][]metav1.Condition{
			{
				{Type: "Accepted", Status: metav1.ConditionTrue, Reason: "Accepted"},
				{Type: "ResolvedRefs", Status: metav1.ConditionTrue, Reason: "ResolvedRefs"},
			},
			{
				{Type: "Accepted", Status: metav1.ConditionFalse, Reason: "Rejected"},
				{Type: "ResolvedRefs", Status: metav1.ConditionFalse, Reason: "RefNotFound"},
			},
		}
	})

	Describe("matching parent conditions", func() {
		It("matches when any parent has matching condition", func() {
			Expect(parentConditions).To(matchers.HaveAnyParentCondition("Accepted", metav1.ConditionTrue))
		})

		It("matches when second parent has matching condition", func() {
			Expect(parentConditions).To(matchers.HaveAnyParentCondition("Accepted", metav1.ConditionFalse))
		})

		It("does not match when no parent has matching condition", func() {
			Expect(parentConditions).ToNot(matchers.HaveAnyParentCondition("NonExistent", metav1.ConditionTrue))
		})

		It("does not match when status does not match any parent", func() {
			Expect(parentConditions).ToNot(matchers.HaveAnyParentCondition("ResolvedRefs", metav1.ConditionUnknown))
		})
	})

	Describe("empty parents", func() {
		It("does not match when parents slice is empty", func() {
			Expect([][]metav1.Condition{}).ToNot(matchers.HaveAnyParentCondition("Accepted", metav1.ConditionTrue))
		})
	})
})

var _ = Describe("HaveAnyAncestorCondition", func() {
	var ancestorConditions [][]metav1.Condition

	BeforeEach(func() {
		ancestorConditions = [][]metav1.Condition{
			{
				{Type: "Accepted", Status: metav1.ConditionTrue, Reason: "PolicyAccepted"},
			},
		}
	})

	Describe("matching ancestor conditions", func() {
		It("matches when any ancestor has matching condition", func() {
			Expect(ancestorConditions).To(matchers.HaveAnyAncestorCondition("Accepted", metav1.ConditionTrue))
		})

		It("does not match when no ancestor has matching condition", func() {
			Expect(ancestorConditions).ToNot(matchers.HaveAnyAncestorCondition("NonExistent", metav1.ConditionTrue))
		})
	})
})

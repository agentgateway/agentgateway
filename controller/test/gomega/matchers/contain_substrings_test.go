package matchers_test

import (
	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"

	"github.com/kgateway-dev/kgateway/v2/test/gomega/matchers"
)

var _ = Describe("ContainSubstrings", func() {

	DescribeTable("contains substrings",
		func(expectedSubstrings []string) {
			actualString := "this is the string"
			Expect(actualString).To(matchers.ContainSubstrings(expectedSubstrings))
		},
		Entry("empty list", []string{}),
		Entry("empty string", []string{""}),
		Entry("single substring", []string{"this"}),
		Entry("multiple substrings", []string{"the", "is", "this"}),
	)

	DescribeTable("does not contain substrings",
		func(expectedSubstrings []string) {
			actualString := "this is the string"
			Expect(actualString).NotTo(matchers.ContainSubstrings(expectedSubstrings))
		},
		Entry("missing substring", []string{"missing"}),
		Entry("substring and missing substring", []string{"this", "missing"}),
	)

})

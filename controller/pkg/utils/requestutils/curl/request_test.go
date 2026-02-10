package curl_test

import (
	"github.com/onsi/gomega/types"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/requestutils/curl"
)

var _ = Describe("Curl", func() {

	Context("BuildArgs", func() {

		DescribeTable("it builds the args using the provided option",
			func(option curl.Option, expectedMatcher types.GomegaMatcher) {
				Expect(curl.BuildArgs(option)).To(expectedMatcher)
			},
			Entry("VerboseOutput",
				curl.VerboseOutput(),
				ContainElement("-v"),
			),
			Entry("Silent",
				curl.Silent(),
				ContainElement("-s"),
			),
			Entry("WithBody",
				curl.WithBody("body"),
				ContainElement("--data-binary"),
			),
			Entry("WithRetries",
				curl.WithRetries(1, 1, 1),
				ContainElements("--retry", "--retry-delay", "--retry-max-time"),
			),
		)

	})

})

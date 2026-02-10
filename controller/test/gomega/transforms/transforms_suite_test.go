package transforms_test

import (
	"testing"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"
)

func TestTransforms(t *testing.T) {
	RegisterFailHandler(Fail)
	RunSpecs(t, "Transforms Suite")
}

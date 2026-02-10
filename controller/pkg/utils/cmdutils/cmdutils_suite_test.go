package cmdutils

import (
	"testing"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"
)

func TestCmdUtils(t *testing.T) {
	RegisterFailHandler(Fail)
	RunSpecs(t, "CmdUtils Suite")
}

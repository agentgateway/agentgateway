package flag

import (
	"os"

	"github.com/spf13/cobra"
)

var (
	kubeconfig = os.Getenv("KUBECONFIG")
)

func AttachGlobalFlags(c *cobra.Command) {
	c.PersistentFlags().StringVarP(&kubeconfig, "kubeconfig", "k", kubeconfig, "kubeconfig")
}

func Kubeconfig() string {
	return kubeconfig
}

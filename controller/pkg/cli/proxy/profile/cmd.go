package profile

import (
	"fmt"
	"time"

	"github.com/spf13/cobra"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

const defaultCPUProfileSeconds = 30

type profileFlags struct {
	namespace      string
	proxyAdminPort int
	outputFile     string
	local          bool
	seconds        int
	now            func() time.Time
}

func Command() flag.Command {
	common := &profileFlags{
		proxyAdminPort: wellknown.ProxyAdminPort,
		now:            time.Now,
	}

	return flag.Command{
		Use:   "profile",
		Short: "Collect Agentgateway proxy pprof profiles",
		Long:  "Collect CPU or heap pprof profiles from an Agentgateway proxy admin endpoint.",
		Children: []flag.CommandBuilder{
			func() flag.Command { return cpuCommand(common) },
			func() flag.Command { return heapCommand(common) },
		},
		AddPersistentFlags: func(cmd *cobra.Command) {
			common.attach(cmd)
		},
	}
}

func cpuCommand(common *profileFlags) flag.Command {
	return flag.Command{
		Use:     "cpu [resource]",
		Short:   "Collect a CPU pprof profile",
		Example: "  agctl proxy profile cpu gateway/my-gateway --seconds 30 -o ./profile.pb.gz\n  agctl proxy profile cpu --local -p 15000",
		Args:    cobra.MaximumNArgs(1),
		AddFlags: func(cmd *cobra.Command) {
			cmd.Flags().IntVar(&common.seconds, "seconds", defaultCPUProfileSeconds, "CPU profile duration in seconds")
		},
		RunE: func(cmd *cobra.Command, args []string) error {
			return run(cmd, common, args, profileKindCPU)
		},
	}
}

func heapCommand(common *profileFlags) flag.Command {
	return flag.Command{
		Use:     "heap [resource]",
		Short:   "Collect a heap pprof profile",
		Example: "  agctl proxy profile heap gateway/my-gateway -o ./heap.pb.gz\n  agctl proxy profile heap --local -p 15000",
		Args:    cobra.MaximumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			return run(cmd, common, args, profileKindHeap)
		},
	}
}

func (f *profileFlags) attach(cmd *cobra.Command) {
	cmd.PersistentFlags().StringVarP(&f.namespace, "namespace", "n", "", "Namespace to use when resolving resources")
	cmd.PersistentFlags().IntVarP(&f.proxyAdminPort, "port", "p", f.proxyAdminPort, "Agentgateway proxy admin port")
	cmd.PersistentFlags().StringVarP(&f.outputFile, "output", "o", "", "Output profile path")
	cmd.PersistentFlags().BoolVar(&f.local, "local", false, "Profile a local agentgateway instance on localhost")
}

func (f *profileFlags) validate(kind profileKind, args []string) error {
	if f.proxyAdminPort < 1 || f.proxyAdminPort > 65535 {
		return fmt.Errorf("invalid --port %d", f.proxyAdminPort)
	}
	if f.local && len(args) > 0 {
		return fmt.Errorf("--local does not accept a resource argument")
	}
	if kind == profileKindCPU && (f.seconds < 1 || f.seconds > 300) {
		return fmt.Errorf("invalid --seconds %d; must be between 1 and 300", f.seconds)
	}
	return nil
}

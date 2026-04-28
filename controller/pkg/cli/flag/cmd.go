package flag

import "github.com/spf13/cobra"

type CommandBuilder func() Command

type Command struct {
	Use                string
	Short              string
	Long               string
	Example            string
	Aliases            []string
	Args               cobra.PositionalArgs
	RunE               func(cmd *cobra.Command, args []string) error
	AddFlags           func(cmd *cobra.Command)
	AddPersistentFlags func(cmd *cobra.Command)
	Children           []CommandBuilder
}

func BuildCobra(cb CommandBuilder) *cobra.Command {
	built := cb()
	cmd := &cobra.Command{
		Use:          built.Use,
		Short:        built.Short,
		Long:         built.Long,
		Example:      built.Example,
		Aliases:      built.Aliases,
		Args:         built.Args,
		RunE:         built.RunE,
		SilenceUsage: true,
	}

	if built.AddFlags != nil {
		built.AddFlags(cmd)
	}
	if built.AddPersistentFlags != nil {
		built.AddPersistentFlags(cmd)
	}

	for _, child := range built.Children {
		cmd.AddCommand(BuildCobra(child))
	}

	return cmd
}

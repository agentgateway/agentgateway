package main

import (
	"fmt"
	"log"

	"github.com/spf13/cobra"

	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/setup"
	"github.com/agentgateway/agentgateway/controller/pkg/version"
)

func main() {
	var agentgatewayVersion bool
	cmd := &cobra.Command{
		Use:   "agentgateway",
		Short: "Runs the agentgateway controller",
		RunE: func(cmd *cobra.Command, args []string) error {
			if agentgatewayVersion {
				fmt.Println(version.String())
				return nil
			}
			s, err := setup.New()
			if err != nil {
				return fmt.Errorf("error setting up agentgateway: %w", err)
			}
			if err := s.Start(cmd.Context()); err != nil {
				return fmt.Errorf("err in main: %w", err)
			}

			return nil
		},
	}
	cmd.Flags().BoolVarP(&agentgatewayVersion, "version", "v", false, "Print the version of agentgateway")

	if err := cmd.Execute(); err != nil {
		log.Fatal(err)
	}
}

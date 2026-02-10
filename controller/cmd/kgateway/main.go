package main

import (
	"fmt"
	"log"

	"github.com/spf13/cobra"

	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/setup"
	"github.com/kgateway-dev/kgateway/v2/pkg/version"
)

func main() {
	var kgatewayVersion bool
	cmd := &cobra.Command{
		Use:   "kgateway",
		Short: "Runs the kgateway controller",
		RunE: func(cmd *cobra.Command, args []string) error {
			if kgatewayVersion {
				fmt.Println(version.String())
				return nil
			}
			s, err := setup.New()
			if err != nil {
				return fmt.Errorf("error setting up kgateway: %w", err)
			}
			if err := s.Start(cmd.Context()); err != nil {
				return fmt.Errorf("err in main: %w", err)
			}

			return nil
		},
	}
	cmd.Flags().BoolVarP(&kgatewayVersion, "version", "v", false, "Print the version of kgateway")

	if err := cmd.Execute(); err != nil {
		log.Fatal(err)
	}
}

package cmd

import (
	"fmt"

	"github.com/abetterinternet/prio-server/integration-tester/tester"
	"github.com/spf13/cobra"
)

var runCmd = &cobra.Command{
	Use:   "run",
	Short: "Run the integration-tester",
	RunE: func(cmd *cobra.Command, args []string) error {
		fmt.Println(namespace, name, manifestFileUrl, serviceAccountName, facilitatorImage, pushGateway, peerIdentity)
		t := tester.New(namespace, name, manifestFileUrl, serviceAccountName, facilitatorImage, pushGateway, peerIdentity)
		return t.Start()
	},
}

func init() {
	rootCmd.AddCommand(runCmd)
}

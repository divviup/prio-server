package cmd

import (
	"github.com/abetterinternet/prio-server/tester/tester"
	"github.com/spf13/cobra"
)

var runCmd = &cobra.Command{
	Use:   "run",
	Short: "Run the tester",
	RunE: func(cmd *cobra.Command, args []string) error {
		t := tester.New(namespace, name, manifestFileUrl, serviceAccountName, facilitatorImage, pushGateway, peerIdentity)
		return t.Start()
	},
}

func init() {
	rootCmd.AddCommand(runCmd)
}

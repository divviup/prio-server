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
		fmt.Println(namespace, name, manifestFileUrl, serviceAccountName, facilitatorImage, pushGateway, peerIdentity, awsAccountId)
		t := tester.New(kubeConfigPath,
			namespace, name,
			manifestFileUrl, serviceAccountName,
			facilitatorImage, pushGateway,
			peerIdentity, awsAccountId,
			dryRun)
		return t.Start()
	},
}

func init() {
	rootCmd.AddCommand(runCmd)
}

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
		fmt.Println(namespace, name, ownManifestUrl, phaManifestUrl, facilManifestUrl, serviceAccountName, facilitatorImage, pushGateway, awsAccountId)
		t := tester.New(kubeConfigPath,
			namespace, name,
			ownManifestUrl, phaManifestUrl, facilManifestUrl,
			serviceAccountName, facilitatorImage,
			pushGateway, awsAccountId,
			dryRun)
		return t.Start()
	},
}

func init() {
	rootCmd.AddCommand(runCmd)
}

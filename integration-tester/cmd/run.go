package cmd

import (
	"github.com/abetterinternet/prio-server/integration-tester/tester"
	"github.com/spf13/cobra"
)

var runCmd = &cobra.Command{
	Use:   "run",
	Short: "Run the integration-tester",
	RunE: func(cmd *cobra.Command, args []string) error {
		t := tester.New(kubeConfigPath,
			namespace, ingestorLabel,
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

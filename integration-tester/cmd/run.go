package cmd

import (
	"github.com/abetterinternet/prio-server/integration-tester/tester"
	"github.com/spf13/cobra"
)

var runCmd = &cobra.Command{
	Use:   "run",
	Short: "Run the integration-tester",
	RunE: func(cmd *cobra.Command, args []string) error {
		t, err := tester.New(kubeConfigPath,
			namespace, ingestorLabel,
			ownManifestUrl, phaManifestUrl, facilManifestUrl,
			serviceAccountName, facilitatorImage,
			pushGateway, awsAccountId,
			dryRun)

		if err != nil {
			return err
		}

		return t.Start()
	},
}

func init() {
	rootCmd.AddCommand(runCmd)
}

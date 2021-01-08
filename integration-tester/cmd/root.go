package cmd

import (
	"fmt"
	"os"
	"strings"

	"github.com/spf13/cobra"
	"github.com/spf13/pflag"
	"github.com/spf13/viper"
)

var (
	namespace          = ""
	name               = ""
	manifestFileUrl    = ""
	serviceAccountName = ""

	facilitatorImage = ""
	pushGateway      = ""
	peerIdentity     = ""
	awsAccountId     = ""
)

var rootCmd = &cobra.Command{
	Use: "integration-tester",
	PersistentPreRun: func(cmd *cobra.Command, args []string) {
		v := viper.New()

		v.SetConfigName("config")
		v.AddConfigPath(".")
		_ = v.ReadInConfig()

		v.SetEnvPrefix("TESTER")
		v.AutomaticEnv()

		handleDashes(cmd, v)
	},
}

func handleDashes(cmd *cobra.Command, v *viper.Viper) {
	cmd.Flags().VisitAll(func(f *pflag.Flag) {
		if strings.Contains(f.Name, "-") {
			envVarSuffix := strings.ToUpper(strings.ReplaceAll(f.Name, "-", "_"))
			_ = v.BindEnv(f.Name, fmt.Sprintf("TESTER_%s", envVarSuffix))
		}

		if !f.Changed && v.IsSet(f.Name) {
			val := v.Get(f.Name)
			_ = cmd.Flags().Set(f.Name, fmt.Sprintf("%v", val))
		}
	})
}

func init() {
	rootCmd.PersistentFlags().StringVarP(&name, "name", "", "", "The name of this ingestor")
	rootCmd.PersistentFlags().StringVarP(&namespace, "namespace", "", "", "The namespace this will run in")
	rootCmd.PersistentFlags().StringVarP(&manifestFileUrl, "manifest-file-url", "", "", "The URL where the manifest can be accessed")
	rootCmd.PersistentFlags().StringVarP(&serviceAccountName, "service-account-name", "", "", "The service account name for kubernetes")

	rootCmd.PersistentFlags().StringVarP(&facilitatorImage, "facilitator-image", "", "", "The image for the facilitator")
	rootCmd.PersistentFlags().StringVarP(&pushGateway, "push-gateway", "", "", "The push gateway")
	rootCmd.PersistentFlags().StringVarP(&peerIdentity, "peer-identity", "", "", "The peer identity")
	rootCmd.PersistentFlags().StringVarP(&awsAccountId, "aws-account-id", "", "", "The aws account ID")
}

func Execute() {
	// Execute run command as default
	cmd, _, err := rootCmd.Find(os.Args[1:])
	if (len(os.Args) <= 1 || os.Args[1] != "help") && (err != nil || cmd == rootCmd) {
		args := append([]string{"run"}, os.Args[1:]...)
		rootCmd.SetArgs(args)
	}

	if err := rootCmd.Execute(); err != nil {
		panic(err)
	}
}

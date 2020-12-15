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
	namespace          string
	name               string
	manifestFileUrl    string
	serviceAccountName string

	facilitatorImage string
	pushGateway      string
	peerIdentity     string
)

var rootCmd = &cobra.Command{
	Use: "tester",
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
	rootCmd.PersistentFlags().StringVarP(&name, "name", "n", "", "The name of this ingestor")
	rootCmd.PersistentFlags().StringVarP(&namespace, "namespace", "ns", "", "The namespace this will run in")
	rootCmd.PersistentFlags().StringVarP(&manifestFileUrl, "manifest-file-url", "mu", "", "The URL where the manifest can be accessed")
	rootCmd.PersistentFlags().StringVarP(&serviceAccountName, "service-account-name", "sau", "", "The service account name for kubernetes")

	rootCmd.PersistentFlags().StringVarP(&facilitatorImage, "facilitator-image", "fi", "", "The image for the facilitator")
	rootCmd.PersistentFlags().StringVarP(&pushGateway, "push-gateway", "pg", "", "The push gateway")
	rootCmd.PersistentFlags().StringVarP(&peerIdentity, "peer-identity", "pi", "", "The peer identity")
}

func Execute() {
	// Allow running from explorer
	cobra.MousetrapHelpText = ""

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

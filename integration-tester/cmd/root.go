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
	kubeConfigPath     = ""
	namespace          = ""
	name               = ""
	manifestFileUrl    = ""
	serviceAccountName = ""

	facilitatorImage = ""
	pushGateway      = ""
	peerIdentity     = ""
	awsAccountId     = ""
)

var dryRun bool

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
	rootCmd.PersistentFlags().StringVar(&kubeConfigPath, "kube-config-path", "", "Path to the kubeconfig file to be used to authenticate to Kubernetes API")

	rootCmd.PersistentFlags().StringVar(&name, "name", "", "The name of this ingestor")
	rootCmd.PersistentFlags().StringVar(&namespace, "namespace", "", "The namespace this will run in")
	rootCmd.PersistentFlags().StringVar(&manifestFileUrl, "manifest-file-url", "", "The URL where the manifest can be accessed")
	rootCmd.PersistentFlags().StringVar(&serviceAccountName, "service-account-name", "", "The service account name for kubernetes")

	rootCmd.PersistentFlags().StringVar(&facilitatorImage, "facilitator-image", "", "The image for the facilitator")
	rootCmd.PersistentFlags().StringVar(&pushGateway, "push-gateway", "", "The push gateway")
	rootCmd.PersistentFlags().StringVar(&peerIdentity, "peer-identity", "", "The peer identity")
	rootCmd.PersistentFlags().StringVar(&awsAccountId, "aws-account-id", "", "The aws account ID")

	rootCmd.PersistentFlags().BoolVar(&dryRun, "dry-run", false, "If set, no operations with side effects will be done")
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

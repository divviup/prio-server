package cmd

import (
	"fmt"
	"log"
	"os"
	"strings"

	"github.com/spf13/cobra"
	"github.com/spf13/pflag"
	"github.com/spf13/viper"
)

var (
	kubeConfigPath = ""
	namespace      = ""
	name           = ""

	ownManifestUrl   = ""
	phaManifestUrl   = ""
	facilManifestUrl = ""

	serviceAccountName = ""

	facilitatorImage = ""
	pushGateway      = ""
	awsAccountId     = ""

	dryRun = true
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
	log.Println("flag values:")
	cmd.Flags().VisitAll(func(f *pflag.Flag) {
		log.Printf("\t--%s=%s\n", f.Name, f.Value)
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

	rootCmd.PersistentFlags().StringVar(&ownManifestUrl, "own-manifest-url", "", "The URL where the manifest for the ingestor (integration-tester) can be accessed")
	rootCmd.PersistentFlags().StringVar(&phaManifestUrl, "pha-manifest-url", "", "The URL where the manifest for the public health authority (pha) can be accessed")
	rootCmd.PersistentFlags().StringVar(&facilManifestUrl, "facil-manifest-url", "", "The URL where the manifest for facilitator can be accessed")

	rootCmd.PersistentFlags().StringVar(&serviceAccountName, "service-account-name", "", "The service account name for kubernetes")

	rootCmd.PersistentFlags().StringVar(&facilitatorImage, "facilitator-image", "", "The image for the facilitator")
	rootCmd.PersistentFlags().StringVar(&pushGateway, "push-gateway", "", "The push gateway")
	rootCmd.PersistentFlags().StringVar(&awsAccountId, "aws-account-id", "", "The aws account ID")

	rootCmd.PersistentFlags().BoolVar(&dryRun, "dry-run", true, "If set, no operations with side effects will be done")
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

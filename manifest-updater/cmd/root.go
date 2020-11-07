package cmd

import (
	log "github.com/sirupsen/logrus"
	"github.com/spf13/cobra"
	"github.com/spf13/viper"
	"os"
)

var rootCmd = &cobra.Command{
	Use: "bot",
	PersistentPreRun: func(cmd *cobra.Command, args []string) {
		viper.SetConfigName("config")
		viper.AddConfigPath(".")
		viper.SetEnvPrefix("KR")
		viper.AutomaticEnv()

		_ = viper.ReadInConfig()

		level, err := log.ParseLevel(viper.GetString("log"))

		if err != nil {
			panic(err)
		}

		log.SetFormatter(&log.TextFormatter{
			ForceColors: viper.GetBool("colors"),
		})
		log.SetOutput(os.Stdout)
		log.SetLevel(level)
	},
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

func init() {
	rootCmd.PersistentFlags().String("log", "info", "The log level to output")
	rootCmd.PersistentFlags().Bool("colors", false, "Force output with colors")

	rootCmd.PersistentFlags().String("environment_name", "", "Name of the environment")
	rootCmd.PersistentFlags().String("locality", "", "Name of locality")
	rootCmd.PersistentFlags().String("manifest_bucket_location", "", "Manifest bucket location")
	rootCmd.PersistentFlags().StringSlice("data_share_processors", make([]string, 0), "List of datashare processors")

	//if err := rootCmd.MarkPersistentFlagRequired("environment_name"); err != nil {
	//	log.Fatalf("environment_name missing")
	//}
	//if err := rootCmd.MarkPersistentFlagRequired("locality"); err != nil {
	//	log.Fatalf("locality missing")
	//}
	//if err := rootCmd.MarkPersistentFlagRequired("manifest_bucket_location"); err != nil {
	//	log.Fatalf("manifest_bucket_location missing")
	//}
	//if err := rootCmd.MarkPersistentFlagRequired("data_share_processors"); err != nil {
	//	log.Fatalf("data_share_processors missing")
	//}

	_ = viper.BindPFlag("log", rootCmd.PersistentFlags().Lookup("log"))
	_ = viper.BindPFlag("colors", rootCmd.PersistentFlags().Lookup("colors"))
}

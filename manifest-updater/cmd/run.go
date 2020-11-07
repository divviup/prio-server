package cmd

import (
	"crypto/rand"
	"crypto/x509"
	"github.com/abetterinternet/prio-server/manifest-updater/manifest"
	"github.com/abetterinternet/prio-server/manifest-updater/secrets"
	log "github.com/sirupsen/logrus"
	"github.com/spf13/cobra"
	"github.com/spf13/viper"
	"os"
	"os/signal"
	"syscall"
)

func init() {
	rootCmd.AddCommand(runCmd)
}

var runCmd = &cobra.Command{
	Use:   "run",
	Short: "Run the manifest updater",
	RunE: func(cmd *cobra.Command, args []string) error {
		environmentName := viper.GetString("environment_name")
		locality := viper.GetString("locality")
		manifestBucket := viper.GetString("manifest_bucket_location")
		dsps := viper.GetStringSlice("data_share_processors")

		log.WithFields(
			map[string]interface{}{
				"environment name":      environmentName,
				"locality":              locality,
				"manifest bucket":       manifestBucket,
				"data share processors": dsps,
			},
		).Info("Starting the updater...")

		kube, _ := secrets.NewKube(environmentName, dsps)

		key, err := kube.ReconcilePacketEncryptionKey()
		if err != nil {
			log.Fatal(err)
		}
		cert, err := key.CreatePemEncodedCertificateRequest(rand.Reader, new(x509.CertificateRequest))
		if err != nil {
			log.Fatal(err)
		}

		packet := &manifest.PacketEncryptionCertificate{Certificate: cert}

		updater, _ := manifest.NewUpdater(environmentName, locality, manifestBucket, dsps)
		err = updater.UpdateDataShareSpecificManifest(&manifest.BatchSigningPublicKey{
			PublicKey:  "TEST",
			Expiration: "TEST",
		}, packet)

		if err != nil {
			log.Fatal(err)
		}

		terminate := make(chan os.Signal, 1)
		signal.Notify(terminate, syscall.SIGINT, syscall.SIGTERM)

		toTerminate := <-terminate
		for {
			if toTerminate != nil {
				// TODO what should we do when this happens?
				break
			}
		}

		return nil
	},
}

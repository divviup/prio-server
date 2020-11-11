package cmd

import (
	"crypto/rand"
	"crypto/x509"
	"fmt"
	"github.com/abetterinternet/prio-server/manifest-updater/manifest"
	"github.com/abetterinternet/prio-server/manifest-updater/secrets"
	log "github.com/sirupsen/logrus"
	"github.com/spf13/cobra"
	"github.com/spf13/viper"
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

		var packetEncryptionCertificate manifest.PacketEncryptionCertificates
		var dspSigningKeys map[string]manifest.BatchSigningPublicKeys

		kube, _ := secrets.NewKube(locality, dsps)

		packetEncryptionKeys, err := kube.ReconcilePacketEncryptionKey()
		if err != nil {
			log.Fatal(err)
		}
		if packetEncryptionKeys != nil {
			packetEncryptionCertificate = make(manifest.PacketEncryptionCertificates)

			for _, key := range packetEncryptionKeys {
				cert, err := key.CreatePemEncodedCertificateRequest(rand.Reader, new(x509.CertificateRequest))
				if err != nil {
					log.Fatal(err)
				}
				packetEncryptionCertificate[*key.KubeIdentifier] = manifest.PacketEncryptionCertificate{Certificate: cert}
			}
		}

		batchSigningKeys, err := kube.ReconcileBatchSigningKey()
		if err != nil {
			log.Fatal(err)
		}
		if batchSigningKeys != nil {
			dspSigningKeys, err = prioKeysToBatchSigningManifests(batchSigningKeys)
			if err != nil {
				log.Fatal(err)
			}
		}

		updater, _ := manifest.NewUpdater(environmentName, locality, manifestBucket, dsps)
		err = updater.UpdateDataShareSpecificManifest(dspSigningKeys, packetEncryptionCertificate)

		if err != nil {
			log.Fatal(err)
		}
		return nil
	},
}

func prioKeysToBatchSigningManifests(keys map[string][]*secrets.PrioKey) (map[string]manifest.BatchSigningPublicKeys, error) {
	results := make(map[string]manifest.BatchSigningPublicKeys)
	for dataShareProcessor, prioKeys := range keys {
		publicKeys := make(manifest.BatchSigningPublicKeys)
		results[dataShareProcessor] = publicKeys

		for _, key := range prioKeys {
			publicKey, err := key.GetPemEncodedPublicKey()
			if err != nil {
				return nil, err
			}
			if key.KubeIdentifier == nil {
				return nil, fmt.Errorf("kubeidentifier was nil")
			}

			if key.Expiration == nil {
				return nil, fmt.Errorf("expiration was nil")
			}

			publicKeys[*key.KubeIdentifier] = manifest.BatchSigningPublicKey{
				PublicKey:  publicKey,
				Expiration: *key.Expiration,
			}
		}
	}
	return results, nil
}

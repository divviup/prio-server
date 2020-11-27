package cmd

import (
	"crypto/rand"
	"crypto/x509"
	"fmt"
	"time"

	manifestConfig "github.com/abetterinternet/prio-server/manifest-updater/config"
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
		ingestors := viper.GetStringSlice("ingestors")

		batchSigningExpiration := viper.GetInt32("batch_signing_key_expiration")
		batchSigningRotation := viper.GetInt32("batch_signing_key_rotation")

		packetEncryptionExpiration := viper.GetInt32("packet_encryption_key_expiration")
		packetEncryptionRotation := viper.GetInt32("packet_encryption_key_rotation")

		configLocation := viper.GetString("config_location")

		log.WithFields(
			map[string]interface{}{
				"environment name":                  environmentName,
				"locality":                          locality,
				"manifest bucket":                   manifestBucket,
				"ingestors":                         ingestors,
				"batch signing expiration days":     batchSigningExpiration,
				"batch signing rotation days":       batchSigningRotation,
				"packet encryption expiration days": packetEncryptionExpiration,
				"packet encryption rotation days":   packetEncryptionRotation,
				"config location":                   configLocation,
			},
		).Info("Starting the updater...")

		config, err := manifestConfig.New(configLocation)

		var packetEncryptionCertificate manifest.PacketEncryptionKeyCSRs
		var ingestorSigningKeys map[string]manifest.BatchSigningPublicKeys

		kube, _ := secrets.NewKube(locality, ingestors,
			secrets.NewKeySpec(time.Duration(batchSigningExpiration), time.Duration(batchSigningRotation)),
			secrets.NewKeySpec(time.Duration(packetEncryptionExpiration), time.Duration(packetEncryptionRotation)),
		)

		packetEncryptionKeys, err := kube.ReconcilePacketEncryptionKey()
		if err != nil {
			log.Fatal(err)
		}
		if packetEncryptionKeys != nil {
			packetEncryptionCertificate = make(manifest.PacketEncryptionKeyCSRs)

			for _, key := range packetEncryptionKeys {
				csr, err := key.CreatePemEncodedCertificateRequest(rand.Reader, new(x509.CertificateRequest))
				if err != nil {
					log.Fatal(err)
				}
				packetEncryptionCertificate[*key.KubeIdentifier] = manifest.PacketEncryptionCertificate{CertificateSigningRequest: csr}
			}
		}

		batchSigningKeys, err := kube.ReconcileBatchSigningKey()
		if err != nil {
			log.Fatal(err)
		}
		if batchSigningKeys != nil {
			ingestorSigningKeys, err = prioKeysToBatchSigningManifests(batchSigningKeys)
			if err != nil {
				log.Fatal(err)
			}
		}

		updater, _ := manifest.NewUpdater(config, environmentName, locality, manifestBucket, ingestors)
		err = updater.UpdateDataShareSpecificManifest(ingestorSigningKeys, packetEncryptionCertificate)

		if err != nil {
			log.Fatal(err)
		}
		return nil
	},
}

func prioKeysToBatchSigningManifests(keys map[string][]*secrets.PrioKey) (map[string]manifest.BatchSigningPublicKeys, error) {
	results := make(map[string]manifest.BatchSigningPublicKeys)
	for ingestor, prioKeys := range keys {
		publicKeys := make(manifest.BatchSigningPublicKeys)
		results[ingestor] = publicKeys

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

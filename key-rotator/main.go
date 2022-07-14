package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"runtime"
	"runtime/pprof"
	"strings"
	"sync"
	"time"

	secretmanager "cloud.google.com/go/secretmanager/apiv1"
	"github.com/aws/aws-sdk-go/aws/session"
	"github.com/aws/aws-sdk-go/service/secretsmanager"
	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/client_golang/prometheus/promauto"
	"github.com/prometheus/client_golang/prometheus/push"
	"github.com/rs/zerolog"
	"github.com/rs/zerolog/log"
	"golang.org/x/sync/errgroup"
	"k8s.io/client-go/kubernetes"
	"k8s.io/client-go/rest"
	"k8s.io/client-go/tools/clientcmd"

	"github.com/abetterinternet/prio-server/key-rotator/key"
	"github.com/abetterinternet/prio-server/key-rotator/manifest"
	"github.com/abetterinternet/prio-server/key-rotator/storage"

	_ "k8s.io/client-go/plugin/pkg/client/auth" // included for k8s client auth plugins
)

var (
	// Required configuration.
	prioEnv           = flag.String("prio-environment", "", "Required. The prio `environment`, e.g. 'prod-us' or 'prod-intl'")
	namespace         = flag.String("kubernetes-namespace", "", "Required. The Kubernetes `namespace`, e.g. 'us-ca' or 'ta-ta'")
	manifestBucketURL = flag.String("manifest-bucket-url", "", "Required. The URL of the manifest `bucket`, e.g. 's3://bucket-name' or 'gs://bucket-name'")
	locality          = flag.String("locality", "", "Required. The Prio `locality`, e.g. 'us-ca' or 'ta-ta'")
	ingestors         = flag.String("ingestors", "", "Required. Comma-separated list of `ingestors`, e.g. 'apple' or 'g-enpa'")
	csrFQDN           = flag.String("csr-fqdn", "", "Required. FQDN to use as common name in generated CSRs")

	// Rotation configuration.
	batchSigningKeyEnableRotation = flag.Bool("batch-signing-key-enable-rotation", true, "Determines if batch signing keys are rotated. If no key versions exist, a new one will be created irrespective of this flag's value")
	batchSigningKeyCreateMinAge   = flag.Duration("batch-signing-key-create-min-age", 9*30*24*time.Hour, "How frequently to create a new batch signing key version")               // default: 9 months
	batchSigningKeyPrimaryMinAge  = flag.Duration("batch-signing-key-primary-min-age", 7*24*time.Hour, "How old a batch signing key version must be before it can become primary") // default: 1 week
	batchSigningKeyDeleteMinAge   = flag.Duration("batch-signing-key-delete-min-age", 13*30*24*time.Hour, "How old a batch signing key version must be before it can be deleted")  // default: 13 months
	batchSigningKeyDeleteMinCount = flag.Int("batch-signing-key-delete-min-count", 2, "The minimum number of batch signing key versions left undeleted after rotation")
	batchSigningKeyAlwaysWrite    = flag.Bool("batch-signing-key-always-write", false, "If set, always write batch signing key to backing storage, even if no changes are detected")

	packetEncryptionKeyEnableRotation = flag.Bool("packet-encryption-key-enable-rotation", true, "Determines if packet encryption keys are rotated. If no key versions exist, a new one will be created irrespective of this flag's value")
	packetEncryptionKeyCreateMinAge   = flag.Duration("packet-encryption-key-create-min-age", 9*30*24*time.Hour, "How frequently to create a new packet encryption key version")              // default: 9 months
	packetEncryptionKeyPrimaryMinAge  = flag.Duration("packet-encryption-key-primary-min-age", 0, "How old a packet encryption key version must be before it can become primary")             // default: 0
	packetEncryptionKeyDeleteMinAge   = flag.Duration("packet-encryption-key-delete-min-age", 13*30*24*time.Hour, "How old a packet encryption key version must be before it can be deleted") // default: 13 months
	packetEncryptionKeyDeleteMinCount = flag.Int("packet-encryption-key-delete-min-count", 2, "The minimum number of packet encryption key versions left undeleted after rotation")
	packetEncryptionKeyAlwaysWrite    = flag.Bool("packet-encryption-key-always-write", false, "If set, always write packet encryption key to backing storage, even if no changes are detected")

	skipManifestPreUpdateValidations  = flag.Bool("unsafe-skip-manifest-pre-update-validations", false, "If set, skip manifest pre-update validations. This flag is unsafe; do not set unless you know what you are doing")
	skipManifestPostUpdateValidations = flag.Bool("unsafe-skip-manifest-post-update-validations", false, "If set, skip manifest post-update validations. This flag is unsafe; do not set unless you know what you are doing")

	// Other flags.
	backup                        = flag.String("backup", "", "Set to 'aws' or 'gcp:gcp-project-id' to back up secrets to the respective cloud's secrets manager")
	dryRun                        = flag.Bool("dry-run", true, "If set, do not actually write any keys or manifests back (only report what would have changed)")
	timeout                       = flag.Duration("timeout", 10*time.Minute, "The `deadline` before key-rotator terminates. Set to 0 to disable timeout")
	defaultManifestByIngestorJSON = flag.String("default-manifest-by-ingestor", "", "If set to a JSON map from ingestor to manifest, the specified manifest will be used as a template if there is no pre-existing manifest (i.e. for newly-provisioned localities)")
	awsRegion                     = flag.String("aws-region", "", "If specified, the AWS `region` to use for manifest storage")
	pushGateway                   = flag.String("push-gateway", "", "Set this to the gateway to use with prometheus. If left empty, metrics will not be pushed to prometheus.")
	kubeconfig                    = flag.String("kubeconfig", "", "The `path` to user's kubeconfig file; if unspecified, assumed to be running in-cluster") // typical value is $HOME/.kube/config
	cpuProfile                    = flag.String("cpuprofile", "", "Write a CPU profile to `file`")
	memProfile                    = flag.String("memprofile", "", "Write a memory profile to `file`")

	// Metrics.
	pusher      *push.Pusher // populated only if --push-gateway is specified.
	keysWritten = promauto.NewGauge(prometheus.GaugeOpts{
		Name: "key_rotator_keys_written",
		Help: "Number of keys written by the key rotator.",
	})
	manifestsWritten = promauto.NewGauge(prometheus.GaugeOpts{
		Name: "key_rotator_manifests_written",
		Help: "Number of manifests written by the key rotator.",
	})
	lastSuccess = promauto.NewGauge(prometheus.GaugeOpts{
		Name: "key_rotator_last_success",
		Help: "Time of last successful run, as a UNIX seconds timestamp.",
	})
	lastFailure = promauto.NewGauge(prometheus.GaugeOpts{
		Name: "key_rotator_last_failure",
		Help: "Time of last failed run, as a UNIX seconds timestamp.",
	})
)

func main() {
	// Parse & validate flags.
	flag.Parse()

	if *pushGateway != "" {
		pusher = push.New(*pushGateway, "key-rotator").
			Gatherer(prometheus.DefaultGatherer).
			Grouping("locality", *locality)
	}

	if *kubeconfig != "" {
		// If we are running on someone's workstation, get nice pretty-printed
		// log lines instead of structured JSON.
		log.Logger = log.Output(zerolog.ConsoleWriter{Out: os.Stderr})
	}

	if *cpuProfile != "" {
		f, err := os.Create(*cpuProfile)
		if err != nil {
			fail("Could not create CPU profile: %v", err)
		}
		defer func() {
			if err := f.Close(); err != nil {
				log.Err(err).Msg("Could not close CPU profile")
			}
		}()
		if err := pprof.StartCPUProfile(f); err != nil {
			fail("Could not start CPU file: %v", err)
		}
		defer pprof.StopCPUProfile()
	}

	switch {
	case *prioEnv == "":
		fail("--prio-environment is required")
	case *namespace == "":
		fail("--kubernetes-namespace is required")
	case *manifestBucketURL == "":
		fail("--manifest-bucket-url is required")
	case *locality == "":
		fail("--locality is required")
	case *csrFQDN == "":
		fail("--csr-fqdn is required")
	case *batchSigningKeyCreateMinAge < 0:
		fail("--batch-signing-key-create-min-age must be non-negative")
	case *batchSigningKeyPrimaryMinAge < 0:
		fail("--batch-signing-key-primary-min-age must be non-negative")
	case *batchSigningKeyDeleteMinAge < 0:
		fail("--batch-signing-key-delete-min-age must be non-negative")
	case *batchSigningKeyDeleteMinCount < 0:
		fail("--batch-signing-key-delete-min-count must be non-negative")
	case *packetEncryptionKeyCreateMinAge < 0:
		fail("--packet-encryption-key-create-min-age must be non-negative")
	case *packetEncryptionKeyPrimaryMinAge < 0:
		fail("--packet-encryption-key-primary-min-age must be non-negative")
	case *packetEncryptionKeyDeleteMinAge < 0:
		fail("--packet-encryption-key-delete-min-age must be non-negative")
	case *packetEncryptionKeyDeleteMinCount < 0:
		fail("--packet-encryption-key-delete-min-count must be non-negative")
	case *backup != "" && *backup != "aws" && !strings.HasPrefix(*backup, "gcp:"):
		fail("--backup must be one of 'aws' or 'gcp:gcp-project-id' if specified")
	case *timeout < 0:
		fail("--timeout must be non-negative")
	}

	ingestorLst := strings.Split(*ingestors, ",")
	for i, v := range ingestorLst {
		v = strings.TrimSpace(v)
		if v == "" {
			fail("--ingestors must be comma-separated list of ingestor names")
		}
		ingestorLst[i] = v
	}

	var defaultManifestByDSP map[string]manifest.DataShareProcessorSpecificManifest
	if *defaultManifestByIngestorJSON != "" {
		var defaultManifestByIngestor map[string]manifest.DataShareProcessorSpecificManifest
		if err := json.Unmarshal([]byte(*defaultManifestByIngestorJSON), &defaultManifestByIngestor); err != nil {
			fail("--default-manifest cannot be deserialized: %v", err)
		}
		defaultManifestByDSP = map[string]manifest.DataShareProcessorSpecificManifest{}
		for ingestor, manifest := range defaultManifestByIngestor {
			defaultManifestByDSP[dspName(*locality, ingestor)] = manifest
		}
	}

	log.Info().Msgf("Starting up")
	if *skipManifestPreUpdateValidations {
		log.Warn().Msgf("--unsafe-skip-manifest-pre-update-validations is set; this flag is inherently unsafe and should only be set temporarily in order to fix an ongoing incident")
	}
	if *skipManifestPostUpdateValidations {
		log.Warn().Msgf("--unsafe-skip-manifest-post-update-validations is set; this flag is inherently unsafe and should only be set temporarily in order to fix an ongoing incident")
	}
	ctx := context.Background()
	if *timeout > 0 {
		var cancel context.CancelFunc
		ctx, cancel = context.WithTimeout(ctx, *timeout)
		defer cancel()
	}

	// Get Kubernetes client & create key store from it.
	log.Info().Msgf("Creating key store")

	var cfg *rest.Config
	switch {
	case *kubeconfig == "": // in-cluster config, https://github.com/kubernetes/client-go/blob/master/examples/in-cluster-client-configuration/main.go
		c, err := rest.InClusterConfig()
		if err != nil {
			fail("Couldn't get in-cluster Kubernetes config (if running out-of-cluster specify --kubeconfig): %v", err)
		}
		cfg = c
		log.Info().Msgf("Using in-cluster Kubernetes config")

	default: // out-of-cluster config, https://github.com/kubernetes/client-go/blob/master/examples/out-of-cluster-client-configuration/main.go
		c, err := clientcmd.BuildConfigFromFlags("", *kubeconfig)
		if err != nil {
			fail("Couldn't get out-of-cluster Kubernetes config: %v", err)
		}
		cfg = c
		log.Info().Msgf("Using out-of-cluster Kubernetes config")
	}

	k8s, err := kubernetes.NewForConfig(cfg)
	if err != nil {
		fail("Couldn't create Kubernetes client: %v", err)
	}
	keyStore := storage.NewKubernetesKey(k8s.CoreV1().Secrets(*namespace), *prioEnv)

	// Create backup key store if configured to do so.
	switch {
	case *backup == "aws":
		sess, err := session.NewSession()
		if err != nil {
			fail("Couldn't create AWS session: %v", err)
		}
		keyStore = storage.NewBackupKey(keyStore, storage.NewAWSKey(secretsmanager.New(sess), *prioEnv))

	case strings.HasPrefix(*backup, "gcp:"):
		gcpProjectID := strings.TrimPrefix(*backup, "gcp:")
		sm, err := secretmanager.NewClient(ctx)
		if err != nil {
			fail("Couldn't create GCP secret manager client: %v", err)
		}
		keyStore = storage.NewBackupKey(keyStore, storage.NewGCPKey(sm, *prioEnv, gcpProjectID))
	}

	// Get Manifest storage client.
	log.Info().Msgf("Creating manifest store")
	var opts []storage.ManifestOption
	if *awsRegion != "" {
		opts = append(opts, storage.WithAWSRegion(*awsRegion))
	}
	if defaultManifestByDSP != nil {
		opts = append(opts, storage.WithDefaultDataShareProcessorManifests(defaultManifestByDSP))
	}
	manifestStore, err := storage.NewManifest(ctx, *manifestBucketURL, opts...)
	if err != nil {
		fail("Couldn't create manifest store: %v", err)
	}

	// ...and go!
	if *dryRun {
		log.Info().Msgf("--dry-run is specified: no writes will actually occur")
		keyStore = dryRunKeyStore{keyStore}
		manifestStore = dryRunManifestStore{manifestStore}
	}
	if err := rotateKeys(ctx, rotateKeysConfig{
		keyStore:        keyStore,
		manifestStore:   manifestStore,
		now:             time.Now(),
		locality:        *locality,
		ingestors:       ingestorLst,
		prioEnvironment: *prioEnv,
		csrFQDN:         *csrFQDN,
		batchCFG: rotateKeyConfig{
			enableRotation: *batchSigningKeyEnableRotation,
			alwaysWrite:    *batchSigningKeyAlwaysWrite,
			rotationCFG: key.RotationConfig{
				CreateKeyFunc:     key.P256.New,
				CreateMinAge:      *batchSigningKeyCreateMinAge,
				PrimaryMinAge:     *batchSigningKeyPrimaryMinAge,
				DeleteMinAge:      *batchSigningKeyDeleteMinAge,
				DeleteMinKeyCount: *batchSigningKeyDeleteMinCount,
			},
		},
		packetCFG: rotateKeyConfig{
			enableRotation: *packetEncryptionKeyEnableRotation,
			alwaysWrite:    *packetEncryptionKeyAlwaysWrite,
			rotationCFG: key.RotationConfig{
				CreateKeyFunc:     key.P256.New,
				CreateMinAge:      *packetEncryptionKeyCreateMinAge,
				PrimaryMinAge:     *packetEncryptionKeyPrimaryMinAge,
				DeleteMinAge:      *packetEncryptionKeyDeleteMinAge,
				DeleteMinKeyCount: *packetEncryptionKeyDeleteMinCount,
			},
		},
		skipManifestPreUpdateValidations:  *skipManifestPreUpdateValidations,
		skipManifestPostUpdateValidations: *skipManifestPostUpdateValidations,
	}); err != nil {
		fail("Couldn't rotate keys: %v", err)
	}

	lastSuccess.SetToCurrentTime()
	if err := tryPushMetrics(); err != nil {
		log.Error().Err(err).Msgf("Couldn't push metrics: %v", err)
	}

	if *memProfile != "" {
		f, err := os.Create(*memProfile)
		if err != nil {
			fail("Could not create memory profile: %v", err)
		}
		runtime.GC()
		if err := pprof.WriteHeapProfile(f); err != nil {
			fail("Could not write memory profile: %v", err)
		}
		if err := f.Close(); err != nil {
			log.Err(err).Msg("Could not close memory profile")
		}
	}

	log.Info().Msgf("Keys rotated successfully")
}

type rotateKeysConfig struct {
	// Dependencies.
	keyStore      storage.Key
	manifestStore storage.Manifest

	// Configuration.
	now                               time.Time
	locality                          string
	ingestors                         []string
	prioEnvironment                   string
	csrFQDN                           string
	batchCFG                          rotateKeyConfig
	packetCFG                         rotateKeyConfig
	skipManifestPreUpdateValidations  bool
	skipManifestPostUpdateValidations bool
}

type rotateKeyConfig struct {
	enableRotation bool // determines if rotation occurs at all
	alwaysWrite    bool // determines if keys are written back to storage, even if they have not changed
	rotationCFG    key.RotationConfig
}

func rotateKeys(ctx context.Context, cfg rotateKeysConfig) error {
	// Retrieve keys & manifests.
	log.Info().Msgf("Reading keys & manifests")
	oldPacketEncryptionKey, oldBatchSigningKeyByIngestor, oldManifestByIngestor, err :=
		readKeysAndManifests(ctx, cfg.keyStore, cfg.manifestStore, cfg.locality, cfg.ingestors)
	if err != nil {
		return fmt.Errorf("couldn't get keys & manifests: %w", err)
	}

	// Rotate keys.
	log.Info().Msgf("Rotating keys & updating manifests")
	var newPacketEncryptionKey key.Key
	if oldPacketEncryptionKey.IsEmpty() || cfg.packetCFG.enableRotation {
		k, err := oldPacketEncryptionKey.Rotate(cfg.now, cfg.packetCFG.rotationCFG)
		if err != nil {
			return fmt.Errorf("couldn't rotate packet encryption key for %q: %w", cfg.locality, err)
		}
		newPacketEncryptionKey = k
	} else {
		log.Info().Str("locality", cfg.locality).Msgf("Skipping rotation of packet encryption key for %q: --packet-encryption-key-enable-rotation set to false", cfg.locality)
		newPacketEncryptionKey = oldPacketEncryptionKey
	}

	newBatchSigningKeyByIngestor := map[string]key.Key{}
	for ingestor, oldKey := range oldBatchSigningKeyByIngestor {
		if oldKey.IsEmpty() || cfg.batchCFG.enableRotation {
			newKey, err := oldKey.Rotate(cfg.now, cfg.batchCFG.rotationCFG)
			if err != nil {
				return fmt.Errorf("couldn't rotate batch signing key for (%q, %q): %w",
					cfg.locality, ingestor, err)
			}
			newBatchSigningKeyByIngestor[ingestor] = newKey
		} else {
			log.Info().Str("locality", cfg.locality).Str("ingestor", ingestor).Msgf("Skipping rotation of batch signing key for (%q, %q): --batch-signing-key-enable-rotation set to false", cfg.locality, ingestor)
			newBatchSigningKeyByIngestor[ingestor] = oldKey
		}
	}

	// Update manifests.
	// We evaluate all manifests for update, not just manifests whose "input"
	// keys were modified by the rotation step, to account for the possibility
	// that a previous run managed to rotate & write some keys but then failed
	// at updating manifests. By re-evaluating manifests for update we will
	// re-attempt writing updated manifests on subsequent runs.
	newManifestByIngestor := map[string]manifest.DataShareProcessorSpecificManifest{}
	for ingestor, oldManifest := range oldManifestByIngestor {
		newManifest, err := oldManifest.UpdateKeys(manifest.UpdateKeysConfig{
			BatchSigningKey: newBatchSigningKeyByIngestor[ingestor],
			BatchSigningKeyIDPrefix: fmt.Sprintf(
				"%s-%s-%s-batch-signing-key", cfg.prioEnvironment, cfg.locality, ingestor),

			PacketEncryptionKey: newPacketEncryptionKey,
			PacketEncryptionKeyIDPrefix: fmt.Sprintf(
				"%s-%s-ingestion-packet-decryption-key", cfg.prioEnvironment, cfg.locality),
			PacketEncryptionKeyCSRFQDN: cfg.csrFQDN,
			SkipPreUpdateValidations:   cfg.skipManifestPreUpdateValidations,
			SkipPostUpdateValidations:  cfg.skipManifestPostUpdateValidations,
		})
		if err != nil {
			return fmt.Errorf("couldn't update manifest for (%q, %q): %w",
				cfg.locality, ingestor, err)
		}
		newManifestByIngestor[ingestor] = newManifest
	}

	// Write keys, then write manifests.
	// We write keys first so that on failure, we avoid the situation of having
	// written the public portion of a key to some manifest, while not having
	// written the associated private key to a secret (which would then be
	// lost).
	log.Info().Msgf("Writing keys")
	if err := writeKeys(ctx, cfg,
		oldPacketEncryptionKey, oldBatchSigningKeyByIngestor,
		newPacketEncryptionKey, newBatchSigningKeyByIngestor); err != nil {
		return fmt.Errorf("couldn't write keys: %w", err)
	}
	log.Info().Msgf("Writing manifests")
	if err := writeManifests(
		ctx, cfg,
		oldManifestByIngestor, newManifestByIngestor); err != nil {
		return fmt.Errorf("couldn't write manifests: %w", err)
	}
	return nil
}

func readKeysAndManifests(
	ctx context.Context, keyStore storage.Key,
	manifestStore storage.Manifest, locality string, ingestors []string,
) (packetEncryptionKey key.Key, batchSigningKeyByIngestor map[string]key.Key,
	manifestByIngestor map[string]manifest.DataShareProcessorSpecificManifest, _ error) {
	eg, ctx := errgroup.WithContext(ctx)
	var mu sync.Mutex                                                             // protects packetEncryptionKey, batchSigningKeyByIngestor, manifestByIngestor
	batchSigningKeyByIngestor = map[string]key.Key{}                              // ingestor -> batch signing key
	manifestByIngestor = map[string]manifest.DataShareProcessorSpecificManifest{} // ingestor -> manifest

	// Get packet encryption key.
	eg.Go(func() error {
		key, err := keyStore.GetPacketEncryptionKey(ctx, locality)
		if err != nil {
			return fmt.Errorf("couldn't get packet encryption key for %q: %w", locality, err)
		}
		mu.Lock()
		defer mu.Unlock()
		packetEncryptionKey = key
		return nil
	})

	for _, ingestor := range ingestors {
		ingestor := ingestor

		// Get batch signing keys.
		eg.Go(func() error {
			key, err := keyStore.GetBatchSigningKey(ctx, locality, ingestor)
			if err != nil {
				return fmt.Errorf("couldn't get batch signing for (%q, %q): %w",
					locality, ingestor, err)
			}
			mu.Lock()
			defer mu.Unlock()
			batchSigningKeyByIngestor[ingestor] = key
			return nil
		})

		// Get manifests.
		eg.Go(func() error {
			dspName := dspName(locality, ingestor)
			manifest, err := manifestStore.GetDataShareProcessorSpecificManifest(ctx, dspName)
			if err != nil {
				return fmt.Errorf("couldn't get manifest for (%q, %q): %w", locality, ingestor, err)
			}
			mu.Lock()
			defer mu.Unlock()
			manifestByIngestor[ingestor] = manifest
			return nil
		})
	}

	if err := eg.Wait(); err != nil {
		return key.Key{}, nil, nil, err
	}
	return packetEncryptionKey, batchSigningKeyByIngestor, manifestByIngestor, nil
}

func writeKeys(ctx context.Context, cfg rotateKeysConfig,
	oldPacketEncryptionKey key.Key, oldBatchSigningKeyByIngestor map[string]key.Key,
	newPacketEncryptionKey key.Key, newBatchSigningKeyByIngestor map[string]key.Key) error {
	eg, ctx := errgroup.WithContext(ctx)

	// Write packet encryption key.
	eg.Go(func() error {
		if !cfg.packetCFG.alwaysWrite && oldPacketEncryptionKey.Equal(newPacketEncryptionKey) {
			log.Debug().Str("locality", cfg.locality).Msgf("Skipping write for packet encryption key for %q: key unchanged", cfg.locality)
			return nil
		}

		diffs := newPacketEncryptionKey.Diff(oldPacketEncryptionKey)
		if cfg.packetCFG.alwaysWrite {
			diffs = semicolonJoin("--packet-encryption-key-always-write is specified", diffs)
		}
		log.Info().Str("locality", cfg.locality).Msgf("Writing packet encryption key for %q because: %s", cfg.locality, diffs)

		if err := cfg.keyStore.PutPacketEncryptionKey(ctx, cfg.locality, newPacketEncryptionKey); err != nil {
			return fmt.Errorf("couldn't write packet encryption key for %q: %w", cfg.locality, err)
		}
		keysWritten.Inc()
		return nil
	})

	// Write batch signing keys.
	for ingestor, oldKey := range oldBatchSigningKeyByIngestor {
		ingestor, oldKey, newKey := ingestor, oldKey, newBatchSigningKeyByIngestor[ingestor]
		eg.Go(func() error {
			if !cfg.batchCFG.alwaysWrite && oldKey.Equal(newKey) {
				log.Debug().Str("locality", cfg.locality).Str("ingestor", ingestor).Msgf("Skipping write for batch signing key for (%q, %q): key unchanged", cfg.locality, ingestor)
				return nil
			}

			diffs := newKey.Diff(oldKey)
			if cfg.batchCFG.alwaysWrite {
				diffs = semicolonJoin("--batch-signing-key-always-write is specified", diffs)
			}
			log.Info().Str("locality", cfg.locality).Str("ingestor", ingestor).Msgf("Writing batch signing key for (%q, %q) because: %s", cfg.locality, ingestor, diffs)

			if err := cfg.keyStore.PutBatchSigningKey(ctx, cfg.locality, ingestor, newKey); err != nil {
				return fmt.Errorf("couldn't write batch signing key for (%q, %q): %w", cfg.locality, ingestor, err)
			}
			keysWritten.Inc()
			return nil
		})
	}

	return eg.Wait()
}

func writeManifests(
	ctx context.Context, cfg rotateKeysConfig,
	oldManifestByIngestor, newManifestByIngestor map[string]manifest.DataShareProcessorSpecificManifest) error {
	eg, ctx := errgroup.WithContext(ctx)

	for ingestor, oldManifest := range oldManifestByIngestor {
		ingestor, oldManifest, newManifest := ingestor, oldManifest, newManifestByIngestor[ingestor]
		eg.Go(func() error {
			if oldManifest.Equal(newManifest) {
				log.Debug().Str("locality", cfg.locality).Str("ingestor", ingestor).Msgf("Skipping write for manifest for (%q, %q): key unchanged", cfg.locality, ingestor)
				return nil
			}
			log.Info().Str("locality", cfg.locality).Str("ingestor", ingestor).Msgf("Writing manifest for (%q, %q): %s", cfg.locality, ingestor, newManifest.Diff(oldManifest))
			if err := cfg.manifestStore.PutDataShareProcessorSpecificManifest(ctx, dspName(cfg.locality, ingestor), newManifest); err != nil {
				return fmt.Errorf("couldn't write manifest for (%q, %q): %w", cfg.locality, ingestor, err)
			}
			manifestsWritten.Inc()
			return nil
		})
	}

	return eg.Wait()
}

func dspName(locality, ingestor string) string { return fmt.Sprintf("%s-%s", locality, ingestor) }

func fail(format string, v ...interface{}) {
	lastFailure.SetToCurrentTime()
	if err := tryPushMetrics(); err != nil {
		log.Error().Msgf("Couldn't push metrics while failing: %v", err)
	}
	log.Fatal().Msgf(format, v...)
}

func tryPushMetrics() error {
	if pusher != nil {
		return pusher.Push()
	}
	return nil
}

// semicolonJoin joins the given values with "; ", dropping any empty values.
func semicolonJoin(vals ...string) string {
	var sb strings.Builder
	for _, v := range vals {
		if len(v) == 0 {
			continue
		}
		if sb.Len() > 0 {
			sb.WriteString("; ")
		}
		sb.WriteString(v)
	}
	return sb.String()
}

// dryRunKeyStore logs (but otherwise ignores) puts, and allows gets by
// deferring to the internal storage.Key's implementation.
type dryRunKeyStore struct{ k storage.Key }

var _ storage.Key = dryRunKeyStore{}

func (dryRunKeyStore) PutBatchSigningKey(_ context.Context, locality, ingestor string, _ key.Key) error {
	log.Info().Msgf("DRY RUN: would have written batch signing key for (%q, %q)", locality, ingestor)
	return nil
}

func (dryRunKeyStore) PutPacketEncryptionKey(_ context.Context, locality string, _ key.Key) error {
	log.Info().Msgf("DRY RUN: would have written packet encryption key for %q", locality)
	return nil
}

func (k dryRunKeyStore) GetBatchSigningKey(ctx context.Context, locality, ingestor string) (key.Key, error) {
	return k.k.GetBatchSigningKey(ctx, locality, ingestor)
}

func (k dryRunKeyStore) GetPacketEncryptionKey(ctx context.Context, locality string) (key.Key, error) {
	return k.k.GetPacketEncryptionKey(ctx, locality)
}

// dryRunManifestStore logs (but otherwise ignores) puts, and allows gets by
// deferring to the internal storage.Manifest's implementation.
type dryRunManifestStore struct{ m storage.Manifest }

var _ storage.Manifest = dryRunManifestStore{}

func (dryRunManifestStore) PutDataShareProcessorSpecificManifest(_ context.Context, dataShareProcessorName string, _ manifest.DataShareProcessorSpecificManifest) error {
	log.Info().Msgf("DRY RUN: would have written manifest for %q", dataShareProcessorName)
	return nil
}

func (dryRunManifestStore) PutIngestorGlobalManifest(context.Context, manifest.IngestorGlobalManifest) error {
	log.Info().Msgf("DRY RUN: would have written global manifest")
	return nil
}

func (m dryRunManifestStore) GetDataShareProcessorSpecificManifest(ctx context.Context, dataShareProcessorName string) (manifest.DataShareProcessorSpecificManifest, error) {
	return m.m.GetDataShareProcessorSpecificManifest(ctx, dataShareProcessorName)
}

func (m dryRunManifestStore) GetIngestorGlobalManifest(ctx context.Context) (manifest.IngestorGlobalManifest, error) {
	return m.m.GetIngestorGlobalManifest(ctx)
}

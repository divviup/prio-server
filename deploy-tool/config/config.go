package config

import (
	"github.com/pelletier/go-toml"
)

// DeployConfig is the base configuration structure
type DeployConfig struct {
	DNS     DNSConfig
	ACME    ACMEConfig
	Storage StorageConfig
}

// DNSConfig is the DNS configuration structure
type DNSConfig struct {
	Provider         string
	CloudflareConfig *CloudflareConfig `toml:"cloudflare"`
	GCPConfig        *GCPConfig        `toml:"gcp"`
}

// GCPConfig is the GCP configuration structure
// The ZoneMapping is used to map a certmagic zone request to a GCP DNS Zone.
// GCP DNS Zones can have arbitrary names. Certmagic Zones are the first level after the level the certificate is being issued for
// Example: issuing certificate for a.b.c.example.com, the zone would be b.c.example.com
type GCPConfig struct {
	Project     string
	ZoneMapping map[string]string `toml:"zone_mapping"`
}

// CloudflareConfig is the Cloudflare based DNS configuration - used only if Provider is set to cloudflare
type CloudflareConfig struct {
	APIKey string `toml:"api_key"`
}

// ACMEConfig is the ACME configuration structure
type ACMEConfig struct {
	Email               string `default:"enpa-prio-ops@letsencrypt.org"`
	ACMEApiEndpoint     string `toml:"acme_api_endpoint" default:"https://acme-v02.api.letsencrypt.org/directory"`
	SubscriberAgreement bool   `toml:"subscriber_agreement"`
}

// StorageConfig is the configuration for the storage system of deploy_tool
type StorageConfig struct {
	Driver     string `default:"kubernetes"`
	Filesystem *FilesystemConfig
	Kubernetes *KubernetesConfig
}

// FilesystemConfig is used when the driver in StorageConfig is set to filesystem
type FilesystemConfig struct {
	Path string `default:"./deploy_tool_output"`
}

// KubernetesConfig is used when the driver in StorageConfig is set to kubernetes
type KubernetesConfig struct {
	Namespace string `default:"key-rotator"`
}

// Read reads a file from a path as DeployConfig
func Read(path string) (DeployConfig, error) {
	var config DeployConfig

	tree, err := toml.LoadFile(path)
	if err != nil {
		return config, err
	}
	err = tree.Unmarshal(&config)

	return config, err
}

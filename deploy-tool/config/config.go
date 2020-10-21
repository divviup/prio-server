package config

import (
	"github.com/pelletier/go-toml"
)

type DeployConfig struct {
	DNS     DNSConfig
	ACME    ACMEConfig
	Storage StorageConfig
}

type DNSConfig struct {
	Provider         string
	CloudflareConfig *CloudflareConfig `toml:"cloudflare"`
	GCPConfig        *GCPConfig        `toml:"gcp"`
}

type GCPConfig struct {
	Project string
}

type CloudflareConfig struct {
	APIKey string `toml:"api_key"`
}

type ACMEConfig struct {
	Email               string `default:"enpa-prio-ops@letsencrypt.org"`
	ACMEApiEndpoint     string `toml:"acme_api_endpoint" default:"https://acme-v02.api.letsencrypt.org/directory"`
	SubscriberAgreement bool   `toml:"subscriber_agreement"`
}

type StorageConfig struct {
	Driver     string `default:"kubernetes"`
	Filesystem *FilesystemConfig
	Kubernetes *KubernetesConfig
}

type FilesystemConfig struct {
	Path string `default:"./deploy_tool_output"`
}

type KubernetesConfig struct {
	Namespace string `default:"key-rotator"`
}

func Read(path string) (DeployConfig, error) {
	var config DeployConfig

	tree, err := toml.LoadFile(path)
	if err != nil {
		return config, err
	}
	err = tree.Unmarshal(&config)

	return config, err
}

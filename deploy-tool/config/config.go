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
	CloudflareConfig *CloudflareConfig `toml:"Cloudflare"`
}

type CloudflareConfig struct {
	APIKey string `toml:"api_key"`
}

type ACMEConfig struct {
	Email               string
	CA                  string
	SubscriberAgreement bool `toml:"subscriber_agreement"`
}

type StorageConfig struct {
	Driver string
}

func Read(path string) (DeployConfig, error) {
	var config DeployConfig

	x, err := toml.LoadFile(path)
	if err != nil {
		return config, err
	}
	err = x.Unmarshal(&config)

	return config, err
}

package config

import (
	"encoding/json"
	"fmt"
	"io/ioutil"
)

type Config struct {
	IngestionBuckets      IngestionBuckets      `json:"ingestion_buckets"`
	PeerValidationBuckets PeerValidationBuckets `json:"peer_validation_buckets"`
}

type IngestionBuckets map[string]string
type PeerValidationBuckets map[string]string

// New returns a new instance of Config
// New reads from the configPath provided
func New(configPath string) (Config, error) {
	configBytes, err := ioutil.ReadFile(configPath)
	if err != nil {
		return Config{}, fmt.Errorf("error when reading the config file: %v", err)
	}

	config := Config{}
	err = json.Unmarshal(configBytes, &config)
	if err != nil {
		return Config{}, fmt.Errorf("error when unmarshalling the config file: %v", err)
	}

	return config, nil
}

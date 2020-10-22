package dns

import (
	"fmt"
	"strings"

	"github.com/abetterinternet/prio-server/deploy-tool/config"
	"github.com/abetterinternet/prio-server/deploy-tool/dns/gcloud"
	"github.com/caddyserver/certmagic"
	"github.com/libdns/cloudflare"
)

// GetACMEDNSProvider configures an ACMEDNSProvider value to be used in cert generation
func GetACMEDNSProvider(deployConfig config.DeployConfig) (certmagic.ACMEDNSProvider, error) {
	switch strings.ToLower(deployConfig.DNS.Provider) {
	case "cloudflare":
		if deployConfig.DNS.CloudflareConfig == nil {
			return nil, fmt.Errorf("cloudflare configuration of the configuration file was nil")
		}
		provider := &cloudflare.Provider{
			APIToken: deployConfig.DNS.CloudflareConfig.APIKey,
		}

		return provider, nil

	case "gcp":
		if deployConfig.DNS.GCPConfig == nil {
			return nil, fmt.Errorf("gcp configuration of the configuration file was nil")
		}
		provider, _ := gcloud.NewGoogleDNSProvider(deployConfig.DNS.GCPConfig.Project)

		return provider, nil
	}

	return nil, fmt.Errorf("no valid provider selected")
}

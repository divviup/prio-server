package dns

import (
	"deploy-tool/config"
	"fmt"
	"github.com/caddyserver/certmagic"
	"github.com/libdns/cloudflare"
	"strings"
)

// GetACMEDNSProvider configures an ACMEDNSProvider value to be used in cert generation
func GetACMEDNSProvider(deployConfig config.DeployConfig) (certmagic.ACMEDNSProvider, error) {
	switch strings.ToLower(deployConfig.DNS.Provider) {
	case "cloudflare":
		cloudflareConfig := deployConfig.DNS.CloudflareConfig
		if cloudflareConfig == nil {
			return nil, fmt.Errorf("cloudflare configuration of the configuration was nil")
		}
		provider := &cloudflare.Provider{
			APIToken: deployConfig.DNS.CloudflareConfig.APIKey,
		}

		return provider, nil
	}

	return nil, fmt.Errorf("no valid provider selected")
}

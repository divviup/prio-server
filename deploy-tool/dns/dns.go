package dns

import (
	"fmt"
	"github.com/caddyserver/certmagic"
	"github.com/libdns/cloudflare"
	"os"
	"strings"
)

// GetACMEDNSProvider configures an ACMEDNSProvider value to be used in cert generation
func GetACMEDNSProvider() (certmagic.ACMEDNSProvider, error) {
	switch strings.ToLower(getDNSProvider()) {
	case "cloudflare":
		provider := &cloudflare.Provider{
			APIToken: getAPIToken(),
		}

		return provider, nil
	}

	return nil, fmt.Errorf("no valid provider selected")
}

func getDNSProvider() string {
	return os.Getenv("D_DNS_PROVIDER")
}

func getAPIToken() string {
	return os.Getenv("D_API_TOKEN")
}

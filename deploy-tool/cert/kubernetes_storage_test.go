package cert

import (
	"fmt"
	"testing"

	"github.com/caddyserver/certmagic"
)

func TestCleanKey(t *testing.T) {
	prefix := "acme."
	var tests = []struct {
		in, out string
	}{
		{"ABC", prefix + "ABC"},
		{"123", prefix + "123"},
		{"", prefix + ""},
		{"ABC$$123", prefix + "ABC123"},
	}

	for _, tt := range tests {
		testname := fmt.Sprintf("%s - %s", tt.in, tt.out)
		t.Run(testname, func(t *testing.T) {
			got := cleanKey(tt.in)
			if got != tt.out {
				t.Errorf("got %s, expected %s", got, tt.out)
			}
		})
	}
}

func TestStorageImplementationComplete(t *testing.T) {
	acme := certmagic.NewDefault()

	// Just a compile time check to see if we can use KubernetesSecretStorage as a certmagic storage system.
	acme.Storage = &KubernetesSecretStorage{
		Namespace:  "",
		KubeClient: nil,
	}
}

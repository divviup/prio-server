package cert

import (
	"deploy-tool/config"
	"testing"
)

func TestValidEmail(t *testing.T) {
	conf := config.DeployConfig{
		ACME: config.ACMEConfig{
			Email: "test@example.com",
		},
	}
	email, err := getUserEmail(conf)

	if err != nil {
		t.Error("Valid email errored.")
	}

	if email != "test@example.com" {
		t.Errorf("got: %s, expected %s", email, "test@example.com")
	}
}

func TestInvalidEmail(t *testing.T) {
	conf := config.DeployConfig{
		ACME: config.ACMEConfig{
			Email: "testexample.com",
		},
	}
	email, err := getUserEmail(conf)

	if err == nil {
		t.Errorf("Error expected, got: %s", email)
	}

}

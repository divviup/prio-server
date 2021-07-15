// Package aws contains utilities related to AWS
package aws

import (
	"fmt"

	"github.com/letsencrypt/prio-server/workflow-manager/tokenfetcher"

	"github.com/aws/aws-sdk-go/aws"
	"github.com/aws/aws-sdk-go/aws/credentials"
	"github.com/aws/aws-sdk-go/aws/credentials/stscreds"
	"github.com/aws/aws-sdk-go/aws/session"
	"github.com/aws/aws-sdk-go/service/sts"
)

func webIDP(sess *session.Session, identity string) (*credentials.Credentials, error) {
	stsSTS := sts.New(sess)
	roleSessionName := ""
	roleProvider := stscreds.NewWebIdentityRoleProviderWithToken(
		stsSTS, identity, roleSessionName, tokenfetcher.NewTokenFetcher("sts.amazonaws.com/gke-identity-federation"))

	return credentials.NewCredentials(roleProvider), nil
}

// ClientConfig returns a (Session, Config) pair suitable for passing to the
// New() functions for various AWS services. If identity contains a valid role
// ARN, the config will use a web identity role provider for that role.
func ClientConfig(region, identity string) (*session.Session, *aws.Config, error) {
	sess, err := session.NewSession()
	if err != nil {
		return nil, nil, fmt.Errorf("making AWS session: %w", err)
	}

	config := aws.NewConfig().WithRegion(region)
	if identity != "" {
		creds, err := webIDP(sess, identity)
		if err != nil {
			return nil, nil, err
		}
		config = config.WithCredentials(creds)
	}
	return sess, config, nil
}

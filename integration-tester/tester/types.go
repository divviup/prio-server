package tester

import (
	"log"

	"github.com/abetterinternet/prio-server/workflow-manager/kubernetes"
)

type Tester struct {
	kubeClient         *kubernetes.Client
	namespace          string
	name               string
	manifestFileUrl    string
	serviceAccountName string

	facilitatorImage string
	pushGateway      string
	peerIdentity     string
	awsAccountId     string
}

func New(
	kubeConfigPath,
	namespace, name,
	manifestFileUrl, serviceAccountName,
	facilitatorImage, pushGateway,
	peerIdentity, awsAccountId string,
	dryRun bool) *Tester {

	kubeClient, err := kubernetes.NewClient(namespace, kubeConfigPath, dryRun)

	if err != nil {
		log.Fatalf("error creating a new kubernetes client: %v", err)
	}

	return &Tester{
		kubeClient,
		namespace,
		name,
		manifestFileUrl,
		serviceAccountName,
		facilitatorImage,
		pushGateway,
		peerIdentity,
		awsAccountId,
	}
}

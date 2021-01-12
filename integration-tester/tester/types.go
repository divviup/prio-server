package tester

import (
	"log"

	"github.com/abetterinternet/prio-server/workflow-manager/kubernetes"
)

type Tester struct {
	kubeClient         *kubernetes.Client
	namespace          string
	name               string
	ownManifestUrl     string
	phaManifestUrl     string
	facilManifestUrl   string
	serviceAccountName string

	facilitatorImage string
	pushGateway      string
	awsAccountId     string
}

func New(
	kubeConfigPath,
	namespace, name,
	ownManifestUrl, phaManifestUrl, facilManifestUrl,
	serviceAccountName, facilitatorImage,
	pushGateway, awsAccountId string,
	dryRun bool) *Tester {

	kubeClient, err := kubernetes.NewClient(namespace, kubeConfigPath, dryRun)

	if err != nil {
		log.Fatalf("error creating a new kubernetes client: %v", err)
	}

	return &Tester{
		kubeClient,
		namespace, name,
		ownManifestUrl, phaManifestUrl, facilManifestUrl,
		serviceAccountName, facilitatorImage,
		pushGateway, awsAccountId,
	}
}

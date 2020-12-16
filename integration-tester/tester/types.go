package tester

type Tester struct {
	namespace          string
	name               string
	manifestFileUrl    string
	serviceAccountName string

	facilitatorImage string
	pushGateway      string
	peerIdentity     string
}

func New(
	namespace, name,
	manifestFileUrl, serviceAccountName,
	facilitatorImage, pushGateway,
	peerIdentity string) *Tester {
	return &Tester{
		namespace,
		name,
		manifestFileUrl,
		serviceAccountName,
		facilitatorImage,
		pushGateway,
		peerIdentity,
	}
}

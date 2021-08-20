module github.com/abetterinternet/prio-server/deploy-tool

go 1.15

require (
	cloud.google.com/go v0.88.0
	github.com/abetterinternet/prio-server/manifest-updater v0.0.0-00010101000000-000000000000
	google.golang.org/api v0.51.0
	google.golang.org/genproto v0.0.0-20210721163202-f1cecdd8b78a
	google.golang.org/grpc v1.39.0
	k8s.io/api v0.21.3
	k8s.io/apimachinery v0.22.1
	k8s.io/client-go v0.21.3
)

replace github.com/abetterinternet/prio-server/manifest-updater => ../manifest-updater

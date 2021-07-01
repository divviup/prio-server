module github.com/abetterinternet/prio-server/deploy-tool

go 1.15

require (
	cloud.google.com/go v0.85.0
	github.com/abetterinternet/prio-server/manifest-updater v0.0.0-00010101000000-000000000000
	google.golang.org/api v0.49.0
	google.golang.org/genproto v0.0.0-20210629200056-84d6f6074151
	google.golang.org/grpc v1.38.0
	k8s.io/api v0.21.2
	k8s.io/apimachinery v0.21.2
	k8s.io/client-go v0.21.2
)

replace github.com/abetterinternet/prio-server/manifest-updater => ../manifest-updater

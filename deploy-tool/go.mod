module github.com/abetterinternet/prio-server/deploy-tool

go 1.15

require (
	cloud.google.com/go v0.79.0
	github.com/abetterinternet/prio-server/manifest-updater v0.0.0-00010101000000-000000000000
	google.golang.org/api v0.41.0
	google.golang.org/genproto v0.0.0-20210310155132-4ce2db91004e
	k8s.io/api v0.19.3
	k8s.io/apimachinery v0.19.3
	k8s.io/client-go v0.19.3
)

replace github.com/abetterinternet/prio-server/manifest-updater => ../manifest-updater

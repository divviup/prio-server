module github.com/abetterinternet/prio-server/integration-tester

go 1.15

require (
	github.com/abetterinternet/prio-server/manifest-updater v0.0.0-00010101000000-000000000000
	github.com/spf13/cobra v1.1.1
	github.com/spf13/pflag v1.0.5
	github.com/spf13/viper v1.7.1
	k8s.io/api v0.20.0
	k8s.io/apimachinery v0.20.0
	k8s.io/client-go v0.20.0
)

replace github.com/abetterinternet/prio-server/manifest-updater => ../manifest-updater

module github.com/abetterinternet/prio-server/tester

go 1.15

require (
	github.com/abetterinternet/prio-server/manifest-updater v0.0.0-00010101000000-000000000000
	github.com/imdario/mergo v0.3.11 // indirect
	github.com/sirupsen/logrus v1.7.0 // indirect
	github.com/spf13/cobra v1.1.1
	github.com/spf13/pflag v1.0.5
	github.com/spf13/viper v1.7.1
	k8s.io/api v0.20.0
	k8s.io/apimachinery v0.20.0
	k8s.io/client-go v11.0.0+incompatible
	k8s.io/klog v1.0.0 // indirect
	k8s.io/utils v0.0.0-20201110183641-67b214c5f920 // indirect
)

replace github.com/abetterinternet/prio-server/manifest-updater => ../manifest-updater

module github.com/abetterinternet/prio-server/manifest-updater

go 1.15

require (
	cloud.google.com/go/storage v1.0.0
	github.com/aws/aws-sdk-go v1.38.45
	github.com/google/gofuzz v1.2.0 // indirect
	github.com/rs/zerolog v1.20.0
	github.com/sirupsen/logrus v1.7.0
	github.com/spf13/cobra v1.1.1
	github.com/spf13/viper v1.7.1
	k8s.io/api v0.19.3
	k8s.io/apimachinery v0.19.3
	k8s.io/client-go v0.19.3
	k8s.io/utils v0.0.0-20201104234853-8146046b121e // indirect
)

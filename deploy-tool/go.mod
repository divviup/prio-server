module github.com/abetterinternet/prio-server/deploy-tool

go 1.17

require (
	cloud.google.com/go/secretmanager v1.0.0
	github.com/abetterinternet/prio-server/key-rotator v0.0.0-00010101000000-000000000000
	github.com/abetterinternet/prio-server/manifest-updater v0.0.0-00010101000000-000000000000
	google.golang.org/api v0.57.0
	google.golang.org/genproto v0.0.0-20210924002016-3dee208752a0
	google.golang.org/grpc v1.40.0
	k8s.io/api v0.21.3
	k8s.io/apimachinery v0.21.3
	k8s.io/client-go v0.21.3
)

require (
	cloud.google.com/go v0.97.0 // indirect
	cloud.google.com/go/storage v1.10.0 // indirect
	github.com/aws/aws-sdk-go v1.38.45 // indirect
	github.com/davecgh/go-spew v1.1.1 // indirect
	github.com/go-logr/logr v0.4.0 // indirect
	github.com/gogo/protobuf v1.3.2 // indirect
	github.com/golang/groupcache v0.0.0-20200121045136-8c9f03a8e57e // indirect
	github.com/golang/protobuf v1.5.2 // indirect
	github.com/google/go-cmp v0.5.6 // indirect
	github.com/google/gofuzz v1.2.0 // indirect
	github.com/googleapis/gax-go/v2 v2.1.0 // indirect
	github.com/googleapis/gnostic v0.4.1 // indirect
	github.com/imdario/mergo v0.3.5 // indirect
	github.com/jmespath/go-jmespath v0.4.0 // indirect
	github.com/json-iterator/go v1.1.10 // indirect
	github.com/modern-go/concurrent v0.0.0-20180306012644-bacd9c7ef1dd // indirect
	github.com/modern-go/reflect2 v1.0.1 // indirect
	github.com/rs/zerolog v1.20.0 // indirect
	github.com/spf13/pflag v1.0.5 // indirect
	go.opencensus.io v0.23.0 // indirect
	golang.org/x/net v0.0.0-20210503060351-7fd8e65b6420 // indirect
	golang.org/x/oauth2 v0.0.0-20210819190943-2bc19b11175f // indirect
	golang.org/x/sys v0.0.0-20210908233432-aa78b53d3365 // indirect
	golang.org/x/term v0.0.0-20210220032956-6a3ed077a48d // indirect
	golang.org/x/text v0.3.6 // indirect
	golang.org/x/time v0.0.0-20210220033141-f8bda1e9f3ba // indirect
	golang.org/x/xerrors v0.0.0-20200804184101-5ec99f83aff1 // indirect
	google.golang.org/appengine v1.6.7 // indirect
	google.golang.org/protobuf v1.27.1 // indirect
	gopkg.in/inf.v0 v0.9.1 // indirect
	gopkg.in/yaml.v2 v2.4.0 // indirect
	k8s.io/klog/v2 v2.8.0 // indirect
	k8s.io/utils v0.0.0-20201110183641-67b214c5f920 // indirect
	sigs.k8s.io/structured-merge-diff/v4 v4.1.2 // indirect
	sigs.k8s.io/yaml v1.2.0 // indirect
)

replace github.com/abetterinternet/prio-server/manifest-updater => ../manifest-updater

replace github.com/abetterinternet/prio-server/key-rotator => ../key-rotator

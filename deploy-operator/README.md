# deploy-operator

This deploy-operator handlaes the key creation and rotation jobs for each locality we deploy to Kubernetes.

This operator defines a new Custom Resource Definition called Locality, which then uses those CRDs to schedule and manage key rotation cron jobs.

## Building and Deploying to Kubernetes

The project has a Makefile, which handles the deploying of this operator and the custom resource definition onto Kubernetes.

`make deploy` will deploy the controller on the cluster.
`make install` will install the custom resource definition on the cluster.

These commands use the kubernetes configuration in `~/.kube/config` to do the deployments.

## Releasing new container images

GitHub Actions is responsible for creating new DockerHub images for the controller. 

## Overview of the files
- `api/v1` defines the CRD.
    * zz_generated.deepcopy.go is automatically generated. (`make generate`)
- `config/`
    * `crd/bases/prio*` is generated from the `api/v1` directory. (`make manifests`)
    * The other files are not generated, but will be used for the final deploy and install of the controller to kubernetes. However if we do add new API types, or new controllers, some of these files will change.
- `controller` contains the go code that handles the operation of the controllers
- `hack` is used for code generation

For more information refer to [Kubebuilder][0], and the [Kubebuilder Book][1]

[0]: https://github.com/kubernetes-sigs/kubebuilder
[1]: https://book.kubebuilder.io/
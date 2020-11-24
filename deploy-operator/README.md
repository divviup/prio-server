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

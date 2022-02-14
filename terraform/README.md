# Prio server Terraform module

This Terraform module manages a [GKE](https://cloud.google.com/kubernetes-engine/docs) or EKS cluster which hosts a Prio data share processor. We create one cluster and one node pool in each region in which we operate, and then run each PHA's data share processor instance in its own Kubernetes namespace.

Consult the [`prio-server` onboarding guide](https://github.com/abetterinternet/docs/blob/main/prio-server/onboarding.md) for instructions on using this module.

We use a [Terraform remote backend](https://www.terraform.io/docs/backends/index.html) to manage the state of the deployment, and the state file resides in a [Google Cloud Storage](https://cloud.google.com/storage/docs) bucket. `terraform/Makefile` is set up to manage the remote state, including creating buckets as needed. To use the `Makefile` targets, set the `ENV` environment variable to something that matches one of the `tfvars` files in `terraform/variables`. For instance, `ENV=demo-gcp make plan` will source Terraform state from the remote state for `demo-gcp`. Try `ENV=<anything> make help` to get a list of targets.

## New clusters

To add a data share processor to support a new locality, add that locality's name to the `localities` variable in the relevant `variables/<environment>.tfvars` file.

To bring up a whole new cluster, follow the steps in the onboarding guide.

## Paired test environments

We have support for creating two paired test environments which can exchange validation shares, along with a convincing simulation of ingestion servers and a portal server. To do this, you will need to create two `.tfvars` files, and on top of the usual variables, each must contain a variable like:

    test_peer_environment = {
      env_with_ingestor    = "with-ingestor"
      env_without_ingestor = "without-ingestor"
    }

The values must correspond to the names of the environments you are using. Pick one of them to be the environment with ingestors. From there, you should be able to bring up the two environments using the instructions in the onboarding guide.

## Name collisions

When modifying Terraform modules, pay close attention to the naming of resources. Depending on whether you are managing cloud-level or Kubernetes-level resources, you are claiming unique names in different scopes. For instance, Kubernetes resources are often [namespaced](https://kubernetes.io/docs/concepts/overview/working-with-objects/namespaces/), and so only need unique names within that narrow scope. However, some Google Cloud Platform resources must have unique names per-project, meaning resources in two different environments deployed to the same GCP project could collide, and some resources like GCS or S3 buckets must have globally unique names. Sufficiently unique names can usually be constructed by interpolating environment name into a resource's name, and it pays to think ahead since some resource names cannot be reclaimed even after the resource is deleted (e.g., S3 buckets).

## Formatting

Our continuous integration is set up to do basic validation of Terraform files on pull requests. Besides any other testing, make sure to run `terraform fmt --recursive` or you will get build failures!

## Debugging

Debugging Terraform problems can be tricky since some providers emit terse and unhelpful error messages. To get more insight into what is going on, set the environment variable `TF_LOG=debug` when invoking `Makefile` targets like `apply` and Terraform will emit a huge amount of data, usually including HTTP requests to cloud platforms. See [Terraform documentation](https://www.terraform.io/docs/cli/config/environment-variables.html) for more on supported environment variables.

## EKS notes

### Updating the `vpc-cni` add-on

In EKS, customers are responsible for keeping the [`vpc-cni` add-on](https://docs.aws.amazon.com/eks/latest/userguide/pod-networking.html) up to date. We do this via Terraform, but there is a constraint that makes certain updates impossible for Terraform to handle for us: [you can only update one minor version at a time](https://docs.aws.amazon.com/eks/latest/userguide/managing-vpc-cni.html#updating-vpc-cni-eks-add-on). If you find yourself with a Terraform plan that would update the addon from something like `v1.7.5-eksbuild.2` to `v1.10.1-eksbuild.1`, then the apply will fail with an error like this:

    {
      RespMetadata: {
        StatusCode: 400,
        RequestID: "692a4d18-dfc4-4b5e-a875-c72e1f65932e"
      },
      AddonName: "vpc-cni",
      ClusterName: "your-cluster-name",
      Message_: "Updating VPC-CNI can only go up or down 1 minor version at a time"
    }

The workaround is to manually apply the addon updates between the version your cluster is on and the version you want to get to. Consult [AWS documentation](https://docs.aws.amazon.com/eks/latest/userguide/managing-vpc-cni.html#updating-vpc-cni-eks-add-on) for instructions on doing this in either the AWS console or the `aws` CLI.

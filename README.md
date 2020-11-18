# prio-server

This is ISRG's implementation server components for [Prio](https://crypto.stanford.edu/prio/), the privacy preserving statistics aggregation system.

`avro-schema` contains [Avro](https://avro.apache.org/docs/current/index.html) schema definitions for interoperation with other actors in the Prio system. `facilitator` contains the Rust implementation of ISRG's Prio facilitation server. `terraform` contains a Terraform module for deploying data share processor servers.

## Prio share processor workflow

![Prio workflow diagram](docs/prio-workflow.gv.svg)

This GitHub project implements the "facilitator" box in the diagram.

## Releases

We use a GitHub Action to build Docker images and push them to [DockerHub](https://hub.docker.com/repository/docker/letsencrypt/prio-facilitator). To cut a release and push, publish a release in [GitHub's releases UI](https://github.com/abetterinternet/prio-server/releases/new). Docker images will be automatically generated and pushed to DockerHub.

## Impersonating cloud service accounts

Because of this project's multi-cloud nature, we wind up using some unusual authentication flows to share data across different accounts, across different cloud providers. Debugging these authentication flows can be tricky so here is a short guide to simulating those flows from your workstation.

### Service account impersonation vs. role assumption

We find ourselves in a situation where we want to attach some set of permissions to a single object, which can be used by others to configure policies on resources they own, and then enable multiple authenticated entities to use that permissions object. Concretely, we create a single AWS IAM role so that peer data share processors can grant it access to all of their peer validation share buckets, and a single GCP service account so that the portal server operator can grant it access to the sum part bucket they control.

In GCP, workloads run as some _service account_. Another service account may be created to which particular roles or permissions are granted, such as read or write access to a cloud storage bucket (n.b.: in GCP, a _role_ is a group of _permissions_ which may be granted to an entity -- contrast with the AWS IAM notion of a role). In GCP terms, one service account can _impersonate_ another.

In AWS, a _role_ is itself an IAM entity to which policies and permissions may be attached (such as read or write access to a cloud storage bucket). A role has a _role assumption policy_ which governs whether and how other entities may _assume_ the role and thus make whatever API calls the role is permitted.

### Assuming an AWS IAM role from a Google Cloud Platform service account

In this flow, a GCP service account assumes an AWS IAM role using OIDC identity federation. This is representative of how our jobs in Google Kubernetes Engine authenticate to AWS. We use [Workload Identity](https://cloud.google.com/kubernetes-engine/docs/how-to/workload-identity) so that Kubernetes service accounts can be mapped to GCP service accounts (see `terraform/modules/kubernetes/kubernetes.tf` for more details). The steps below start from the GCP service account that our GKE Kubernetes service accounts are permitted to impersonate. You will need:

- The email of the GCP service account
- The OIDC audience to scope the OIDC token to (its value should match the role assumption policy on the AWS IAM role you are assuming, which you can find in the AWS console or via AWS IAM API)
- The IAM ARN of the AWS IAM role you are assuming

First, you need an _identity token_ from the GCP IAM service scoped to the correct _OIDC audience_.

    gcloud auth print-identity-token --impersonate-service-account=<SERVICE_ACCOUNT_EMAIL>@<YOUR_GCP_PROJECT>.iam.gserviceaccount.com --audiences="<AUDIENCE_VALUE>"

This will print out a Base64 encoded token, which you can write to a file. Be careful with this, as it will let anyone impersonate the service account for as long as the token is valid!

Next, you [set up a _profile_](https://docs.aws.amazon.com/cli/latest/userguide/cli-configure-role.htm) for your `aws` CLI to use that is configured to use that token. Add a couplet like this to your `~/.aws/config`:

    [profile google-web-identity]
    role_arn=arn:aws:iam::<YOUR AWS ACCOUNT>:role/<NAME OF THE AWS IAM ROLE>
    web_identity_token_file=/path/where/you/wrote/the/gcloud/token

Now, you can issue `aws` commands with the `--profile` flag to use the web identity token. For instance, you can try to list the contents of an S3 bucket:

    aws s3 ls s3://<BUCKET NAME> --profile=google-web-identity

You can omit the `--profile` to make the same request with your default AWS identity, probably a `user` in the account. This lets you compare what actions the IAM role is allowed to take versus which ones your `user` can do.

Eventually you will find that `aws` invocations fail because the token you got from `gcloud` expires. If that happens, request a new one with `gcloud` and update the file where you stored it.

### Assuming an AWS IAM role from your user account

In this flow, your privileged AWS user assumes a role. You will need the ARN of the AWS IAM role you wish to assume. [Set up a _profile_](https://docs.aws.amazon.com/cli/latest/userguide/cli-configure-role.htm) by adding a couplet like this to your `~/.aws/config`:

    [profile assume-role]
    role_arn=arn:aws:iam::<YOUR AWS ACCOUNT>:role/<NAME OF THE AWS IAM ROLE>
    source_profile=<YOUR USUAL PROFILE>

For `source_profile`, substitute a profile that permits you to authenticate as yourself, probably a `user` in the AWS account. Then you can issue `aws` commands to assume the named role:

    aws s3 ls s3://<BUCKET NAME> --profile=assume-role

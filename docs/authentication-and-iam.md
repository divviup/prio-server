# Authentication and IAM

`prio-server` supports running in either Google Kubernetes Engine or Amazon AWS Elastic Kubernetes service. In either configuration, it is a multi-cloud application, because we expect that the two `prio-server` deployments will run on different clouds to eliminate the possibility of a single cloud vendor being able to decrypt and reassemble Prio data shares and learning anything about individual data submissions. So an instance running in GKE will need to read and write to and from Amazon S3 buckets, and an instance running in EKS will need to do the same to Google Cloud Storage buckets. Fortunately, GCP and AWS IAM can be federated in both directions, enabling us to securely share cloud resources with no or minimal credential sharing. However, the authentication flows can get quite complicated. Since Rust lacks mature cloud platform SDKs, `facilitator` has to provide its own implementation of several authentication flows, and while `workflow-manager` benefits from robust, first-party SDKs, it remains our responsibility to configure GCP and AWS cloud resources in Terraform so that they can be accessed. This document attempts to enumerate the various authentication flows used in different deployments of `prio-server` and to clarify some concepts used in the client implementations.

## Kubernetes service accounts

A `prio-server` environment may support multiple localities (e.g., `us-mn` or `fr-fr`). In each locality, we create a [Kubernetes service account](https://kubernetes.io/docs/tasks/configure-pod-container/configure-service-account/) (KSA) and run the `workflow-manager` cronjob and `facilitator` deployments as that KSA. Specifically, Kubernetes makes an authentication token for the KSA available in the container filesystem. This token allows Kubernetes pods to authenticate to the Kubernetes API server, and can govern access to Kubernetes API resources (such as a secret or a job) but does not grant access to any cloud platform resources (such as an AWS S3 bucket or a GCP PubSub queue).

## Mapping Kubernetes service accounts to cloud IAM entities

Cloud resources that we create have ACLs, policies or permissions attached to them to restrict which entities may act on them. Providing an overview of either GCP or AWS IAM and their permissions schemes is outside of the scope of this document, so we will simply say that we define policies for AWS resources using _AWS IAM roles_ and for GCP resources using _GCP service accounts_.

In GCP, workloads run as some _service account_. Another service account may be created to which particular roles or permissions are granted, such as read or write access to a cloud storage bucket (n.b.: in GCP, a _role_ is a group of _permissions_ which may be granted to an entity -- contrast with the AWS IAM notion of a role). In GCP terms, one service account can _impersonate_ another.

In AWS, a _role_ is itself an IAM entity to which policies and permissions may be attached (such as read or write access to a cloud storage bucket). A role has a _role assumption policy_ which governs whether and how other entities may _assume_ the role and thus make whatever API calls the role is permitted.

Each platform has a different mechanism for exchanging a Kubernetes service account token for credentials at the cloud platform level.

### AWS

On AWS, we use [IAM roles for service accounts](https://docs.aws.amazon.com/eks/latest/userguide/iam-roles-for-service-accounts.html) to allow Kubernetes service accounts to assume an AWS IAM role. This requires associating an IAM OpenID Connect Identity Provider with the Kubernetes cluster, configuring a role assumption policy on the IAM role that federates trust to the cluster IAM OIDC Identity Provider and decorating the Kubernetes service account with a label referencing the AWS IAM role. Environment variables are injected into Kubernetes pods that permit AWS client libraries (`github.com/aws/aws-sdk-go/aws/credentials/stscreds.NewWebIdentityRoleProviderWithToken` in the Go world, `rusoto_sts::WebIdentityProvider::from_k8s_env()` in Rust) to connect to the AWS STS service and obtain credentials for an AWS IAM role (an access key ID, a secret key and an authentication token). With those credentials, a Kubernetes pod can then make API requests to various AWS services, and the policies attached to the IAM role will determine whether access is granted.

### GCP

Google Kubernetes Engine offers [workload identity](https://cloud.google.com/kubernetes-engine/docs/how-to/workload-identity) as its means of mapping Kubernetes service accounts to GCP service accounts. We enable workload identity on the GKE clusters we create, and then annotate the Kubernetes service accounts and GCP service accounts so that GCP IAM can determine which goes with which. Oauth tokens for GCP service accounts are made available to pods via an unauthenticated request to the GKE metadata service, which is exposed in each pod's network namespace. Those tokens may then be used as bearer tokens in `Authentication` headers on subsequent requests to GCP API endpoints.

This is implemented in `facilitator::gcp_oauth::DefaultTokenProvider::account_token_from_gke_metadata_service()` in `facilitator/src/gcp_oauth.rs`.

It is possible for a GCP service account to impersonate another service account, if the latter is configured to allow it. For example, every per-locality data share processor in a `prio-server` environment impersonates a single GCP service account to send sum parts to the portal server. This impersonation is achieved by sending a request to `iamcredentials.googleapis.com`, authenticated with the workload identity token obtained from the GKE metadata service.

This is implemented in `facilitator::gcp_oauth::GcpOauthTokenProvider::ensure_impersonated_service_account_oauth_token()` in `facilitator/src/gcp_oauth.rs`.

## Cross-cloud authentication

Pods will always need to assume an identity in the cloud they are running on in order to make API requests, but occasionally, they will need to assume or impersonate a further identity in order to access resources in a different cloud. For example, a data share processor hosted on AWS will eventually need to write objects to a storage bucket owned by its peer in Google Cloud Storage.

### Impersonating GCP service accounts from AWS

#### Service account key files

The simplest means of impersonating a GCP service account is a [service acount key file](https://cloud.google.com/iam/docs/creating-managing-service-account-keys). The GCP IAM service emits a JSON document containing credentials for a GCP service account. The JSON document is then placed into a Kubernetes secret and made available to the Kubernetes pods as an environment variable. The downside of this approach is that it requires sharing a GCP secret into AWS and that rotating the GCP credential means updating a Kubernetes secret in the EKS cluster.

In this authentication flow, neither the Kubernetes service account nor an AWS IAM role come into play. The Kubernetes pod uses the private key in the key file to sign a request to sign a JWT, sends it to `iamcredentials.googleapis.com` and obtains an Oauth token which can be presented as a bearer token in subsequent API requests to other GCP services.

This is implemented in `facilitator::gcp_oauth::DefaultTokenProvider::account_token_with_key_file()` in `facilitator/src/gcp_oauth.rs`.

#### [Workload identity pool](https://cloud.google.com/iam/docs/access-resources-aws)

Workload identity pools (not to be confused with GKE workload identity) allows federating GCP and AWS IAM together. A _workload identity pool_ and a _workload identity pool provider_ are created in the GCP project, configured to allow one or more AWS IAM entities (users or roles) to impersonate a GCP service account. The pod in EKS must obtain credentials for the AWS IAM role (via IAM roles for service accounts), then can use those credentials to construct a signed `sts:GetCallerIdentity`request. Instead of sending that request to `sts.amazonaws.com`, the pod serializes it into a JSON request to `sts.googleapis.com`. That GCP service reconstructs the serialized `sts:GetCallerIdentity` request and sends it to `sts.amazonaws.com`, proving that the EKS Kubernetes pod does have valid credentials for the particular AWS IAM role. `sts.googleapis.com` then provides a _federated access token_ to the pod, which can then be presented to `iamcredentials.googleapis.com` and exchanged for an Oauth token for a GCP service account.

The downside of this authentication flow is that it is more complicated both in terms of setup and configuration and on the hot or data path of making API requests. However, it is more secure since no secret keys need to be shared from GCP to AWS, and is more hands off once deployed because rotating a GCP service account key is transparent to the AWS side.

So, end to end, this flow goes:

    - Kubernetes service account token provided by Kubernetes API server to pod
    - Exchange Kubernetes service account token for IAM role credentials with `sts.amazonaws.com`
    - Exchange IAM role credentials for GCP federated access token with `sts.googleapis.com`
    - Exchange GCP federated access token for GCP service account token with `iamcredentials.googleapis.com`
    - Use GCP service account token to authenticate API request to e.g. `storage.googleapis.com`

Construction of the `GetCallerIdentity` token is implemented in `facilitator::aws_authentication::get_caller_identity_token()` in `facilitator/src/aws_credentials.rs`. The request to `sts.googleapis.com` is implemented in `facilitator::gcp_oauth::DefaultProvider::account_token_with_workload_identity_pool()` in `facilitator/src/gcp_oauth.rs`.

### Assuming AWS IAM roles from GCP

AWS supports identity federation with GCP using [OpenID Connect web identity](https://docs.aws.amazon.com/IAM/latest/UserGuide/id_roles_providers_oidc.html). When creating an IAM role, [we define a role assumption policy that federates trust to `accounts.google.com` for a particular GCP service account](https://docs.aws.amazon.com/IAM/latest/UserGuide/id_roles_create_for-idp_oidc.html). This enables us to obtain AWS IAM credentials for the role by presenting a request authenticated by a GCP service account to `sts:AssumeRoleWithWebIdentity`.

The end-to-end authentication flow:

    - Obtain GCP service account OIDC token from GKE metadata service
    - Exchange GCP service account token for AWS IAM credentials with `sts.amazonaws.com`
    - Use AWS IAM credentials to authenticate API request to e.g. `s3.amazonaws.com`

Obtaining the OIDC authentication token from the GKE metadata service is implemented in `facilitator::aws_credentials::Provider::new_web_identity_with_oidc()` in `facilitator/src/aws_authentication.rs`.

### Simulating authentication flows at the command line

It can be helpful to replicate these authenticating and impersonation flows using command line utilities like `gcloud` or `aws` to debug policies without having to run `prio-server`.

#### Assuming an AWS IAM role from a Google Cloud Platform service account

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

#### Assuming an AWS IAM role from your user account

In this flow, your privileged AWS user assumes a role. You will need the ARN of the AWS IAM role you wish to assume. [Set up a _profile_](https://docs.aws.amazon.com/cli/latest/userguide/cli-configure-role.htm) by adding a couplet like this to your `~/.aws/config`:

    [profile assume-role]
    role_arn=arn:aws:iam::<YOUR AWS ACCOUNT>:role/<NAME OF THE AWS IAM ROLE>
    source_profile=<YOUR USUAL PROFILE>

For `source_profile`, substitute a profile that permits you to authenticate as yourself, probably a `user` in the AWS account. Then you can issue `aws` commands to assume the named role:

    aws s3 ls s3://<BUCKET NAME> --profile=assume-role


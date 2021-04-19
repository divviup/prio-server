use crate::{
    config::Identity,
    http::{Method, RequestParameters, RetryingAgent},
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use rusoto_core::{
    credential::{
        AutoRefreshingProvider, AwsCredentials, CredentialsError, DefaultCredentialsProvider,
        ProvideAwsCredentials, Secret, Variable,
    },
    RusotoError, RusotoResult,
};
use rusoto_mock::MockCredentialsProvider;
use rusoto_sts::WebIdentityProvider;
use slog_scope::{debug, info};
use std::{
    boxed::Box,
    convert::From,
    env,
    fmt::{self, Debug, Display},
    sync::Arc,
};
use tokio::runtime::{Builder, Runtime};
use url::Url;

/// Constructs a basic runtime suitable for use in our single threaded context
pub(crate) fn basic_runtime() -> Result<Runtime> {
    Ok(Builder::new_current_thread().enable_all().build()?)
}

/// The Provider enum allows us to generically handle different scenarios for
/// authenticating to AWS APIs, while still having a concrete value that
/// implements provideAwsCredentials and which can easily be used with Rusoto's
/// various client objects. It also provides a convenient place to implement
/// std::fmt::Display, allowing facilitator's AWS clients to log something
/// useful about the identity they use.
/// There are a number of ways to shave this yak. We could have used the newtype
/// pattern on each ProvideAwsCredentials implementation we use, but that would
/// make it hard to figure out the appropriate return type for a factory method
/// or function. We could have used Box<dyn ProvideAwsCredentials>, but trait
/// objects are harder to use with Rusoto's API clients, especially since we
/// need the credential provider to implement Clone. This approach makes it easy
/// to eliminate boilerplate in facilitator.rs while also integrating gracefully
/// with rusoto_s3::S3Client and rusoto_sqs::SqsClient, at the cost of some
/// repetitive match arms in some of the enum's methods.
#[derive(Clone)]
pub enum Provider {
    /// Rusoto's default credentials provider, which attempts to source
    /// AWS credentials from environment variables or ~/.aws/credentials.
    /// Suitable for use when running on a development environment or most AWS
    /// contexts.
    /// https://github.com/rusoto/rusoto/blob/master/AWS-CREDENTIALS.md
    Default(DefaultCredentialsProvider),
    /// A Rusoto WebIdentityProvider configured to obtain AWS credentials from
    /// the Google Kubernetes Engine metadata service. Should only be used when
    /// running within GKE.
    WebIdentityWithOidc(AutoRefreshingProvider<WebIdentityProvider>),
    /// A Rusoto WebIdentityProvider configured via environment variables set by
    /// AWS Elastic Kubernetes Service. Should only be used when running within
    /// EKS (+Fargate?).
    WebIdentityFromKubernetesEnvironment(AutoRefreshingProvider<WebIdentityProvider>),
    /// Rusoto's mock credentials provider, wrapped in an Arc to provide
    /// Send + Sync. Should only be used in tests.
    Mock(Arc<MockCredentialsProvider>),
}

impl Provider {
    /// Instantiates an appropriate Provider based on the provided configuration
    /// values
    pub fn new(identity: Identity, use_default_provider: bool, purpose: &str) -> Result<Self> {
        match (use_default_provider, identity) {
            (true, _) => Self::new_default(),
            (_, Some(identity)) => Self::new_web_identity_with_oidc(identity, purpose.to_owned()),
            (_, None) => Self::new_web_identity_from_kubernetes_environment(),
        }
    }

    fn new_default() -> Result<Self> {
        Ok(Self::Default(DefaultCredentialsProvider::new().context(
            "failed to create default AWS credentials provider",
        )?))
    }

    /// Instantiates a mock credentials provider.
    pub fn new_mock() -> Self {
        Self::Mock(Arc::new(MockCredentialsProvider))
    }

    fn new_web_identity_from_kubernetes_environment() -> Result<Self> {
        Ok(Self::WebIdentityFromKubernetesEnvironment(
            AutoRefreshingProvider::new(WebIdentityProvider::from_k8s_env()).context(
                "failed to create autorefreshing web identity provider from kubernetes environment",
            )?,
        ))
    }

    // We use workload identity to map GCP service accounts to Kubernetes
    // service accounts and make an auth token for the GCP account available to
    // containers in the GKE metadata service. Sadly we can't use the Kubernetes
    // feature to automount a service account token, because that would provide
    // the token for the *Kubernetes* service account, not the GCP one.
    // See terraform/modules/gke/gke.tf and terraform/modules/kuberenetes/kubernetes.tf
    fn metadata_service_token_url() -> Url {
        Url::parse("http://metadata.google.internal:80/computeMetadata/v1/instance/service-accounts/default/identity")
            .expect("could not parse token metadata api url")
    }

    fn new_web_identity_with_oidc(iam_role: &str, purpose: String) -> Result<Self> {
        // When running in GKE, the token used to authenticate to AWS S3 is
        // available from the instance metadata service.
        // See terraform/modules/kubernetes/kubernetes.tf for discussion.
        // This dynamic variable lets us provide a callback for fetching tokens,
        // allowing Rusoto to automatically get new credentials if they expire
        // (which they do every hour).
        let iam_role_clone = iam_role.to_owned();
        let oidc_token_variable = Variable::dynamic(move || {
            debug!(
                "obtaining OIDC token from GKE metadata service for IAM role {} and purpose {}",
                iam_role_clone, purpose
            );
            let aws_account_id = env::var("AWS_ACCOUNT_ID").map_err(|e| {
                CredentialsError::new(format!(
                    "could not read AWS account ID from environment: {}",
                    e
                ))
            })?;

            // We use ureq for this request because it is designed to use purely
            // synchronous Rust. Ironically, we are in the context of an async
            // runtime when this callback is invoked, but because the closure is
            // not declared async, we cannot use .await to work with Futures.
            let mut url = Self::metadata_service_token_url();

            url.query_pairs_mut()
                .append_pair("audience", &format!("sts.amazonaws.com/{}", aws_account_id))
                .finish();

            let agent = RetryingAgent::default();
            let mut request = agent
                .prepare_request(RequestParameters {
                    url,
                    method: Method::Get,
                    ..Default::default()
                })
                .map_err(|e| CredentialsError::new(format!("failed to create request: {:?}", e)))?;

            request = request.set("Metadata-Flavor", "Google");

            let response = agent.call(request).map_err(|e| {
                CredentialsError::new(format!(
                    "failed to fetch {} auth token from metadata service: {:?}",
                    purpose, e
                ))
            })?;

            let token = response.into_string().map_err(|e| {
                CredentialsError::new(format!(
                    "failed to fetch {} auth token from metadata service: {}",
                    purpose, e
                ))
            })?;
            Ok(Secret::from(token))
        });

        let provider = AutoRefreshingProvider::new(WebIdentityProvider::new(
            oidc_token_variable,
            // The AWS role that we are assuming is provided via environment
            // variable. See terraform/modules/kubernetes/kubernetes.tf.
            iam_role,
            // The role ARN we assume is already bound to a specific facilitator
            // instance, so we don't get much from further scoping the role
            // assumption, unless we eventually introduce something like a job
            // ID that could then show up in AWS-side logs.
            // https://docs.aws.amazon.com/credref/latest/refdocs/setting-global-role_session_name.html
            Some(Variable::from_env_var_optional("AWS_ROLE_SESSION_NAME")),
        ))
        .context("failed to create auto refreshing credentials provider")?;

        Ok(Self::WebIdentityWithOidc(provider))
    }
}

impl Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Default(_) => write!(f, "ambient AWS credentials"),
            Self::WebIdentityWithOidc(p) => {
                // std::fmt::Error does not allow wrapping an error so we must
                // discard whatever Variable.resolve() might return. In
                // practice, it will always be Variable::static and so no error
                // is possible.
                let role_arn = p.get_ref().role_arn.resolve().map_err(|_| fmt::Error)?;
                write!(f, "{} via OIDC web identity", role_arn)
            }
            Self::WebIdentityFromKubernetesEnvironment(p) => {
                let role_arn = p.get_ref().role_arn.resolve().map_err(|_| fmt::Error)?;
                write!(
                    f,
                    "{} via web identity from Kubernetes environment",
                    role_arn
                )
            }
            Self::Mock(_) => write!(f, "mock credentials"),
        }
    }
}

#[async_trait]
impl ProvideAwsCredentials for Provider {
    async fn credentials(&self) -> Result<AwsCredentials, CredentialsError> {
        match self {
            Self::Default(p) => p.credentials().await,
            Self::WebIdentityWithOidc(p) => p.credentials().await,
            Self::WebIdentityFromKubernetesEnvironment(p) => p.credentials().await,
            Self::Mock(p) => p.credentials().await,
        }
    }
}

/// We attempt AWS API requests up to three times (i.e., two retries)
const MAX_ATTEMPT_COUNT: i32 = 3;

/// Calls the provided closure, retrying up to MAX_ATTEMPT_COUNT times if it
/// fails with RusotoError::HttpDispatch, which indicates a problem sending the
/// request such as the connection getting closed under us.
/// Additionally, in the case where there is an error while fetching credentials
/// before making an actual request, the error will be RusotoError::Credentials,
/// wrapping a rusoto_core::credential::CredentialsError, which can in turn
/// contain an HttpDispatchError! Sadly, CredentialsError does not preserve the
/// structure of the underlying error, just its message, so we must resort to
/// matching on a substring in order to detect it.
pub fn retry_request<F, T, E>(action: &str, mut f: F) -> RusotoResult<T, E>
where
    F: FnMut() -> RusotoResult<T, E>,
    E: Debug,
{
    let mut attempts = 0;
    loop {
        match f() {
            Err(err) if retryable(&err) => {
                attempts += 1;
                if attempts >= MAX_ATTEMPT_COUNT {
                    break Err(err);
                }
                info!(
                    "failed to {} (will retry {} more times): {:?}",
                    action,
                    MAX_ATTEMPT_COUNT - attempts,
                    err
                );
            }
            Err(err) => {
                debug!("encountered non retryable error: {:?}", err);
                break Err(err);
            }
            result => break result,
        }
    }
}

fn retryable<T>(error: &RusotoError<T>) -> bool {
    match error {
        RusotoError::HttpDispatch(_) => true,
        RusotoError::Credentials(err) if err.message.contains("Error during dispatch") => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use rusoto_core::request::HttpDispatchError;

    #[test]
    fn retry_request_success() {
        let mut attempts = 0;

        let result = retry_request::<_, _, RusotoError<String>>("successful request", || {
            attempts += 1;
            Ok("success")
        });
        assert_matches!(result, Ok("success"));
        assert_eq!(attempts, 1);
    }

    #[test]
    fn non_retryable_error() {
        let mut attempts = 0;
        let error_message = "not retryable";

        let result: RusotoResult<String, String> = retry_request("not retryable", || {
            attempts += 1;
            Err(RusotoError::Validation(error_message.to_string()))
        });
        assert_matches!(result, Err(RusotoError::Validation(e)) => {
            assert_eq!(e, error_message);
        });
        assert_eq!(attempts, 1);
    }

    #[test]
    fn retry_request_http_dispatch_error() {
        let mut attempts = 0;
        let error_message = "http_dispatch_error";

        let result: RusotoResult<String, String> = retry_request(error_message, || {
            attempts += 1;
            Err(RusotoError::HttpDispatch(HttpDispatchError::new(
                error_message.to_string(),
            )))
        });
        assert_matches!(result, Err(RusotoError::HttpDispatch(e)) => {
            assert_eq!(e, HttpDispatchError::new(error_message.to_string()));
        });
        assert_eq!(attempts, MAX_ATTEMPT_COUNT);
    }

    #[test]
    fn retry_request_credentials_http_dispatch_error() {
        let mut attempts = 0;
        let error_message = "something Error during dispatch something";

        let result: RusotoResult<String, String> =
            retry_request("credentials dispatch error", || {
                attempts += 1;
                Err(RusotoError::Credentials(CredentialsError::new(
                    error_message.to_string(),
                )))
            });
        assert_matches!(result, Err(RusotoError::Credentials(e)) => {
            assert_eq!(e.message, error_message);
        });
        assert_eq!(attempts, MAX_ATTEMPT_COUNT);
    }

    #[test]
    fn non_retryable_credentials_error() {
        let mut attempts = 0;
        let error_message = "not a dispatch error";

        let result: RusotoResult<String, String> =
            retry_request("non retryable credentials dispatch error", || {
                attempts += 1;
                Err(RusotoError::Credentials(CredentialsError::new(
                    error_message.to_string(),
                )))
            });
        assert_matches!(result, Err(RusotoError::Credentials(e)) => {
            assert_eq!(e.message, error_message);
        });
        assert_eq!(attempts, 1);
    }
}

use crate::{
    config::Identity,
    http::{Method, RequestParameters, RetryingAgent},
    retries,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use rusoto_core::proto::xml::{
    error::XmlErrorDeserializer,
    util::{find_start_element, XmlResponse},
};
use rusoto_core::{
    credential::{
        AutoRefreshingProvider, AwsCredentials, CredentialsError, DefaultCredentialsProvider,
        ProvideAwsCredentials, Secret, Variable,
    },
    RusotoError, RusotoResult,
};
use rusoto_mock::MockCredentialsProvider;
use rusoto_sts::WebIdentityProvider;
use slog_scope::debug;
use std::{
    boxed::Box,
    convert::From,
    default::Default,
    env,
    fmt::{self, Debug, Display},
    sync::Arc,
};
use tokio::runtime::{Builder, Runtime};
use url::Url;
use xml::EventReader;

/// Constructs a basic runtime suitable for use in our single threaded context
pub(crate) fn basic_runtime() -> Result<Runtime> {
    Ok(Builder::new_current_thread().enable_all().build()?)
}

/// The Provider enum allows us to generically handle different scenarios for
/// authenticating to AWS APIs, while still having a concrete value that
/// implements ProvideAwsCredentials and which can easily be used with Rusoto's
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
///
/// A note on thread safety and sharing Providers: each variant of this enum
/// wraps an implementation of ProvideAwsCredentials which in turn uses an
/// Arc<Mutex<>> to share the cached credentials across cloned instances. That
/// means that even if a single instance of Provider is .clone()d many times and
/// provided to multiple threads, they will efficiently share a single cached
/// credential.
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

            let response = agent.call(&request).map_err(|e| {
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

/// Calls the provided closure, retrying with exponential backoff on failure if
/// the error is retryable (see retryable() in this module and its comments for
/// details).
pub(crate) fn retry_request<F, T, E>(action: &str, f: F) -> RusotoResult<T, E>
where
    F: FnMut() -> RusotoResult<T, E>,
    E: Debug,
{
    retries::retry_request(action, f, |rusoto_error| retryable(rusoto_error))
}

/// Returns true if the error is transient and should be retried, false
/// otherwise.
fn retryable<T>(error: &RusotoError<T>) -> bool {
    match error {
        // RusotoError::HttpDispatch indicates a problem sending the request
        // such as a timeout or the connection getting closed under us. Rusoto
        // doesn't expose any structured information about the underlying
        // problem (`rusoto_core::request::HttpDispatchError` only wraps a
        // string) so we assume any `HttpDispatch` is retryable.
        RusotoError::HttpDispatch(_) => true,
        // Rusoto might need to use the AWS STS API to obtain credentials to use
        // with whatever service we are accessing. If something goes wrong
        // during that phase, the error will be `RusotoError::Credentials`,
        // wrapping a `rusoto_core::credential::CredentialsError`, which can in
        // turn contain an `HttpDispatchError`! Sadly, `CredentialsError` does
        // not preserve the structure of the underlying error, so we cannot
        // distinguish between access being denied or something like a timeout
        // or the STS API being temporarily unavailable. We err on the side of
        // caution and unconditionally retry in the face of a credentials error.
        // Exponential backoff means we shouldn't be abusing the AWS endpoint
        // too badly even if we are futilely retrying a request that cannot
        // succeed.
        RusotoError::Credentials(_) => true,
        // Finally, any AWS error that isn't explicitly handled by
        // `RusotoError::Service` will be reported as `RusotoError::Unknown`.
        // Happily this variant does let us see the HTTP response, including the
        // status code, so we can be more selective about when to retry. AWS'
        // general guidance on retries[1] is that we should "should retry
        // original requests that receive server (5xx) or throttling errors".
        // Per the AWS S3[2] and SQS[3] API guides, throttling errors may appear
        // as HTTP 503 or HTTP 403 with error code ThrottlingException. Sadly
        // there are other causes of HTTP 403 than throttling, so we must
        // examine the response's *error* code (not the HTTP status code). To
        // get at the error code we have to parse the XML response body, using
        // code borrowed from Rusoto[4].
        //
        // [1]: https://docs.aws.amazon.com/general/latest/gr/api-retries.html
        // [2]: https://docs.aws.amazon.com/AmazonS3/latest/API/ErrorResponses.html#ErrorCodeList
        // [3]: https://docs.aws.amazon.com/AWSSimpleQueueService/latest/APIReference/CommonErrors.html
        // [4]: https://github.com/rusoto/rusoto/blob/31cf1506f9f4bd7af7a1f86ced3cec436913d518/rusoto/services/sqs/src/generated.rs#L2796
        RusotoError::Unknown(response) if response.status.is_server_error() => true,
        RusotoError::Unknown(response) if response.status == 403 => {
            let reader = EventReader::new(response.body.as_ref());
            let mut stack = XmlResponse::new(reader.into_iter().peekable());
            find_start_element(&mut stack);
            if let Ok(parsed_error) = XmlErrorDeserializer::deserialize("Error", &mut stack) {
                parsed_error.code == "ThrottlingException"
            } else {
                false
            }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use http::{status::StatusCode, HeaderMap};
    use rusoto_core::request::{BufferedHttpResponse, HttpDispatchError};

    #[test]
    fn error_is_retryable() {
        struct TestCase {
            // RusotoError<bool> is nonsense, but the compiler insists that we
            // specify what specialization of RusotoError we use
            error: RusotoError<bool>,
            is_retryable: bool,
        }

        // XML representation of AWS errors obtained from production logs
        let throttling_exception_xml = r#"
<Error>
    <Code>ThrottlingException</Code>
    <Message>test</Message>
    <RequestId>W7E5GN21PBFBZWHV</RequestId>
    <HostId>l7cXCunu8uBfGqFG9bVFVUouCLPihMJlC2EUqFHb484TMNpNixSsFxBvLYKQpu373YHhkIaaEQ8=</HostId>
</Error>
"#;

        let other_exception_xml = r#"
<Error>
    <Code>SomeOtherException</Code>
    <Message>test</Message>
    <RequestId>W7E5GN21PBFBZWHV</RequestId>
    <HostId>l7cXCunu8uBfGqFG9bVFVUouCLPihMJlC2EUqFHb484TMNpNixSsFxBvLYKQpu373YHhkIaaEQ8=</HostId>
</Error>
"#;

        let test_cases = vec![
            TestCase {
                error: RusotoError::HttpDispatch(HttpDispatchError::new(
                    "http_dispatch_error".to_string(),
                )),
                is_retryable: true,
            },
            TestCase {
                error: RusotoError::Credentials(CredentialsError::new("message")),
                is_retryable: true,
            },
            TestCase {
                error: RusotoError::Service(true),
                is_retryable: false,
            },
            TestCase {
                error: RusotoError::Validation("message".to_string()),
                is_retryable: false,
            },
            TestCase {
                error: RusotoError::ParseError("message".to_string()),
                is_retryable: false,
            },
            TestCase {
                error: RusotoError::Blocking,
                is_retryable: false,
            },
            TestCase {
                error: RusotoError::Unknown(BufferedHttpResponse {
                    status: StatusCode::from_u16(404).unwrap(),
                    headers: HeaderMap::with_capacity(0),
                    body: Bytes::new(),
                }),
                is_retryable: false,
            },
            TestCase {
                error: RusotoError::Unknown(BufferedHttpResponse {
                    status: StatusCode::from_u16(500).unwrap(),
                    headers: HeaderMap::with_capacity(0),
                    body: Bytes::new(),
                }),
                is_retryable: true,
            },
            TestCase {
                error: RusotoError::Unknown(BufferedHttpResponse {
                    status: StatusCode::from_u16(403).unwrap(),
                    headers: HeaderMap::with_capacity(0),
                    body: Bytes::from_static(throttling_exception_xml.as_bytes()),
                }),
                is_retryable: true,
            },
            TestCase {
                error: RusotoError::Unknown(BufferedHttpResponse {
                    status: StatusCode::from_u16(403).unwrap(),
                    headers: HeaderMap::with_capacity(0),
                    body: Bytes::from_static(other_exception_xml.as_bytes()),
                }),
                is_retryable: false,
            },
        ];

        for test_case in test_cases {
            assert_eq!(retryable(&test_case.error), test_case.is_retryable);
        }
    }
}

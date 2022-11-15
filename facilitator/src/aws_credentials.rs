use crate::{
    config::Identity,
    gcp_oauth::{
        AccessScope, GcpAccessTokenProviderFactory, GkeMetadataServiceIdentityTokenProvider,
        ImpersonatedServiceAccountIdentityTokenProvider, ProvideGcpIdentityToken,
    },
    metrics::ApiClientMetricsCollector,
    retries,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{prelude::Utc, DateTime};
use hmac::{digest::InvalidLength, Hmac, Mac};
use http::header::{HeaderMap, HeaderName, InvalidHeaderValue};
use rusoto_core::{
    credential::{
        AutoRefreshingProvider, AwsCredentials, ChainProvider, CredentialsError,
        ProvideAwsCredentials, Secret, Variable,
    },
    proto::xml::{
        error::XmlErrorDeserializer,
        util::{find_start_element, XmlResponse},
    },
    region::ParseRegionError,
    Region, RusotoError, RusotoResult,
};
#[cfg(test)]
use rusoto_mock::MockCredentialsProvider;
use rusoto_sts::WebIdentityProvider;
use sha2::{Digest, Sha256};
use slog::{debug, o, Logger};
#[cfg(test)]
use std::sync::Arc;
use std::{
    boxed::Box,
    collections::HashMap,
    convert::From,
    default::Default,
    fmt::{self, Debug, Display},
    future::Future,
    io::{self, ErrorKind},
    str,
    time::{Duration, Instant},
};
use tokio::{runtime::Handle, time::timeout};
use url::Url;
use xml::EventReader;

const LONG_DATETIME: &str = "%Y%m%dT%H%M%SZ";
const SHORT_DATE: &str = "%Y%m%d";
const HOST_HEADER: &str = "host";
const AMAZON_DATE_HEADER: &str = "x-amz-date";
const AMAZON_SECURITY_TOKEN_HEADER: &str = "x-amz-security-token";
const GOOGLE_TARGET_RESOURCE_HEADER: &str = "x-goog-cloud-target-resource";

static CREDENTIAL_ENDPOINT_LABEL: &str = "provide_credentials";

// The following service names are for the purposes of labeling metrics. They are
// not literally domains of services, but this is as fine grained as we can get from
// outside the AWS SDK.
static DEFAULT_CREDENTIALS_SERVICE_NAME: &str = "aws_credentials_from_environment";
static WEB_IDENTITY_WITH_OIDC_SERVICE_NAME: &str = "aws_web_identity_from_oidc";
static WEB_IDENTITY_FROM_KUBERNETES_ENVIRONMENT_SERVICE_NAME: &str =
    "aws_web_identity_from_kubernetes";
#[cfg(test)]
static MOCK_CREDENTIALS_SERVICE_NAME: &str = "aws_mock_credentials";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ProviderFactoryKey {
    aws_identity: Identity,
    impersonate_gcp_service_account: Identity,
    use_default_provider: bool,
    purpose: &'static str,
}

/// Manages a cache of credentials providers keyed by the identity for which
/// credentials are being obtained, for efficient credential caching across
/// multiple API clients.
#[derive(Clone, Debug)]
pub struct ProviderFactory {
    gcp_access_token_provider_factory: GcpAccessTokenProviderFactory,
    api_metrics: ApiClientMetricsCollector,
    logger: Logger,
    providers: HashMap<ProviderFactoryKey, Provider>,
}

impl ProviderFactory {
    /// Creates a new credential provider factory. The provided `api_metrics`,
    /// `runtime_handle` and `logger` will be used when creating any `Provider`.
    pub fn new(
        gcp_access_token_provider_factory: &GcpAccessTokenProviderFactory,
        api_metrics: &ApiClientMetricsCollector,
        logger: &Logger,
    ) -> Self {
        Self {
            gcp_access_token_provider_factory: gcp_access_token_provider_factory.clone(),
            api_metrics: api_metrics.clone(),
            logger: logger.clone(),
            providers: HashMap::new(),
        }
    }

    pub fn get(
        &mut self,
        aws_identity: Identity,
        impersonate_gcp_service_account: Identity,
        use_default_provider: bool,
        purpose: &'static str,
    ) -> Result<Provider> {
        let key = ProviderFactoryKey {
            aws_identity: aws_identity.clone(),
            impersonate_gcp_service_account: impersonate_gcp_service_account.clone(),
            use_default_provider,
            purpose,
        };
        match self.providers.get(&key) {
            Some(provider) => {
                debug!(
                    self.logger,
                    "reusing cached AWS credentials provider {:?}", key
                );
                Ok(provider.clone())
            }
            None => {
                debug!(
                    self.logger,
                    "creating new AWS credentials provider {:?}", key
                );
                let provider = Provider::new(
                    aws_identity,
                    impersonate_gcp_service_account,
                    use_default_provider,
                    purpose,
                    &mut self.gcp_access_token_provider_factory,
                    &self.logger,
                    &self.api_metrics,
                )?;
                self.providers.insert(key, provider.clone());
                Ok(provider)
            }
        }
    }
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
#[allow(clippy::large_enum_variant)]
pub enum Provider {
    /// Based on Rusoto's default credentials provider, which attempts to source
    /// AWS credentials from environment variables or ~/.aws/credentials.
    /// Suitable for use when running on a development environment or most AWS
    /// contexts.
    /// https://github.com/rusoto/rusoto/blob/master/AWS-CREDENTIALS.md
    Default(AutoRefreshingProvider<MeasuringProvider<ChainProvider>>),
    /// A Rusoto WebIdentityProvider configured to obtain AWS credentials from
    /// the Google Kubernetes Engine metadata service. Should only be used when
    /// running within GKE.
    WebIdentityWithOidc(
        AutoRefreshingProvider<RetryingProvider<MeasuringProvider<WebIdentityProvider>>>,
    ),
    /// A Rusoto WebIdentityProvider configured via environment variables set by
    /// AWS Elastic Kubernetes Service. Should only be used when running within
    /// EKS (+Fargate?).
    WebIdentityFromKubernetesEnvironment(
        AutoRefreshingProvider<RetryingProvider<MeasuringProvider<WebIdentityProvider>>>,
    ),
    /// Rusoto's mock credentials provider, wrapped in an Arc to provide
    /// Send + Sync. Should only be used in tests.
    #[cfg(test)]
    Mock(RetryingProvider<MeasuringProvider<Arc<MockCredentialsProvider>>>),
}

impl Provider {
    /// Instantiates an appropriate Provider based on the provided configuration
    /// values.
    /// `aws_identity` is the AWS IAM entity (typically a role) for which this
    /// provider will provide credentials.
    /// `impersonate_gcp_service_account` is a GCP service account which should
    /// be impersonated to obtain identity tokens allowing assumption of
    /// `aws_identity`.
    /// If `use_default_provider` is true, then ambient AWS credentials from the
    /// environment or ~/.aws/credentials will be used and other arguments are
    /// ignored.
    fn new(
        aws_identity: Identity,
        impersonate_gcp_service_account: Identity,
        use_default_provider: bool,
        purpose: &'static str,
        gcp_access_token_provider_factory: &mut GcpAccessTokenProviderFactory,
        logger: &Logger,
        api_metrics: &ApiClientMetricsCollector,
    ) -> Result<Self> {
        match (use_default_provider, aws_identity.as_str()) {
            (true, _) => Self::new_default(api_metrics),
            (_, Some(identity)) => Self::new_web_identity_with_oidc(
                identity,
                purpose,
                impersonate_gcp_service_account,
                gcp_access_token_provider_factory,
                logger,
                api_metrics,
            ),
            (_, None) => Self::new_web_identity_from_kubernetes_environment(logger, api_metrics),
        }
    }

    /// Instantiates a mock credentials provider.
    #[cfg(test)]
    pub(crate) fn new_mock(logger: &Logger, api_metrics: &ApiClientMetricsCollector) -> Self {
        let arc = Arc::new(MockCredentialsProvider);
        let measuring_provider = MeasuringProvider::new(
            arc,
            MOCK_CREDENTIALS_SERVICE_NAME,
            CREDENTIAL_ENDPOINT_LABEL,
            api_metrics,
        );
        let retrying_provider = RetryingProvider::new(measuring_provider, logger);
        Self::Mock(retrying_provider)
    }

    fn new_default(api_metrics: &ApiClientMetricsCollector) -> Result<Self> {
        let base_provider = ChainProvider::new();
        let measuring_provider = MeasuringProvider::new(
            base_provider,
            DEFAULT_CREDENTIALS_SERVICE_NAME,
            CREDENTIAL_ENDPOINT_LABEL,
            api_metrics,
        );
        let auto_refreshing_provider = AutoRefreshingProvider::new(measuring_provider)
            .context("failed to create autorefreshing provider from chain provider")?;
        Ok(Self::Default(auto_refreshing_provider))
    }

    fn new_web_identity_from_kubernetes_environment(
        logger: &Logger,
        api_metrics: &ApiClientMetricsCollector,
    ) -> Result<Self> {
        let base_provider = WebIdentityProvider::from_k8s_env();
        let measuring_provider = MeasuringProvider::new(
            base_provider,
            WEB_IDENTITY_FROM_KUBERNETES_ENVIRONMENT_SERVICE_NAME,
            CREDENTIAL_ENDPOINT_LABEL,
            api_metrics,
        );
        let retrying_provider = RetryingProvider::new(measuring_provider, logger);
        let auto_refreshing_provider = AutoRefreshingProvider::new(retrying_provider).context(
            "failed to create autorefreshing web identity provider from kubernetes environment",
        )?;
        Ok(Self::WebIdentityFromKubernetesEnvironment(
            auto_refreshing_provider,
        ))
    }

    fn new_web_identity_with_oidc(
        iam_role: &str,
        purpose: &'static str,
        impersonated_gcp_service_account: Identity,
        gcp_access_token_provider_factory: &mut GcpAccessTokenProviderFactory,
        logger: &Logger,
        api_metrics: &ApiClientMetricsCollector,
    ) -> Result<Self> {
        // When running in GKE, we obtain an identity token which we can then
        // present to sts.amazonaws.com to get credentials that can authenticate
        // to AWS S3.
        // This dynamic variable lets us provide a callback for fetching tokens,
        // allowing Rusoto to automatically get new credentials if they expire
        // (which they do every hour).
        let token_logger = logger.new(o!(
            "iam_role" => iam_role.to_owned(),
            "purpose" => purpose,
        ));
        let token_provider: Box<dyn ProvideGcpIdentityToken> =
            match impersonated_gcp_service_account.as_str() {
                Some(impersonated_gcp_service_account) => {
                    Box::new(ImpersonatedServiceAccountIdentityTokenProvider::new(
                        impersonated_gcp_service_account.to_owned(),
                        gcp_access_token_provider_factory.get(
                            AccessScope::CloudPlatform,
                            Identity::none(),
                            None,
                            None,
                        )?,
                        token_logger.clone(),
                        api_metrics,
                    ))
                }
                None => Box::new(GkeMetadataServiceIdentityTokenProvider::new(
                    token_logger.clone(),
                    api_metrics,
                )),
            };
        let oidc_token_variable = Variable::dynamic(move || {
            let token = token_provider.identity_token().map_err(|e| {
                CredentialsError::new(format!(
                    "failed to fetch {} identity token from GCP: {}",
                    purpose, e
                ))
            })?;
            Ok(Secret::from(token))
        });

        let base_provider = WebIdentityProvider::new(
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
        );
        let measuring_provider = MeasuringProvider::new(
            base_provider,
            WEB_IDENTITY_WITH_OIDC_SERVICE_NAME,
            CREDENTIAL_ENDPOINT_LABEL,
            api_metrics,
        );
        let retrying_provider = RetryingProvider::new(measuring_provider, logger);
        let auto_refreshing_provider = AutoRefreshingProvider::new(retrying_provider)
            .context("failed to create auto refreshing credentials provider")?;

        Ok(Self::WebIdentityWithOidc(auto_refreshing_provider))
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
                let role_arn_res = p.get_ref().get_ref().get_ref().role_arn.resolve();
                let role_arn = role_arn_res
                    .as_ref()
                    .map(String::as_str)
                    .unwrap_or_else(|_| "(error resolving variable)");
                write!(f, "{} via OIDC web identity", role_arn)
            }
            Self::WebIdentityFromKubernetesEnvironment(p) => {
                let role_arn_res = p.get_ref().get_ref().get_ref().role_arn.resolve();
                let role_arn = role_arn_res
                    .as_ref()
                    .map(String::as_str)
                    .unwrap_or_else(|_| "(error resolving variable)");
                write!(
                    f,
                    "{} via web identity from Kubernetes environment",
                    role_arn
                )
            }
            #[cfg(test)]
            Self::Mock(_) => write!(f, "mock credentials"),
        }
    }
}

impl Debug for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Provider")
            .field("credential_source", &self.to_string())
            .finish()
    }
}

#[async_trait]
impl ProvideAwsCredentials for Provider {
    async fn credentials(&self) -> Result<AwsCredentials, CredentialsError> {
        match self {
            Self::Default(p) => p.credentials().await,
            Self::WebIdentityWithOidc(p) => p.credentials().await,
            Self::WebIdentityFromKubernetesEnvironment(p) => p.credentials().await,
            #[cfg(test)]
            Self::Mock(p) => p.credentials().await,
        }
    }
}

/// Wraps an AWS credential provider and records API call latencies when
/// credentials are fetched.
#[derive(Clone)]
pub struct MeasuringProvider<P> {
    inner: P,
    service: &'static str,
    endpoint: &'static str,
    api_metrics: ApiClientMetricsCollector,
}

impl<P> MeasuringProvider<P> {
    pub fn new(
        inner: P,
        service: &'static str,
        endpoint: &'static str,
        api_metrics: &ApiClientMetricsCollector,
    ) -> MeasuringProvider<P> {
        MeasuringProvider {
            inner,
            service,
            endpoint,
            api_metrics: api_metrics.clone(),
        }
    }

    /// Get a shared reference to the inner provider.
    pub fn get_ref(&self) -> &P {
        &self.inner
    }

    /// Get a mutable reference to the inner provider.
    pub fn get_mut(&mut self) -> &mut P {
        &mut self.inner
    }
}

#[async_trait]
impl<P: ProvideAwsCredentials + Send + Sync> ProvideAwsCredentials for MeasuringProvider<P> {
    async fn credentials(&self) -> Result<AwsCredentials, CredentialsError> {
        let before = Instant::now();
        let result = self.inner.credentials().await;
        let latency = before.elapsed().as_millis();

        // Rusoto's CredentialsError doesn't let us see what if any HTTP code
        // was returned by the server, so the best we can do is report success
        // or failure
        let status_label = if result.is_ok() { "success" } else { "failure" };

        self.api_metrics
            .latency
            .with_label_values(&[self.service, self.endpoint, status_label])
            .observe(latency as f64);

        result
    }
}

/// Wraps an AWS credential provider and performs retries with exponential
/// backoff upon failure.
#[derive(Clone)]
pub struct RetryingProvider<P> {
    inner: P,
    logger: Logger,
}

impl<P> RetryingProvider<P> {
    pub fn new(inner: P, logger: &Logger) -> RetryingProvider<P> {
        RetryingProvider {
            inner,
            logger: logger.clone(),
        }
    }

    /// Get a shared reference to the inner provider.
    pub fn get_ref(&self) -> &P {
        &self.inner
    }

    /// Get a mutable reference to the inner provider.
    pub fn get_mut(&mut self) -> &mut P {
        &mut self.inner
    }
}

#[async_trait]
impl<P: ProvideAwsCredentials + Send + Sync> ProvideAwsCredentials for RetryingProvider<P> {
    async fn credentials(&self) -> Result<AwsCredentials, CredentialsError> {
        retries::retry_request_future(
            &self.logger,
            || async {
                let future = self.inner.credentials();
                timeout(Duration::from_secs(15), future)
                    .await
                    .map_err(CredentialsError::new)?
            },
            |_error| {
                // rusoto flattens the error into a string, so we will always retry for now.
                true
            },
        )
        .await
    }
}

/// Calls the provided closure, retrying with exponential backoff on failure if
/// the error is retryable (see retryable() in this module and its comments for
/// details). The latency and status of each attempt is recorded in the provided
/// metrics collector.
pub(crate) fn retry_request<FN, F, T, E>(
    logger: &Logger,
    api_metrics: &ApiClientMetricsCollector,
    service: &str,
    endpoint: &str,
    runtime_handle: &Handle,
    mut f: FN,
) -> RusotoResult<T, E>
where
    F: Future<Output = RusotoResult<T, E>>,
    FN: FnMut() -> F,
    E: Debug,
{
    retries::retry_request(
        logger,
        || {
            let before = Instant::now();
            let future = f();
            let result = runtime_handle.block_on(async {
                timeout(Duration::from_secs(600), future)
                    .await
                    .map_err(|err| RusotoError::from(io::Error::new(ErrorKind::TimedOut, err)))?
            });
            let latency = before.elapsed().as_millis();

            api_metrics
                .latency
                .with_label_values(&[service, endpoint, status_label(&result)])
                .observe(latency as f64);

            result
        },
        |rusoto_error| retryable(rusoto_error),
    )
}

/// Returns a label describing the status of the result, suitable for use in
/// metrics.
fn status_label<T, E>(result: &RusotoResult<T, E>) -> &str {
    match result {
        Ok(_) => "success",
        Err(RusotoError::HttpDispatch(_)) => "http_dispatch",
        Err(RusotoError::Credentials(_)) => "credentials",
        Err(RusotoError::Unknown(response)) => response.status.as_str(),
        Err(_) => "unknown",
    }
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

/// Errors encountered when creating a sts:GetCallerIdentity token.
#[derive(Debug, thiserror::Error)]
pub enum StsError {
    #[error("no security token in AWS credentials")]
    MissingAccessToken,
    #[error("no host in provided URL {0}")]
    MissingHost(Url),
    #[error("no query params in request URL")]
    MissingQuery,
    #[error("failed to parse host as header value: {0}")]
    BadHeaderHost(InvalidHeaderValue),
    #[error("failed to parse formatted date as header value: {0}")]
    BadHeaderDate(InvalidHeaderValue),
    #[error("failed to parse Amazon security token as header value: {0}")]
    BadHeaderToken(InvalidHeaderValue),
    #[error("failed to parse workload identity pool provider as header value: {0}")]
    BadHeaderTarget(InvalidHeaderValue),
    #[error(transparent)]
    ParseRegion(#[from] ParseRegionError),
    #[error("failed to construct AWS request signing key: {0}")]
    KeyConstruction(InvalidLength),
    #[error("failed to construct HMAC from signing key: {0}")]
    HmacConstruction(InvalidLength),
}

/// Construct a GetCallerIdentity token, as specified in GCP's workload identity
/// pool guide. This entails constructing a signed sts:GetCallerIdentity AWS API
/// request, using the form that has no body (sts.googleapis.com will reject the
/// other form), then signing that request using AWS v4 request signing, then
/// constructing a JSON serialization of the GetCallerIdentity request, which
/// will later be reconstructed by sts.googleapis.com into an HTTP POST and sent
/// to sts.amazonaws.com. This means that the request we construct gets
/// scrutinized by not one but two different STS implementations, so we must be
/// unusually careful.
///
/// # Arguments
///
/// * `sts_request_url` - The full URL to which the serialized request should be
///   sent. Regional API endpoints are supported.
/// * `workload_identity_pool_provider` - The full resource name of the GCP
///   workload identity provider, like
///   "//iam.googleapis.com/projects/PROJECT_NUMBER/locations/global/workloadIdentityPools/POOL_ID/providers/PROVIDER_ID".
/// * `aws_region` - The AWS region
/// * `credentials` - AWS credentials to use to sign the request.
///
/// References:
/// GCP workload identity pool / sts.googleapis.com:
/// https://cloud.google.com/iam/docs/access-resources-aws#exchange-token
/// https://cloud.google.com/iam/docs/reference/sts/rest/v1/TopLevel/token
/// AWS sts:GetCallerIdentity:
/// https://docs.aws.amazon.com/STS/latest/APIReference/API_GetCallerIdentity.html
/// AWS v4 request signing
/// https://docs.aws.amazon.com/general/latest/gr/sigv4_signing.html
pub(crate) fn get_caller_identity_token(
    sts_request_url: &Url,
    workload_identity_pool_provider: &str,
    aws_region: &Region,
    credentials: &AwsCredentials,
) -> Result<serde_json::Value, StsError> {
    get_caller_identity_token_at_time(
        Utc::now(),
        sts_request_url,
        workload_identity_pool_provider,
        aws_region,
        credentials,
    )
}

fn get_caller_identity_token_at_time(
    request_time: DateTime<Utc>,
    sts_request_url: &Url,
    workload_identity_pool_provider: &str,
    aws_region: &Region,
    credentials: &AwsCredentials,
) -> Result<serde_json::Value, StsError> {
    // Rusoto provides a SignedRequest struct that can construct and sign
    // correct sts:GetCallerIdentity requests, but unfortunately it insists on
    // using the form with a body, which sts.googleapis.com rejects. Instead we
    // use a specialized implementation of AWS v4 request signing, vendored from
    // the rust-s3 crate (https://crates.io/crates/rust-s3). See also Amazon
    // documentation on the request signing format:
    // https://docs.aws.amazon.com/general/latest/gr/sigv4_signing.html
    let aws_security_token = credentials
        .token()
        .as_ref()
        .ok_or(StsError::MissingAccessToken)?;
    let host = sts_request_url
        .host_str()
        .ok_or_else(|| StsError::MissingHost(sts_request_url.clone()))?;
    let request_time_long = request_time.format(LONG_DATETIME).to_string();

    // Construct the headers that we will sign over. Arbitrary headers may be
    // signed over in an AWS signed request, but adding headers here may
    // cause sts.googleapis.com to reject the GetCallerIdentity token.
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static(HOST_HEADER),
        host.parse().map_err(StsError::BadHeaderHost)?,
    );
    headers.insert(
        HeaderName::from_static(AMAZON_DATE_HEADER),
        request_time
            .format(LONG_DATETIME)
            .to_string()
            .parse()
            .map_err(StsError::BadHeaderDate)?,
    );
    headers.insert(
        HeaderName::from_static(AMAZON_SECURITY_TOKEN_HEADER),
        aws_security_token
            .parse()
            .map_err(StsError::BadHeaderToken)?,
    );
    headers.insert(
        HeaderName::from_static(GOOGLE_TARGET_RESOURCE_HEADER),
        workload_identity_pool_provider
            .parse()
            .map_err(StsError::BadHeaderTarget)?,
    );

    // Create a canonical signature v4 request.
    // The last part of the canonical request is the SHA256 digest of the
    // request body. In our case the body is empty, so we provide the hash of
    // the empty string, per step 6 in the Amazon documentation.
    // https://docs.aws.amazon.com/general/latest/gr/sigv4-create-canonical-request.html
    let canonical_request = format!(
        "POST\n{uri}\n{query_string}\n{headers}\n\n{signed}\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        uri = sts_request_url.path(),
        query_string = sts_request_url.query().ok_or(StsError::MissingQuery)?,
        headers = canonical_header_string(&headers),
        signed = signed_header_string(&headers),
    );

    let mut hasher = Sha256::default();
    hasher.update(canonical_request.as_bytes());

    // Construct the string to be signed
    // https://docs.aws.amazon.com/general/latest/gr/sigv4-create-string-to-sign.html
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{timestamp}\n{date}/{region}/sts/aws4_request\n{hash}",
        timestamp = request_time_long,
        date = request_time.format(SHORT_DATE),
        region = aws_region.name(),
        hash = hex::encode(hasher.finalize().as_slice()),
    );

    // Derive the "signing" (actually HMAC) key scoped to the current date, the
    // service and the regional endpoint
    // https://docs.aws.amazon.com/general/latest/gr/sigv4-calculate-signature.html
    let signing_key = signing_key(
        &request_time,
        credentials.aws_secret_access_key(),
        &aws_region.name().parse()?,
        "sts",
    )
    .map_err(StsError::KeyConstruction)?;

    // "Sign" (HMAC) the string to sign
    let mut hmac: Hmac<Sha256> =
        Hmac::new_from_slice(&signing_key).map_err(StsError::HmacConstruction)?;
    hmac.update(string_to_sign.as_bytes());
    let signature = hex::encode(hmac.finalize().into_bytes());

    // Construct the HTTP authorization header to sendin the request
    // https://docs.aws.amazon.com/general/latest/gr/sigv4-add-signature-to-request.html
    let authorization_header = format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{date}/{region}/sts/aws4_request,\
                SignedHeaders={signed_headers},Signature={signature}",
        access_key = credentials.aws_access_key_id(),
        date = request_time.format("%Y%m%d"),
        region = aws_region.name(),
        signed_headers = &signed_header_string(&headers),
        signature = signature
    );

    // Construct the GetCallerIdentity token expected by sts.googleapis.com.
    // Except for the Authorization header, these must match the headers signed
    // over earlier.
    Ok(ureq::json!({
        "url": sts_request_url.as_str(),
        "method": "POST",
        "headers": [
            {
                "key": "Authorization",
                "value": authorization_header,
            },
            {
                "key": HOST_HEADER,
                "value": host,
            },
            {
                "key": AMAZON_DATE_HEADER,
                "value": request_time_long,
            },
            {
                "key": AMAZON_SECURITY_TOKEN_HEADER,
                "value": aws_security_token,
            },
            {
                "key": GOOGLE_TARGET_RESOURCE_HEADER,
                "value": workload_identity_pool_provider,
            },
        ]
    }))
}

/// Generate a canonical header string from the provided headers.
pub fn canonical_header_string(headers: &HeaderMap) -> String {
    let mut keyvalues = headers
        .iter()
        .map(|(key, value)| {
            // Values that are not strings are silently dropped (AWS wouldn't
            // accept them anyway)
            key.as_str().to_lowercase() + ":" + value.to_str().unwrap().trim()
        })
        .collect::<Vec<String>>();
    keyvalues.sort();
    keyvalues.join("\n")
}

/// Generate a signed header string from the provided headers.
pub fn signed_header_string(headers: &HeaderMap) -> String {
    let mut keys = headers
        .keys()
        .map(|key| key.as_str().to_lowercase())
        .collect::<Vec<String>>();
    keys.sort();
    keys.join(";")
}

/// Generate the AWS signing key, derived from the secret key, date, region,
/// and service name.
pub fn signing_key(
    datetime: &DateTime<Utc>,
    secret_key: &str,
    region: &Region,
    service: &str,
) -> Result<Vec<u8>, InvalidLength> {
    let secret = format!("AWS4{}", secret_key);
    let mut date_hmac: Hmac<Sha256> = Hmac::new_from_slice(secret.as_bytes())?;
    date_hmac.update(datetime.format(SHORT_DATE).to_string().as_bytes());
    let mut region_hmac: Hmac<Sha256> = Hmac::new_from_slice(&date_hmac.finalize().into_bytes())?;
    region_hmac.update(region.name().as_bytes());
    let mut service_hmac: Hmac<Sha256> =
        Hmac::new_from_slice(&region_hmac.finalize().into_bytes())?;
    service_hmac.update(service.as_bytes());
    let mut signing_hmac: Hmac<Sha256> =
        Hmac::new_from_slice(&service_hmac.finalize().into_bytes())?;
    signing_hmac.update(b"aws4_request");
    Ok(signing_hmac.finalize().into_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use chrono::TimeZone;
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

    #[test]
    fn get_caller_identity_token_correctness() {
        // Valid request URL
        let sts_request_url = Url::parse(
            "https://sts.us-east-1.amazonaws.com/?Action=GetCallerIdentity&Version=2011-06-15",
        )
        .unwrap();

        // Host-less but valid URL from Url docs
        let bad_request_url = Url::parse("data:text/plain,Stuff").unwrap();

        // AWS credentials invalid due to missing token
        let aws_creds_no_token = AwsCredentials::new("fake-key", "fake-secret", None, None);

        // Valid AWS credentials
        let aws_creds = AwsCredentials::new(
            "fake-key",
            "fake-secret",
            Some("fake-token".to_string()),
            None,
        );

        // AWS creds without a token should cause non-specific error
        get_caller_identity_token(
            &sts_request_url,
            "fake-workload-identity-pool-provider",
            &Region::ApEast1,
            &aws_creds_no_token,
        )
        .unwrap_err();

        // Request URL without host should cause non-specific error
        get_caller_identity_token(
            &bad_request_url,
            "fake-workload-identity-pool-provider",
            &Region::ApEast1,
            &aws_creds,
        )
        .unwrap_err();

        // To avoid providing an ad-hoc implementation of sts.googleapis.com, we check the output
        // against a recorded, strongly-believed-to-be-good token so we can at least prevent trivial
        // regressions.
        let token = get_caller_identity_token_at_time(
            Utc.with_ymd_and_hms(2021, 1, 1, 1, 1, 1).unwrap(),
            &sts_request_url,
            "fake-workload-identity-pool-provider",
            &Region::ApEast1,
            &aws_creds,
        )
        .unwrap();

        let expected_token = serde_json::json!({
            "url": "https://sts.us-east-1.amazonaws.com/?Action=GetCallerIdentity&Version=2011-06-15",
            "method": "POST",
            "headers": [
                {
                    "key": "Authorization",
                    "value": "AWS4-HMAC-SHA256 Credential=fake-key/20210101/ap-east-1/sts/aws4_request,SignedHeaders=host;x-amz-date;x-amz-security-token;x-goog-cloud-target-resource,Signature=e5e352ea8b93552bd3a11e01ff83f6fa6cdc4dfa19cc57be9d54c9d746918dc0"
                },
                {
                    "key": "host",
                    "value": "sts.us-east-1.amazonaws.com"
                },
                {
                    "key": "x-amz-date",
                    "value": "20210101T010101Z"
                },
                {
                    "key": "x-amz-security-token",
                    "value": "fake-token"
                },
                {
                    "key": "x-goog-cloud-target-resource",
                    "value": "fake-workload-identity-pool-provider"
                }
            ]
        });

        assert_eq!(token, expected_token);
    }
}

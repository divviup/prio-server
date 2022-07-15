//! API clients for obtaining authentication tokens for use with Google Cloud
//! Platform
//!
//! This module provides clients that support a variety of authentication flows
//! to GCP, whether for Google Kubernetes Engine workloads, clients using a GCP
//! service account key file or identity federation with sts.amazonaws.com.
//!
//! Throughout this module we make reference to _access_ or _identity_ tokens.
//! Both are Oauth tokens but are not the same. [Access tokens] represent "the
//! authorization of a specific application to access specific parts of a user’s
//! data" and are used to authenticate to GCP services while [identity or ID
//! tokens] encode "the user’s authentication information" and are used for
//! federation with external identity services (i.e. `sts.amazonaws.com`).
//!
//! [Access tokens]: https://www.oauth.com/oauth2-servers/access-tokens/
//! [identity of ID tokens]: https://www.oauth.com/oauth2-servers/openid-connect/id-tokens/

use anyhow::{anyhow, Context, Result};
use chrono::{prelude::Utc, DateTime, Duration};
use dyn_clone::DynClone;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use rusoto_core::{credential::ProvideAwsCredentials, Region};
use serde::{Deserialize, Serialize, Serializer};
use slog::{debug, o, Logger};
use std::{
    collections::HashMap,
    convert::TryFrom,
    fmt::{self, Debug, Display, Formatter},
    io::Read,
    str,
    sync::{Arc, RwLock},
    time::Duration as StdDuration,
};
use tokio::runtime::Handle;
use ureq::{AgentBuilder, Response};
use url::Url;

use crate::{
    aws_credentials::{self, get_caller_identity_token, StsError},
    config::{Identity, WorkloadIdentityPoolParameters},
    http::{
        AccessTokenProvider, Method, RequestParameters, RetryingAgent, StaticAccessTokenProvider,
    },
    metrics::ApiClientMetricsCollector,
    parse_url, UrlParseError,
};

const DEFAULT_METADATA_BASE_URL: &str = "http://metadata.google.internal:80";
const DEFAULT_ACCESS_TOKEN_PATH: &str =
    "/computeMetadata/v1/instance/service-accounts/default/token";
const DEFAULT_IDENTITY_TOKEN_PATH: &str =
    "/computeMetadata/v1/instance/service-accounts/default/identity";
const DEFAULT_IAM_BASE_URL: &str = "https://iamcredentials.googleapis.com";
// Both the GKE metadata service and iamcredentials.googleapis.com require an
// audience parameter when requesting identity tokens, so we must provide
// *something*. However, the value we provide can be anything so long as it
// matches the role assumption policy configured on the AWS IAM role we are
// trying to assume. That policy will be scoped to the numeric account ID of a
// specific GCP service account anyway, so there's not much to be gained by
// further scoping the identiy token to an audience
const IDENTITY_TOKEN_AUDIENCE: &str = "sts.amazonaws.com/gke-identity-federation";

/// Represents the claims encoded into JWTs when using a service account key
/// file to authenticate as the default GCP service account.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Claims {
    iss: String,
    scope: String,
    aud: String,
    iat: i64,
    exp: i64,
}

/// A wrapper around an access token and its expiration date.
#[derive(Clone, Debug)]
struct AccessToken {
    token: String,
    expiration: DateTime<Utc>,
}

impl AccessToken {
    /// Returns true if the token is expired.
    fn expired(&self) -> bool {
        Utc::now() >= self.expiration
    }
}

/// Represents the response from a GET request to the GKE metadata service's
/// service account token endpoint, or to oauth2.googleapis.com.token[1], or to
/// sts.googleapis.com's token method[2]
/// [1] https://developers.google.com/identity/protocols/oauth2/service-account
/// [2] https://cloud.google.com/iam/docs/reference/sts/rest/v1/TopLevel/token
#[derive(Clone, Debug, Deserialize, PartialEq)]
struct AccessTokenResponse {
    access_token: String,
    expires_in: i64,
    token_type: String,
}

/// Represents the response from a POST request to the GCP IAM service's
/// generateAccessToken endpoint.
/// https://cloud.google.com/iam/docs/reference/credentials/rest/v1/projects.serviceAccounts/generateAccessToken
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct GenerateAccessTokenResponse {
    access_token: String,
    expire_time: DateTime<Utc>,
}

/// This is the subset of a GCP service account key file that we need to parse
/// to construct a signed JWT.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq)]
struct ServiceAccountKeyFile {
    /// The PEM-armored base64 encoding of the ASN.1 encoding of the account's
    /// RSA private key.
    private_key: String,
    /// The private key ID.
    private_key_id: String,
    /// The service account's email address.
    client_email: String,
    /// The URL from which OAuth tokens should be requested.
    token_uri: String,
}

impl TryFrom<Box<dyn Read>> for ServiceAccountKeyFile {
    type Error = crate::Error;
    fn try_from(reader: Box<dyn Read>) -> Result<Self, Self::Error> {
        serde_json::from_reader(reader).map_err(crate::Error::BadKeyFile)
    }
}

/// OAuth access scopes used when obtaining access tokens. This enum only
/// represents the access scopes we currently use.
/// https://cloud.google.com/compute/docs/access/service-accounts#accesscopesiam
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) enum AccessScope {
    /// IAM credentials
    /// https://developers.google.com/identity/protocols/oauth2/scopes#iam
    CloudPlatform,
    /// Read and write cloud storage
    /// https://developers.google.com/identity/protocols/oauth2/scopes#storage
    DevStorageReadWrite,
    /// Read and write PubSub topics and subscriptions
    /// https://developers.google.com/identity/protocols/oauth2/scopes#pubsub
    PubSub,
}

impl AccessScope {
    fn as_str(&self) -> &'static str {
        match self {
            Self::CloudPlatform => "https://www.googleapis.com/auth/cloud-platform",
            Self::DevStorageReadWrite => "https://www.googleapis.com/auth/devstorage.read_write",
            Self::PubSub => "https://www.googleapis.com/auth/pubsub",
        }
    }
}

impl Display for AccessScope {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Serialize for AccessScope {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// Errors related to GCP cloud service authentication.
#[derive(Debug, thiserror::Error)]
pub enum GcpAuthError {
    #[error("failed to parse PEM RSA key: {0}")]
    PemRsaKeyParse(jsonwebtoken::errors::Error),
    #[error("failed to construct and sign JWT: {0}")]
    JwtSign(jsonwebtoken::errors::Error),
    #[error("failed to obtain default account access token from GKE metadata service: {0}")]
    MetadataRequest(ureq::Error),
    #[error("failed to obtain access token with service account key file: {0}")]
    TokenRequest(ureq::Error),
    #[error("failed to deserialize response from GCP token service: {0}")]
    TokenResponseDeserialization(std::io::Error),
    #[error("failed to get access token to impersonate service account {1}: {0}")]
    ImpersonationRequest(ureq::Error, String),
    #[error("failed to deserialize response from IAM API: {0}")]
    ImpersonationResponseDeserialization(std::io::Error),
    #[error("failed to obtain federated access token from sts.googleapis.com: {0}")]
    FederationRequest(ureq::Error),
    #[error("unexpected token type {0}")]
    UnexpectedTokenType(String),
    #[error(transparent)]
    Url(#[from] UrlParseError),
    #[error("failed to get AWS credentials: {0}")]
    Aws(rusoto_core::credential::CredentialsError),
    #[error(transparent)]
    Sts(#[from] StsError),
}

/// Implementations of ProvideDefaultAccessToken obtain a default access token,
/// used either to authenticate to GCP services or to obtain a further service
/// account access token from GCP IAM.
trait ProvideDefaultAccessToken: Debug + DynClone + Send + Sync {
    fn default_access_token(&self) -> Result<Response, GcpAuthError>;
}

dyn_clone::clone_trait_object!(ProvideDefaultAccessToken);

/// Fetches an access token from the GKE metadata service for the GCP SA mapped
/// to the current Kubernetes SA via GKE workload identity (not to be confused
/// with workload identity *pool*). Used by workloads in Google Kubernetes
/// Engine.
#[derive(Clone, Debug)]
struct GkeMetadataServiceDefaultAccessTokenProvider {
    agent: RetryingAgent,
    logger: Logger,
    /// Base URL at which to access GKE metadata service
    metadata_service_base_url: &'static str,
}

impl GkeMetadataServiceDefaultAccessTokenProvider {
    fn new(agent: RetryingAgent, logger: Logger) -> Self {
        Self {
            agent,
            logger,
            metadata_service_base_url: DEFAULT_METADATA_BASE_URL,
        }
    }

    /// Returns the GKE metadata service URL from which access tokens for the
    /// default GCP service account may be obtained.
    fn default_access_token_url(base: &str) -> Url {
        let mut request_url =
            Url::parse(base).expect("unable to parse metadata.google.internal url");
        request_url.set_path(DEFAULT_ACCESS_TOKEN_PATH);
        request_url
    }
}

impl ProvideDefaultAccessToken for GkeMetadataServiceDefaultAccessTokenProvider {
    fn default_access_token(&self) -> Result<Response, GcpAuthError> {
        debug!(
            self.logger,
            "obtaining default account access token from GKE metadata service"
        );

        let mut request = self.agent.prepare_anonymous_request(
            Self::default_access_token_url(self.metadata_service_base_url),
            Method::Get,
        );

        request = request.set("Metadata-Flavor", "Google");

        self.agent
            .call(&self.logger, &request, DEFAULT_ACCESS_TOKEN_PATH)
            .map_err(GcpAuthError::MetadataRequest)
    }
}

/// Uses a GCP service account key file to authenticate to GCP IAM as some
/// service account.
#[derive(Clone, Debug)]
struct ServiceAccountKeyFileDefaultAccessTokenProvider {
    key_file: ServiceAccountKeyFile,
    scope: AccessScope,
    agent: RetryingAgent,
    logger: Logger,
}

impl ProvideDefaultAccessToken for ServiceAccountKeyFileDefaultAccessTokenProvider {
    fn default_access_token(&self) -> Result<Response, GcpAuthError> {
        debug!(
            self.logger,
            "obtaining access token with service account key file"
        );
        // We construct the JWT per Google documentation:
        // https://developers.google.com/identity/protocols/oauth2/service-account#authorizingrequests
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(self.key_file.private_key_id.to_owned());

        // The iat and exp fields in a JWT are in seconds since UNIX epoch.
        let now = Utc::now().timestamp();
        let claims = Claims {
            iss: self.key_file.client_email.to_owned(),
            scope: self.scope.to_string(),
            aud: self.key_file.token_uri.to_owned(),
            iat: now,
            exp: now + 3600, // token expires in one hour
        };

        let encoding_key = EncodingKey::from_rsa_pem(self.key_file.private_key.as_bytes())
            .map_err(GcpAuthError::PemRsaKeyParse)?;

        let token = encode(&header, &claims, &encoding_key).map_err(GcpAuthError::JwtSign)?;

        let request = self
            .agent
            .prepare_anonymous_request(parse_url(self.key_file.token_uri.clone())?, Method::Post);

        self.agent
            .send_form(
                &self.logger,
                &request,
                "token-for-key-file",
                &[
                    ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                    ("assertion", &token),
                ],
            )
            .map_err(GcpAuthError::TokenRequest)
    }
}

/// Uses a GCP Workload Identity Pool to federate GCP IAM with AWS IAM, allowing
/// workloads in AWS Elastic Kubernetes Service to impersonate a GCP SA.
#[derive(Clone)]
struct AwsIamFederationViaWorkloadIdentityPoolDefaultAccessTokenProvider {
    aws_credentials_provider: aws_credentials::Provider,
    workload_identity_pool_provider: String,
    runtime_handle: Handle,
    logger: Logger,
    agent: RetryingAgent,
}

impl Debug for AwsIamFederationViaWorkloadIdentityPoolDefaultAccessTokenProvider {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("AwsIamFederationViaWorkloadIdentityPoolDefaultAccessTokenProvider")
            .field(
                "token_source",
                &"AWS IAM federation via workload identity pool",
            )
            .finish()
    }
}

impl ProvideDefaultAccessToken
    for AwsIamFederationViaWorkloadIdentityPoolDefaultAccessTokenProvider
{
    fn default_access_token(&self) -> Result<Response, GcpAuthError> {
        debug!(
            self.logger,
            "getting GCP workload identity pool federated token"
        );
        // rusoto_core::Region::default consults AWS_DEFAULT_REGION and
        // AWS_REGION environment variables to figure out what region we are
        // running in, which we need to pick an STS endpoint
        let aws_region = Region::default();

        // First, we must obtain credentials for the AWS IAM role mapped to this
        // facilitator's Kubernetes service account
        let credentials = self
            .runtime_handle
            .block_on(self.aws_credentials_provider.credentials())
            .map_err(GcpAuthError::Aws)?;

        // Next, we construct a GetCallerIdentity token
        let sts_request_url = parse_url(format!(
            "https://sts.{}.amazonaws.com?Action=GetCallerIdentity&Version=2011-06-15",
            aws_region.name()
        ))?;

        let get_caller_identity_token = get_caller_identity_token(
            &sts_request_url,
            &self.workload_identity_pool_provider,
            &aws_region,
            &credentials,
        )?;

        // The GetCallerIdentity token goes into the body of the request sent to
        // sts.googleapis.com (one assumes that GCP relays that request to
        // sts.amazonaws.com to prove that we control the AWS IAM role).
        let request_body = ureq::json!({
            "audience": &self.workload_identity_pool_provider,
            "grantType": "urn:ietf:params:oauth:grant-type:token-exchange",
            "requestedTokenType": "urn:ietf:params:oauth:token-type:access_token",
            "scope": AccessScope::CloudPlatform,
            "subjectToken": urlencoding::encode(&get_caller_identity_token.to_string()),
            "subjectTokenType": "urn:ietf:params:aws:token-type:aws4_request",
        });

        let endpoint = "token";

        let request = self
            .agent
            .prepare_request(RequestParameters {
                url: parse_url(format!("https://sts.googleapis.com/v1/{}", endpoint))?,
                method: Method::Post,
                // This request is unauthenticated, except for the signature and
                // token on the inner subjectToken
                token_provider: None,
            })?
            .set("Content-Type", "application/json; charset=utf-8");

        self.agent
            .send_json_request(&self.logger, &request, endpoint, &request_body)
            .map_err(GcpAuthError::FederationRequest)
    }
}

/// GcpAccessTokenProvider manages a default service account access token (e.g.,
/// the one for a GCP service account mapped to a Kubernetes service account, or
/// the one found in a JSON key file) and an access token used to impersonate
/// another service account.
///
/// A note on thread safety: this struct stores any Oauth tokens it obtains in
/// an Arc+Mutex, so an instance of GcpOauthTokenProvider may be .clone()d
/// liberally and shared across threads, and credentials obtained from the GCP
/// credentials API will be shared efficiently and safely.
#[derive(Clone)]
pub(crate) struct GcpAccessTokenProvider {
    /// The Oauth scope for which tokens should be requested.
    scope: AccessScope,
    /// Provides the default access token, which may be used to directly access
    /// GCP services or may be used to impersonate some GCP service account.
    default_token_provider: Box<dyn ProvideDefaultAccessToken>,
    /// Holds the service account email to impersonate, if one was provided to
    /// GcpAccessTokenProvider::new.
    account_to_impersonate: Identity,
    /// This field is None after instantiation and is Some after the first
    /// successful request for a token for the default service account, though
    /// the contained token may be expired.
    default_access_token: Arc<RwLock<Option<AccessToken>>>,
    /// This field is None after instantiation and is Some after the first
    /// successful request for a token for the impersonated service account,
    /// though the contained token may be expired. This will always be None if
    /// account_to_impersonate is None.
    impersonated_account_token: Arc<RwLock<Option<AccessToken>>>,
    /// The agent will be used when making HTTP requests to GCP APIs to fetch
    /// Oauth tokens.
    agent: RetryingAgent,
    /// Logger to which messages will be logged
    logger: Logger,
    /// Base URL at which to access GCP IAM service
    iam_service_base_url: &'static str,
}

impl Debug for GcpAccessTokenProvider {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("GcpAccessTokenProvider")
            .field("account_to_impersonate", &self.account_to_impersonate)
            .field("default_token_provider", &self.default_token_provider)
            .field(
                "default_access_token",
                &self
                    .default_access_token
                    .read()
                    .unwrap()
                    .as_ref()
                    .map(|_| "redacted"),
            )
            .field(
                "impersonated_account_token",
                &self
                    .impersonated_account_token
                    .read()
                    .unwrap()
                    .as_ref()
                    .map(|_| "redacted"),
            )
            .finish()
    }
}

impl AccessTokenProvider for GcpAccessTokenProvider {
    /// Returns the Oauth token to use with GCP API in an Authorization header,
    /// fetching it or renewing it if necessary. If a service account to
    /// impersonate was provided, the default service account is used to
    /// authenticate to the GCP IAM API to retrieve an Oauth token. If no
    /// impersonation is taking place, provides the default service account
    /// Oauth token.
    fn ensure_access_token(&self) -> Result<String, GcpAuthError> {
        if let Some(account_to_impersonate) = self.account_to_impersonate.as_str() {
            self.ensure_impersonated_service_account_access_token(account_to_impersonate)
        } else {
            self.ensure_default_access_token()
        }
    }
}

impl GcpAccessTokenProvider {
    /// Creates a token provider which can impersonate the specified service
    /// account.
    fn new(
        scope: AccessScope,
        account_to_impersonate: Identity,
        key_file: Option<ServiceAccountKeyFile>,
        workload_identity_pool_params: Option<WorkloadIdentityPoolParameters>,
        runtime_handle: &Handle,
        parent_logger: &Logger,
        api_metrics: &ApiClientMetricsCollector,
    ) -> Result<Self> {
        let logger = parent_logger.new(o!(
            "scope" => scope.to_string(),
            "account_to_impersonate" => account_to_impersonate.to_string(),
        ));

        let default_token_provider: Box<dyn ProvideDefaultAccessToken> =
            match (key_file, workload_identity_pool_params) {
                (Some(_), Some(_)) => {
                    return Err(anyhow!(
                        "either but not both of key_file_reader or aws_credentials may be provided"
                    ))
                }
                (Some(key_file), None) => {
                    let agent = RetryingAgent::new("iamcredentials.googleapis.com", api_metrics);
                    let provider = ServiceAccountKeyFileDefaultAccessTokenProvider {
                        key_file,
                        scope: scope.clone(),
                        agent,
                        logger: logger.clone(),
                    };
                    Box::new(provider)
                }
                (None, Some(parameters)) => {
                    let agent = RetryingAgent::new("sts.googleapis.com", api_metrics);
                    let provider =
                        AwsIamFederationViaWorkloadIdentityPoolDefaultAccessTokenProvider {
                            aws_credentials_provider: parameters.aws_credentials_provider,
                            workload_identity_pool_provider: parameters
                                .workload_identity_pool_provider,
                            logger: logger.clone(),
                            agent,
                            runtime_handle: runtime_handle.clone(),
                        };
                    Box::new(provider)
                }
                (None, None) => {
                    let agent = AgentBuilder::new()
                        .timeout(StdDuration::from_secs(10))
                        .build();
                    let agent = RetryingAgent::new_with_agent(
                        agent,
                        // We have seen transient 404 errors from this endpoint previously.
                        vec![404],
                        "metadata.google.internal",
                        api_metrics,
                    );
                    let provider =
                        GkeMetadataServiceDefaultAccessTokenProvider::new(agent, logger.clone());
                    Box::new(provider)
                }
            };

        Ok(Self {
            scope,
            default_token_provider,
            account_to_impersonate,
            default_access_token: Arc::new(RwLock::new(None)),
            impersonated_account_token: Arc::new(RwLock::new(None)),
            agent: RetryingAgent::new("iamcredentials.googleapis.com", api_metrics),
            logger,
            iam_service_base_url: DEFAULT_IAM_BASE_URL,
        })
    }

    /// Returns the URL from which access tokens for the provided GCP service
    /// account may be obtained.
    /// API reference: https://cloud.google.com/iam/docs/reference/credentials/rest/v1/projects.serviceAccounts/generateAccessToken
    fn access_token_url_for_service_account(
        base: &str,
        service_account_to_impersonate: &str,
    ) -> Result<Url, UrlParseError> {
        let request_url = format!(
            "{}{}",
            base,
            Self::access_token_path_for_service_account(service_account_to_impersonate)
        );

        parse_url(request_url)
    }

    /// Returns the path relative to an API base URL from which access tokens for
    /// the provided GCP service account may be obtained
    fn access_token_path_for_service_account(service_account_to_impersonate: &str) -> String {
        format!(
            "/v1/projects/-/serviceAccounts/{}:generateAccessToken",
            service_account_to_impersonate
        )
    }

    /// Returns the current access token for the default service account, if it
    /// is valid. Otherwise obtains and returns a new one.
    /// The returned value is an owned reference because the token owned by this
    /// struct could change while the caller is still holding the returned token
    fn ensure_default_access_token(&self) -> Result<String, GcpAuthError> {
        debug!(self.logger, "obtaining read lock on default access token");
        if let Some(token) = &*self.default_access_token.read().unwrap() {
            debug!(self.logger, "obtained read lock on default access token");
            if !token.expired() {
                debug!(self.logger, "cached default access token is still valid");
                return Ok(token.token.clone());
            }
        }

        debug!(self.logger, "obtaining write lock on default access token");
        let mut default_access_token = self.default_access_token.write().unwrap();
        debug!(self.logger, "obtained write lock on default access token");
        // Check if the token was updated between when we dropped the read lock
        // and when we acquired the write lock
        if let Some(token) = &*default_access_token {
            if !token.expired() {
                debug!(self.logger, "cached default access token is still valid");
                return Ok(token.token.clone());
            }
        }

        let http_response = self.default_token_provider.default_access_token()?;

        let response = http_response
            .into_json::<AccessTokenResponse>()
            .map_err(GcpAuthError::TokenResponseDeserialization)?;

        if response.token_type != "Bearer" {
            return Err(GcpAuthError::UnexpectedTokenType(response.token_type));
        }

        *default_access_token = Some(AccessToken {
            token: response.access_token.clone(),
            expiration: Utc::now() + Duration::seconds(response.expires_in),
        });

        Ok(response.access_token)
    }

    /// Returns the current access token for the impersonated service account,
    /// if it is valid. Otherwise obtains and returns a new one.
    fn ensure_impersonated_service_account_access_token(
        &self,
        service_account_to_impersonate: &str,
    ) -> Result<String, GcpAuthError> {
        debug!(
            self.logger,
            "obtaining read lock on impersonated account token"
        );
        if let Some(token) = &*self.impersonated_account_token.read().unwrap() {
            debug!(
                self.logger,
                "obtained read lock on impersonated account token"
            );
            if !token.expired() {
                debug!(
                    self.logger,
                    "cached token is still valid for impersonating service account"
                );
                return Ok(token.token.clone());
            }
        }

        let default_token = self.ensure_default_access_token()?;
        debug!(
            self.logger,
            "obtaining write lock on impersonated account token"
        );
        let mut impersonated_account_token = self.impersonated_account_token.write().unwrap();
        debug!(
            self.logger,
            "obtained write lock on impersonated account token"
        );

        let request = self.agent.prepare_request(RequestParameters {
            url: Self::access_token_url_for_service_account(
                self.iam_service_base_url,
                service_account_to_impersonate,
            )?,
            method: Method::Post,
            token_provider: Some(&StaticAccessTokenProvider::from(default_token)),
        })?;

        debug!(
            self.logger,
            "obtaining token to impersonate service account"
        );

        let http_response = self
            .agent
            .send_json_request(
                &self.logger,
                &request,
                "generateAccessToken",
                &ureq::json!({
                    "scope": [self.scope]
                }),
            )
            .map_err(|e| {
                GcpAuthError::ImpersonationRequest(e, service_account_to_impersonate.to_owned())
            })?;

        let response = http_response
            .into_json::<GenerateAccessTokenResponse>()
            .map_err(GcpAuthError::ImpersonationResponseDeserialization)?;

        *impersonated_account_token = Some(AccessToken {
            token: response.access_token.clone(),
            expiration: response.expire_time,
        });

        Ok(response.access_token)
    }
}

/// Implementations of ProvideGcpIdentityToken obtain OpenID Connect identity
/// tokens from some GCP server
pub(crate) trait ProvideGcpIdentityToken: Debug + Send + Sync {
    fn identity_token(&self) -> Result<String>;
}

/// Obtains identity tokens from the GKE metadata service for the workload's
/// default service account. Used by workloads in GKE with Workload Identity
/// configured.
#[derive(Clone, Debug)]
pub(crate) struct GkeMetadataServiceIdentityTokenProvider {
    /// Base URL at which to access GKE metadata service
    metadata_service_base_url: &'static str,
    agent: RetryingAgent,
    logger: Logger,
}

impl GkeMetadataServiceIdentityTokenProvider {
    pub(crate) fn new(logger: Logger, api_metrics: &ApiClientMetricsCollector) -> Self {
        Self {
            metadata_service_base_url: DEFAULT_METADATA_BASE_URL,
            agent: RetryingAgent::new("metadata.google.internal", api_metrics),
            logger: logger.clone(),
        }
    }

    /// Returns the GKE metadata service URL from which identity tokens for the
    /// default GCP service account may be obtained.
    fn default_identity_token_url(base: &str) -> Url {
        let mut request_url =
            Url::parse(base).expect("unable to parse metadata.google.internal url");
        request_url.set_path(DEFAULT_IDENTITY_TOKEN_PATH);
        request_url
    }
}

impl ProvideGcpIdentityToken for GkeMetadataServiceIdentityTokenProvider {
    fn identity_token(&self) -> Result<String> {
        debug!(
            self.logger,
            "obtaining OIDC token from GKE metadata service"
        );

        let mut url = Self::default_identity_token_url(self.metadata_service_base_url);

        url.query_pairs_mut()
            .append_pair("audience", IDENTITY_TOKEN_AUDIENCE)
            .finish();

        let request = self
            .agent
            .prepare_anonymous_request(url, Method::Get)
            .set("Metadata-Flavor", "Google");

        self.agent
            .call(&self.logger, &request, DEFAULT_IDENTITY_TOKEN_PATH)
            .context("failed to fetch identity token from metadata service")?
            .into_string()
            .context("failed to get response body from metadata service response")
    }
}

/// The response from iamcredentials.googleapis.com's generateIdToken endpoint.
/// API reference: https://cloud.google.com/iam/docs/reference/credentials/rest/v1/projects.serviceAccounts/generateIdToken#response-body
#[derive(Clone, Debug, Deserialize, PartialEq)]
struct GenerateIdTokenResponse {
    token: String,
}

/// Obtains identity tokens from iamcredentials.googleapis.com for the specified
/// GCP service account.
#[derive(Clone, Debug)]
pub(crate) struct ImpersonatedServiceAccountIdentityTokenProvider {
    /// Email address of the GCP service account for which identity tokens are
    /// to be obtained.
    impersonated_service_account: String,
    /// An access token provider for a service account with the permission
    /// iam.serviceAccounts.getAccessToken on impersonated_service_account.
    access_token_provider: Box<dyn AccessTokenProvider>,
    /// Base URL at which to access GCP IAM service
    iam_service_base_url: &'static str,
    agent: RetryingAgent,
    logger: Logger,
}

impl ImpersonatedServiceAccountIdentityTokenProvider {
    pub(crate) fn new(
        impersonated_service_account: String,
        access_token_provider: GcpAccessTokenProvider,
        logger: Logger,
        api_metrics: &ApiClientMetricsCollector,
    ) -> Self {
        Self {
            impersonated_service_account,
            access_token_provider: Box::new(access_token_provider),
            iam_service_base_url: DEFAULT_IAM_BASE_URL,
            agent: RetryingAgent::new("iamcredentials.googleapis.com", api_metrics),
            logger,
        }
    }

    /// Returns the URL from which identity tokens for the provided GCP service
    /// account may be obtained.
    /// API reference: https://cloud.google.com/iam/docs/reference/credentials/rest/v1/projects.serviceAccounts/generateAccessToken
    fn identity_token_url_for_service_account(&self) -> Result<Url, UrlParseError> {
        let request_url = format!(
            "{}{}",
            self.iam_service_base_url,
            Self::identity_token_path_for_service_account(&self.impersonated_service_account)
        );

        parse_url(request_url)
    }

    /// Returns the path relative to an API base URL from which identity tokens for
    /// the provided GCP service account may be obtained
    fn identity_token_path_for_service_account(service_account_to_impersonate: &str) -> String {
        format!(
            "/v1/projects/-/serviceAccounts/{}:generateIdToken",
            service_account_to_impersonate
        )
    }
}

impl ProvideGcpIdentityToken for ImpersonatedServiceAccountIdentityTokenProvider {
    fn identity_token(&self) -> Result<String> {
        debug!(
            self.logger,
            "obtaining identity token from IAM service";
            "impersonated_service_account" => &self.impersonated_service_account,
        );

        let request = self.agent.prepare_request(RequestParameters {
            url: self.identity_token_url_for_service_account()?,
            method: Method::Post,
            token_provider: Some(self.access_token_provider.as_ref()),
        })?;

        Ok(self
            .agent
            .send_json_request(
                &self.logger,
                &request,
                "generateIdToken",
                &ureq::json!({
                    "audience": IDENTITY_TOKEN_AUDIENCE,
                    // By inspection, the identity tokens obtained from GKE metadata
                    // include `email` and `email_verified` claims, so we request
                    // those here for consistency
                    "includeEmail": true,
                }),
            )
            .context(format!(
                "failed to get identity token to impersonate service account {}",
                self.impersonated_service_account
            ))?
            .into_json::<GenerateIdTokenResponse>()
            .context("failed to deserialize response from IAM API")?
            .token)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct GcpAccessTokenProviderFactoryKey {
    scope: AccessScope,
    account_to_impersonate: Identity,
    key_file: Option<ServiceAccountKeyFile>,
    workload_identity_pool_provider: Option<String>,
}

#[derive(Clone, Debug)]
pub struct GcpAccessTokenProviderFactory {
    runtime_handle: Handle,
    logger: Logger,
    api_metrics: ApiClientMetricsCollector,
    providers: HashMap<GcpAccessTokenProviderFactoryKey, GcpAccessTokenProvider>,
}

impl GcpAccessTokenProviderFactory {
    pub fn new(
        runtime_handle: &Handle,
        api_metrics: &ApiClientMetricsCollector,
        logger: &Logger,
    ) -> Self {
        Self {
            runtime_handle: runtime_handle.clone(),
            logger: logger.clone(),
            api_metrics: api_metrics.clone(),
            providers: HashMap::new(),
        }
    }

    pub(crate) fn get(
        &mut self,
        scope: AccessScope,
        account_to_impersonate: Identity,
        key_file_reader: Option<Box<dyn Read>>,
        workload_identity_pool_parameters: Option<WorkloadIdentityPoolParameters>,
    ) -> Result<GcpAccessTokenProvider> {
        let key_file = match key_file_reader {
            Some(reader) => Some(ServiceAccountKeyFile::try_from(reader)?),
            None => None,
        };
        let key = GcpAccessTokenProviderFactoryKey {
            scope: scope.clone(),
            account_to_impersonate: account_to_impersonate.clone(),
            key_file: key_file.clone(),
            workload_identity_pool_provider: workload_identity_pool_parameters
                .as_ref()
                .map(|p| p.workload_identity_pool_provider.clone()),
        };
        match self.providers.get(&key) {
            Some(provider) => {
                debug!(
                    self.logger,
                    "reusing cached GCP access token provider {:?}", key
                );
                Ok(provider.clone())
            }
            None => {
                debug!(
                    self.logger,
                    "creating new GCP access token provider {:?}", key
                );
                let provider = GcpAccessTokenProvider::new(
                    scope,
                    account_to_impersonate,
                    key_file,
                    workload_identity_pool_parameters,
                    &self.runtime_handle,
                    &self.logger,
                    &self.api_metrics,
                )?;
                self.providers.insert(key, provider.clone());
                Ok(provider)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use mockito::{mock, Matcher};
    use serde_json::json;
    use std::str::FromStr;

    use crate::{config::leak_string, logging::setup_test_logging};

    #[test]
    fn metadata_service_token() {
        let logger = setup_test_logging();
        let api_metrics =
            ApiClientMetricsCollector::new_with_metric_name("metadata_service_token").unwrap();

        let mocked_get = mock("GET", DEFAULT_ACCESS_TOKEN_PATH)
            .match_header("Metadata-Flavor", "Google")
            .with_status(200)
            .with_body(
                r#"{
  "access_token": "fake-token",
  "scope": "fake-scope",
  "token_type": "Bearer",
  "expires_in": 3600
}
"#,
            )
            .expect(1)
            .create();

        let provider = GkeMetadataServiceDefaultAccessTokenProvider {
            agent: RetryingAgent::new("metadata_service_token", &api_metrics),
            logger,
            metadata_service_base_url: leak_string(mockito::server_url()),
        };

        provider
            .default_access_token()
            .unwrap()
            .into_json::<AccessTokenResponse>()
            .unwrap();
        mocked_get.assert();
    }

    #[test]
    fn get_token_with_key_file() {
        let logger = setup_test_logging();
        let api_metrics =
            ApiClientMetricsCollector::new_with_metric_name("get_token_with_key_file").unwrap();

        let key_file = ServiceAccountKeyFile {
            private_key: r#"
-----BEGIN RSA PRIVATE KEY-----
MIIEowIBAAKCAQEAoEwmsVUxIOyq775Bmh2jPb6jtMR8BhWtLuT0O2YgrRMGkx6p
Ld9/svdZVs1AicgMOkv6FkLsXphFobJOyVU8E5E7d1Nk5YZKOdgDltYWtuL4hpAo
Ti2PaCqNDKM1JUG2ffIwNAUAkLoI3+7/aqNnc8pax9dSXknDKtK5Bn4KY0N4JFdo
KXxGJG2jqnguI0hsOGdIpw1UtcQm/1KCMm5SiQcKwRD4gAyAKzkl8jiZzgo46JCK
DRWFTjxC4TWjpD3t8CEZggWmaRB69cMXvrWzvKJTtno/ldI8cskhELJXEuvZ59p3
5YVdHgV9+HycfGkLndei5yubeu9cpOi6moUXHwIDAQABAoIBACBd3/40lnvwbb+E
6hglXd3MzZ9lgSl1XQe4ATyxLW3lBpHUQhLaKx3G5gop3Zs0gouO5cty7elX08+H
gnMSu9Ozoo9AjoHt8LTnUio1xlZdVBNPrmPCvU8qMFrZ5ZRFRYT+zw7h57BRcBNP
XdF5dx0hQd1SM/aH7FmMPQH7lztdhWNehmqVwtMTGq84uOWH+EqCM7bcTCJsG93E
TMPFTcoUGTmpTo98gNSt/kTTf3zktfmTFL6CiwLaXaXl9LQc8XgTG+QZeOYkxk9g
kzE7fn3r6f7N/Hfc2ZutIsFEC7VMX+cUwfKZLsQNeIWvszQOsPVXoQVbjKN9XEQP
U5/oaXkCgYEAz2IhlQEvdMUfR+CX93vAJRHjEQ2lTkMTKSRYyVccaDfF/csurDtc
S9MASiSojHtkiKPyudJxnBK7OMZHPxPx2VZVdfz/XFXEZ+jG6CkgE7suWa/4XJGm
IVcPpMeR0mYQhzbF2HDHRow62ZPwPeflGOuTwtqnhakI2ehmBNfgG/sCgYEAxeA3
iAlTCgnlkXdb7Rj/KPFYZSgoOTxOZqcbVZzQJ1hlEPNICDH62lhwNREkLM9TWzgo
Jb6SUNvpCBaRGMkkLbr2Qfmm6ms/m2osylBfP/xMnCqHeO2XABxtCvOVseYFdCuL
bGpe65Q+bwPpsdQvCj1AcanNxclZhl1+NWXcxC0CgYAc194p1jdee0glfBRGxHxt
63X0WjyCjQuuLjL3FdmKmS89ZDQCmmL03Mzugvi6STMrWfoZZC6O8X/+nn0sRb7e
ZoaOWXi+w+MEPLjlc0rV07PXn4TggxVjD7PKTEN4yt9DnxeXSeA9bKWGu2+vfIA9
ng44DKc+DMuBWzRNOiUeXwKBgDcdQppjbnunUgf4ZORfSALRZjuWuc1nXLb+6IAq
E1hCKLRV7sRJl4NliqtdQOQyQxdvRs9sizh2aCvWjUeIDsml/51UugclJCxXoG4h
gMZDsdr1hZJLKvne8QhR3GoWlYJL9qOV5SZcvh8Rye+8F/YUJXUDRMtIT+U6+UJK
QvlpAoGBAKff7Y0NethwzBEJJl9QGOstwgnsQKrqcDhj7wNX1PKa7r4zu6WdWSF8
2/Hor4D6eW5dNFHhQgC4u/z9JgikHfeCaAgRPMJXxsKWnECP/i++NpiGdhXdtei7
jbxbE/VdW03+iXZyrnDNFAFAsRR+XgjeYheAUVLelg9qBjM7jYNf
-----END RSA PRIVATE KEY-----
"#
            .to_owned(),
            private_key_id: "fake-key-id".to_owned(),
            client_email: "fake@fake.fake".to_owned(),
            token_uri: format!("{}/fake-token-uri", mockito::server_url()),
        };

        // We intentionally don't check the body here: if we did, we would have
        // to re-implement most of account_token_with_key_file to construct the
        // expected body, and all that does is prove we can copy code rather
        // than prove that account_token_with_key_file is correct.
        let mocked_post = mock("POST", "/fake-token-uri")
            .with_status(200)
            .with_body(
                r#"{
  "access_token": "fake-token",
  "scope": "https://www.googleapis.com/auth/cloud-platform",
  "token_type": "Bearer",
  "expires_in": 3600
}
"#,
            )
            .expect(1)
            .create();

        let provider = ServiceAccountKeyFileDefaultAccessTokenProvider {
            key_file,
            scope: AccessScope::CloudPlatform,
            agent: RetryingAgent::new("iamcredentials.googleapis.com", &api_metrics),
            logger,
        };
        provider
            .default_access_token()
            .unwrap()
            .into_json::<AccessTokenResponse>()
            .unwrap();

        mocked_post.assert();
    }

    #[derive(Clone, Debug)]
    struct FakeDefaultAccessTokenProvider {}

    impl ProvideDefaultAccessToken for FakeDefaultAccessTokenProvider {
        fn default_access_token(&self) -> Result<Response, GcpAuthError> {
            Ok(Response::new(
                200,
                "OK",
                r#"{
  "access_token": "fake-default-token",
  "scope": "https://www.googleapis.com/auth/cloud-platform",
  "token_type": "Bearer",
  "expires_in": 3600
}
"#,
            )
            .expect("failed to create response"))
        }
    }

    #[test]
    fn get_impersonated_token() {
        let logger = setup_test_logging();
        let api_metrics =
            ApiClientMetricsCollector::new_with_metric_name("get_impersonated_token").unwrap();

        let access_token_path: &str =
            &GcpAccessTokenProvider::access_token_path_for_service_account("fake-service-account");
        let mocked_post_impersonated = mock("POST", access_token_path)
            .match_header("Authorization", "Bearer fake-default-token")
            .match_body(Matcher::Json(
                json!({"scope": ["https://www.googleapis.com/auth/cloud-platform"] }),
            ))
            .with_status(200)
            .with_body(
                r#"
{
    "accessToken": "fake-impersonated-token",
    "expireTime": "2099-10-02T15:01:23Z"
}
"#,
            )
            .expect(1)
            .create();

        let provider = GcpAccessTokenProvider {
            scope: AccessScope::CloudPlatform,
            default_token_provider: Box::new(FakeDefaultAccessTokenProvider {}),
            account_to_impersonate: Identity::from_str("fake-service-account").unwrap(),
            default_access_token: Arc::new(RwLock::new(None)),
            impersonated_account_token: Arc::new(RwLock::new(None)),
            agent: RetryingAgent::new("iamcredentials.googleapis.com", &api_metrics),
            logger,
            iam_service_base_url: leak_string(mockito::server_url()),
        };

        assert_matches!(
            provider.ensure_impersonated_service_account_access_token(
                provider.account_to_impersonate.as_str().unwrap(),
            ), Ok(token) => {
                assert_eq!(token, "fake-impersonated-token")
            }
        );

        // Get the token again and we should not see any more network requests
        assert_matches!(
            provider.ensure_impersonated_service_account_access_token(
                provider.account_to_impersonate.as_str().unwrap(),
            ), Ok(token) => {
                assert_eq!(token, "fake-impersonated-token")
            }
        );

        mocked_post_impersonated.assert();
    }
}

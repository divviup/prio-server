use anyhow::{anyhow, Context, Result};
use chrono::{prelude::Utc, DateTime, Duration};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use rusoto_core::{credential::ProvideAwsCredentials, Region};
use serde::{Deserialize, Serialize};
use slog::{debug, o, Logger};
use std::{fmt, io::Read, str};
use ureq::Response;
use url::Url;

use crate::{
    aws_credentials::{basic_runtime, get_caller_identity_token},
    config::WorkloadIdentityPoolParameters,
    http::{
        Method, OauthTokenProvider, RequestParameters, RetryingAgent, StaticOauthTokenProvider,
    },
};

fn default_oauth_token_url() -> Url {
    Url::parse("http://metadata.google.internal:80/computeMetadata/v1/instance/service-accounts/default/token",)
    .expect("unable to parse metadata.google.internal url")
}

// API reference:
// https://cloud.google.com/iam/docs/reference/credentials/rest/v1/projects.serviceAccounts/generateAccessToken
fn access_token_url_for_service_account(service_account_to_impersonate: &str) -> Result<Url> {
    let request_url = format!("https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/{}:generateAccessToken",
            service_account_to_impersonate);

    Url::parse(&request_url).context(format!("failed to parse: {}", request_url))
}

/// Represents the claims encoded into JWTs when using a service account key
/// file to authenticate as the default GCP service account.
#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    iss: String,
    scope: String,
    aud: String,
    iat: i64,
    exp: i64,
}

/// A wrapper around an Oauth token and its expiration date.
struct OauthToken {
    token: String,
    expiration: DateTime<Utc>,
}

impl OauthToken {
    /// Returns true if the token is expired.
    fn expired(&self) -> bool {
        Utc::now() >= self.expiration
    }
}

/// Represents the response from a GET request to the GKE metadata service's
/// service account token endpoint, or to oauth2.googleapis.com.token[1], or to
/// sts.googleapis.com's token() endpoint[2]
/// [1] https://developers.google.com/identity/protocols/oauth2/service-account
/// [2] https://cloud.google.com/iam/docs/reference/sts/rest/v1/TopLevel/token
#[derive(Deserialize, PartialEq)]
struct OauthTokenResponse {
    access_token: String,
    expires_in: i64,
    token_type: String,
}

/// Represents the response from a POST request to the GCP IAM service's
/// generateAccessToken endpoint.
/// https://cloud.google.com/iam/docs/reference/credentials/rest/v1/projects.serviceAccounts/generateAccessToken
#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct GenerateAccessTokenResponse {
    access_token: String,
    expire_time: DateTime<Utc>,
}

/// This is the subset of a GCP service account key file that we need to parse
/// to construct a signed JWT.
#[derive(Debug, Deserialize, PartialEq)]
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

/// This enum captures the different ways that a GcpOauthTokenProvider can
/// obtain the default Oauth token, used either to authenticate to GCP services
/// or to obtain a further service account Oauth token from GCP IAM.
enum DefaultTokenProvider<P: ProvideAwsCredentials> {
    /// Fetch a token from the GKE metadata service for the GCP SA mapped to
    /// the current Kubernetes SA via GKE workload identity (not to be confused)
    /// with workload identity *pool*). Used by workloads in Google Kubernetes
    /// Engine.
    GkeMetadataService,
    /// Use a GCP service account key file to authenticate to GCP IAM as some
    /// service account.
    ServiceAccountKeyFile(ServiceAccountKeyFile),
    /// Use a GCP Workload Identity Pool to federate GCP IAM with AWS IAM,
    /// allowing workloads in AWS Elastic Kubernetes Service to impersonate a
    /// GCP SA.
    AwsIamFederationViaWorkloadIdentityPool(WorkloadIdentityPoolParameters<P>),
}

impl<P: ProvideAwsCredentials> fmt::Debug for DefaultTokenProvider<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug_struct = f.debug_struct("DefaultTokenProvider");
        match self {
            Self::GkeMetadataService => debug_struct.field(
                "token_source",
                &"default service account from GKE metadata service".to_owned(),
            ),
            Self::ServiceAccountKeyFile(key_file) => debug_struct
                .field("token_source", &"service account key file".to_owned())
                .field("service_account", &key_file.client_email),
            Self::AwsIamFederationViaWorkloadIdentityPool(_) => debug_struct.field(
                "token_source",
                &"AWS IAM federation via workload identity pool".to_owned(),
            ),
        };
        debug_struct.finish()
    }
}

impl<P: ProvideAwsCredentials> DefaultTokenProvider<P> {
    fn default_token(
        &self,
        scope: &str,
        agent: &RetryingAgent,
        logger: &Logger,
    ) -> Result<Response> {
        match self {
            Self::GkeMetadataService => self.account_token_from_gke_metadata_service(agent, logger),
            Self::ServiceAccountKeyFile(key_file) => {
                self.account_token_with_key_file(&key_file, scope, agent, logger)
            }
            Self::AwsIamFederationViaWorkloadIdentityPool(parameters) => self
                .account_token_with_workload_identity_pool(
                    &parameters.aws_credentials_provider,
                    &parameters.workload_identity_pool_provider,
                    agent,
                    logger,
                ),
        }
    }

    /// Fetches default account token from GKE metadata service. Returns the
    /// ureq::Response, whose body will be an OauthTokenResponse if the HTTP
    /// call was successful.
    fn account_token_from_gke_metadata_service(
        &self,
        agent: &RetryingAgent,
        logger: &Logger,
    ) -> Result<Response> {
        debug!(
            logger,
            "obtaining default account token from GKE metadata service"
        );

        let mut request = agent.prepare_request(RequestParameters {
            url: default_oauth_token_url(),
            method: Method::Get,
            ..Default::default()
        })?;

        request = request.set("Metadata-Flavor", "Google");

        agent
            .call(logger, &request)
            .context("failed to query GKE metadata service")
    }

    /// Fetches the default account token from Google OAuth API using a JWT
    /// constructed from the parameters in the provided key file. If the JWT is
    /// successfully constructed, returns the ureq::Response whose body will be
    /// an OauthTokenResponse if the HTTP call was successful, but may be an
    /// error.
    fn account_token_with_key_file(
        &self,
        key_file: &ServiceAccountKeyFile,
        scope: &str,
        agent: &RetryingAgent,
        logger: &Logger,
    ) -> Result<Response> {
        debug!(logger, "obtaining account token from key file");
        // We construct the JWT per Google documentation:
        // https://developers.google.com/identity/protocols/oauth2/service-account#authorizingrequests
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(key_file.private_key_id.to_owned());

        // The iat and exp fields in a JWT are in seconds since UNIX epoch.
        let now = Utc::now().timestamp();
        let claims = Claims {
            iss: key_file.client_email.to_owned(),
            scope: scope.to_owned(),
            aud: key_file.token_uri.to_owned(),
            iat: now,
            exp: now + 3600, // token expires in one hour
        };

        let encoding_key = EncodingKey::from_rsa_pem(key_file.private_key.as_bytes())
            .context("failed to parse PEM RSA key")?;

        let token =
            encode(&header, &claims, &encoding_key).context("failed to construct and sign JWT")?;

        let request_body = format!(
            "grant_type={}&assertion={}",
            urlencoding::encode("urn:ietf:params:oauth:grant-type:jwt-bearer"),
            token
        );

        let request = agent.prepare_request(RequestParameters {
            url: Url::parse(&key_file.token_uri).context(format!(
                "failed to parse key_file.token_uri: {}",
                &key_file.token_uri
            ))?,
            method: Method::Post,
            ..Default::default()
        })?;

        let request = request.set("Content-Type", "application/x-www-form-urlencoded");
        agent
            .send_string(logger, &request, &request_body)
            .context("failed to get account token with key file")
    }

    /// Uses a GCP workload identity pool provider to fetch a federated access
    /// token which can later be presented to iamcredentials.googleapis.com to
    /// obtain an Oauth token that allows impersonation of some GCP service
    /// account.
    /// https://cloud.google.com/iam/docs/access-resources-aws#exchange-token
    fn account_token_with_workload_identity_pool(
        &self,
        aws_credentials: &P,
        workload_identity_pool_provider: &str,
        agent: &RetryingAgent,
        logger: &Logger,
    ) -> Result<Response> {
        debug!(logger, "getting GCP workload identity pool federated token");
        // rusoto_core::Region::default consults AWS_DEFAULT_REGION and
        // AWS_REGION environment variables to figure out what region we are
        // running in, which we need to pick an STS endpoint
        let aws_region = Region::default();

        // First, we must obtain credentials for the AWS IAM role mapped to this
        // facilitator's Kubernetes service account
        let credentials = basic_runtime()?
            .block_on(aws_credentials.credentials())
            .context("failed to get AWS credentials")?;

        // Next, we construct a GetCallerIdentity token
        let sts_request_url = Url::parse(&format!(
            "https://sts.{}.amazonaws.com?Action=GetCallerIdentity&Version=2011-06-15",
            aws_region.name()
        ))
        .context("failed to parse STS request URL")?;

        let get_caller_identity_token = get_caller_identity_token(
            &sts_request_url,
            &workload_identity_pool_provider,
            &aws_region,
            &credentials,
        )?;

        // The GetCallerIdentity token goes into the body of the request sent to
        // sts.googleapis.com (one assumes that GCP relays that request to
        // sts.amazonaws.com to prove that we control the AWS IAM role).
        let request_body = ureq::json!({
            "audience": &workload_identity_pool_provider,
            "grantType": "urn:ietf:params:oauth:grant-type:token-exchange",
            "requestedTokenType": "urn:ietf:params:oauth:token-type:access_token",
            "scope": "https://www.googleapis.com/auth/cloud-platform",
            "subjectToken": urlencoding::encode(&get_caller_identity_token.to_string()),
            "subjectTokenType": "urn:ietf:params:aws:token-type:aws4_request",
        });

        let request = agent
            .prepare_request(RequestParameters {
                url: Url::parse("https://sts.googleapis.com/v1/token")
                    .context("failed to construct sts.googleapis.com URL")?,
                method: Method::Post,
                // This request is unauthenticated, except for the signature and
                // token on the inner subjectToken
                token_provider: None,
            })?
            .set("Content-Type", "application/json; charset=utf-8");

        debug!(
            logger,
            "obtaining federated access token. request {:?}\ngci_token {}\nbody {}",
            request,
            get_caller_identity_token,
            request_body
        );
        agent
            .send_json_request(logger, &request, &request_body)
            .context("failed to obtain federated access token from sts.googleapis.com")
    }
}

/// GcpOauthTokenProvider manages a default service account Oauth token (i.e. the
/// one for a GCP service account mapped to a Kubernetes service account, or the
/// one found in a JSON key file) and an Oauth token used to impersonate another
/// service account.
pub(crate) struct GcpOauthTokenProvider<P: ProvideAwsCredentials> {
    /// The Oauth scope for which tokens should be requested.
    scope: String,
    /// Provides the default Oauth token, which may be used to directly access
    /// GCP services or may be used to impersonate some GCP service account.
    default_token_provider: DefaultTokenProvider<P>,
    /// Holds the service account email to impersonate, if one was provided to
    /// GcpOauthTokenProvider::new.
    account_to_impersonate: Option<String>,
    /// This field is None after instantiation and is Some after the first
    /// successful request for a token for the default service account, though
    /// the contained token may be expired.
    default_account_token: Option<OauthToken>,
    /// This field is None after instantiation and is Some after the first
    /// successful request for a token for the impersonated service account,
    /// though the contained token may be expired. This will always be None if
    /// account_to_impersonate is None.
    impersonated_account_token: Option<OauthToken>,
    /// The agent will be used when making HTTP requests to GCP APIs to fetch
    /// Oauth tokens.
    agent: RetryingAgent,
    /// Logger to which messages will be logged
    logger: Logger,
}

impl<P: ProvideAwsCredentials> fmt::Debug for GcpOauthTokenProvider<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OauthTokenProvider")
            .field("account_to_impersonate", &self.account_to_impersonate)
            .field("default_token_provider", &self.default_token_provider)
            .field(
                "default_account_token",
                &self.default_account_token.as_ref().map(|_| "redacted"),
            )
            .field(
                "impersonated_account_token",
                &self.default_account_token.as_ref().map(|_| "redacted"),
            )
            .finish()
    }
}

impl<P: ProvideAwsCredentials> OauthTokenProvider for GcpOauthTokenProvider<P> {
    /// Returns the Oauth token to use with GCP API in an Authorization header,
    /// fetching it or renewing it if necessary. If a service account to
    /// impersonate was provided, the default service account is used to
    /// authenticate to the GCP IAM API to retrieve an Oauth token. If no
    /// impersonation is taking place, provides the default service account
    /// Oauth token.
    fn ensure_oauth_token(&mut self) -> Result<String> {
        match self.account_to_impersonate {
            Some(_) => self.ensure_impersonated_service_account_oauth_token(),
            None => self.ensure_default_account_token(),
        }
    }
}

impl<P: ProvideAwsCredentials> GcpOauthTokenProvider<P> {
    /// Creates a token provider which can impersonate the specified service
    /// account.
    pub(crate) fn new(
        scope: &str,
        account_to_impersonate: Option<String>,
        key_file_reader: Option<Box<dyn Read>>,
        workload_identity_pool_params: Option<WorkloadIdentityPoolParameters<P>>,
        parent_logger: &Logger,
    ) -> Result<Self> {
        let default_token_provider = match (key_file_reader, workload_identity_pool_params) {
            (Some(_), Some(_)) => {
                return Err(anyhow!(
                    "either but not both of key_file_reader or aws_credentials may be provided"
                ))
            }
            (Some(reader), None) => DefaultTokenProvider::ServiceAccountKeyFile(
                serde_json::from_reader(reader).context("failed to deserialize JSON key file")?,
            ),
            (None, Some(parameters)) => {
                DefaultTokenProvider::AwsIamFederationViaWorkloadIdentityPool(parameters)
            }
            (None, None) => DefaultTokenProvider::GkeMetadataService,
        };

        let logger = parent_logger.new(o!(
            "scope" => scope.to_owned(),
            "account_to_impersonate" => account_to_impersonate.clone().unwrap_or_else(|| "none".to_owned()),
        ));

        Ok(GcpOauthTokenProvider {
            scope: scope.to_owned(),
            default_token_provider,
            account_to_impersonate,
            default_account_token: None,
            impersonated_account_token: None,
            agent: RetryingAgent::default(),
            logger,
        })
    }

    /// Returns the current OAuth token for the default service account, if it
    /// is valid. Otherwise obtains and returns a new one.
    /// The returned value is an owned reference because the token owned by this
    /// struct could change while the caller is still holding the returned token
    fn ensure_default_account_token(&mut self) -> Result<String> {
        if let Some(token) = &self.default_account_token {
            if !token.expired() {
                debug!(self.logger, "cached default account token is still valid");
                return Ok(token.token.clone());
            }
        }

        let http_response =
            self.default_token_provider
                .default_token(&self.scope, &self.agent, &self.logger)?;

        let response = http_response
            .into_json::<OauthTokenResponse>()
            .context("failed to deserialize response from GCP token service")?;

        if response.token_type != "Bearer" {
            return Err(anyhow!("unexpected token type {}", response.token_type));
        }

        self.default_account_token = Some(OauthToken {
            token: response.access_token.clone(),
            expiration: Utc::now() + Duration::seconds(response.expires_in),
        });

        Ok(response.access_token)
    }

    /// Returns the current OAuth token for the impersonated service account, if
    /// it is valid. Otherwise obtains and returns a new one.
    fn ensure_impersonated_service_account_oauth_token(&mut self) -> Result<String> {
        if self.account_to_impersonate.is_none() {
            return Err(anyhow!("no service account to impersonate was provided"));
        }

        if let Some(token) = &self.impersonated_account_token {
            if !token.expired() {
                debug!(
                    self.logger,
                    "cached token is still valid for impersonating service account"
                );
                return Ok(token.token.clone());
            }
        }

        let service_account_to_impersonate = self.account_to_impersonate.clone().unwrap();

        let default_token = self.ensure_default_account_token()?;
        debug!(self.logger, "default token: {}", default_token);

        let request = self.agent.prepare_request(RequestParameters {
            url: access_token_url_for_service_account(&service_account_to_impersonate)?,
            method: Method::Post,
            token_provider: Some(&mut StaticOauthTokenProvider::from(default_token)),
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
                &ureq::json!({
                    "scope": [self.scope]
                }),
            )
            .context(format!(
                "failed to get Oauth token to impersonate service account {}",
                service_account_to_impersonate
            ))?;

        let response = http_response
            .into_json::<GenerateAccessTokenResponse>()
            .context("failed to deserialize response from IAM API")?;
        self.impersonated_account_token = Some(OauthToken {
            token: response.access_token.clone(),
            expiration: response.expire_time,
        });

        Ok(response.access_token)
    }
}

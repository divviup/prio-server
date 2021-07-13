use anyhow::{anyhow, Context, Result};
use chrono::{prelude::Utc, DateTime, Duration};
use dyn_clone::DynClone;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use rusoto_core::{credential::ProvideAwsCredentials, Region};
use serde::{Deserialize, Serialize};
use slog::{debug, o, Logger};
use std::{
    fmt::{self, Debug},
    io::Read,
    str,
    sync::{Arc, RwLock},
};
use ureq::Response;
use url::Url;

use crate::{
    aws_credentials::{self, basic_runtime, get_caller_identity_token},
    config::{Identity, WorkloadIdentityPoolParameters},
    http::{
        Method, OauthTokenProvider, RequestParameters, RetryingAgent, StaticOauthTokenProvider,
    },
};

const DEFAULT_METADATA_BASE_URL: &str = "http://metadata.google.internal:80";
const DEFAULT_TOKEN_PATH: &str = "/computeMetadata/v1/instance/service-accounts/default/token";
const DEFAULT_IAM_BASE_URL: &str = "https://iamcredentials.googleapis.com";

fn default_oauth_token_url(base: &str) -> Url {
    let mut request_url = Url::parse(base).expect("unable to parse metadata.google.internal url");
    request_url.set_path(DEFAULT_TOKEN_PATH);
    request_url
}

// API reference:
// https://cloud.google.com/iam/docs/reference/credentials/rest/v1/projects.serviceAccounts/generateAccessToken
fn access_token_url_for_service_account(
    base: &str,
    service_account_to_impersonate: &str,
) -> Result<Url> {
    let request_url = format!(
        "{}{}",
        base,
        access_token_path_for_service_account(service_account_to_impersonate)
    );

    Url::parse(&request_url).context(format!("failed to parse: {}", request_url))
}

fn access_token_path_for_service_account(service_account_to_impersonate: &str) -> String {
    format!(
        "/v1/projects/-/serviceAccounts/{}:generateAccessToken",
        service_account_to_impersonate
    )
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
#[derive(Clone)]
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
/// sts.googleapis.com's token method[2]
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
#[derive(Clone, Debug, Deserialize, PartialEq)]
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

/// Implementations of ProvideDefaultToken obtain a default Oauth token, used
/// either to authenticate to GCP services or to obtain a further service
/// account Oauth token from GCP IAM.
trait ProvideDefaultToken: DynClone + Debug {
    fn default_token(&self) -> Result<Response>;
}

dyn_clone::clone_trait_object!(ProvideDefaultToken);

/// Fetches a token from the GKE metadata service for the GCP SA mapped to the
/// current Kubernetes SA via GKE workload identity (not to be confused with
/// workload identity *pool*). Used by workloads in Google Kubernetes Engine.
#[derive(Clone, Debug)]
struct GkeMetadataServiceDefaultTokenProvider {
    agent: RetryingAgent,
    logger: Logger,
    /// Base URL at which to access GKE metadata service
    metadata_service_base_url: &'static str,
}

impl GkeMetadataServiceDefaultTokenProvider {
    fn new(agent: RetryingAgent, logger: Logger) -> Self {
        GkeMetadataServiceDefaultTokenProvider {
            agent,
            logger,
            metadata_service_base_url: DEFAULT_METADATA_BASE_URL,
        }
    }
}

impl ProvideDefaultToken for GkeMetadataServiceDefaultTokenProvider {
    fn default_token(&self) -> Result<Response> {
        debug!(
            self.logger,
            "obtaining default account token from GKE metadata service"
        );

        let mut request = self.agent.prepare_request(RequestParameters {
            url: default_oauth_token_url(self.metadata_service_base_url),
            method: Method::Get,
            ..Default::default()
        })?;

        request = request.set("Metadata-Flavor", "Google");

        self.agent
            .call(&self.logger, &request)
            .context("failed to query GKE metadata service")
    }
}

/// Uses a GCP service account key file to authenticate to GCP IAM as some
/// service account.
#[derive(Clone, Debug)]
struct ServiceAccountKeyFileDefaultTokenProvider {
    key_file: ServiceAccountKeyFile,
    scope: String,
    agent: RetryingAgent,
    logger: Logger,
}

impl ProvideDefaultToken for ServiceAccountKeyFileDefaultTokenProvider {
    fn default_token(&self) -> Result<Response> {
        debug!(self.logger, "obtaining account token from key file");
        // We construct the JWT per Google documentation:
        // https://developers.google.com/identity/protocols/oauth2/service-account#authorizingrequests
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(self.key_file.private_key_id.to_owned());

        // The iat and exp fields in a JWT are in seconds since UNIX epoch.
        let now = Utc::now().timestamp();
        let claims = Claims {
            iss: self.key_file.client_email.to_owned(),
            scope: self.scope.clone(),
            aud: self.key_file.token_uri.to_owned(),
            iat: now,
            exp: now + 3600, // token expires in one hour
        };

        let encoding_key = EncodingKey::from_rsa_pem(self.key_file.private_key.as_bytes())
            .context("failed to parse PEM RSA key")?;

        let token =
            encode(&header, &claims, &encoding_key).context("failed to construct and sign JWT")?;

        let request = self.agent.prepare_request(RequestParameters {
            url: Url::parse(&self.key_file.token_uri).context(format!(
                "failed to parse key_file.token_uri: {}",
                &self.key_file.token_uri
            ))?,
            method: Method::Post,
            ..Default::default()
        })?;

        self.agent
            .send_form(
                &self.logger,
                &request,
                &[
                    ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                    ("assertion", &token),
                ],
            )
            .context("failed to get account token with key file")
    }
}

/// Uses a GCP Workload Identity Pool to federate GCP IAM with AWS IAM, allowing
/// workloads in AWS Elastic Kubernetes Service to impersonate a GCP SA.
#[derive(Clone)]
struct AwsIamFederationViaWorkloadIdentityPoolDefaultTokenProvider {
    aws_credentials_provider: aws_credentials::Provider,
    workload_identity_pool_provider: String,
    logger: Logger,
    agent: RetryingAgent,
}

impl Debug for AwsIamFederationViaWorkloadIdentityPoolDefaultTokenProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AwsIamFederationViaWorkloadIdentityPoolDefaultTokenProvider")
            .field(
                "token_source",
                &"AWS IAM federation via workload identity pool",
            )
            .finish()
    }
}

impl ProvideDefaultToken for AwsIamFederationViaWorkloadIdentityPoolDefaultTokenProvider {
    fn default_token(&self) -> Result<Response> {
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
        let credentials = basic_runtime()?
            .block_on(self.aws_credentials_provider.credentials())
            .context("failed to get AWS credentials")?;

        // Next, we construct a GetCallerIdentity token
        let sts_request_url = Url::parse(&format!(
            "https://sts.{}.amazonaws.com?Action=GetCallerIdentity&Version=2011-06-15",
            aws_region.name()
        ))
        .context("failed to parse STS request URL")?;

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
            "scope": "https://www.googleapis.com/auth/cloud-platform",
            "subjectToken": urlencoding::encode(&get_caller_identity_token.to_string()),
            "subjectTokenType": "urn:ietf:params:aws:token-type:aws4_request",
        });

        let request = self
            .agent
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
            self.logger,
            "obtaining federated access token. request {:?}\ngci_token {}\nbody {}",
            request,
            get_caller_identity_token,
            request_body
        );
        self.agent
            .send_json_request(&self.logger, &request, &request_body)
            .context("failed to obtain federated access token from sts.googleapis.com")
    }
}

/// GcpOauthTokenProvider manages a default service account Oauth token (i.e. the
/// one for a GCP service account mapped to a Kubernetes service account, or the
/// one found in a JSON key file) and an Oauth token used to impersonate another
/// service account.
///
/// A note on thread safety: this struct stores any Oauth tokens it obtains in
/// an Arc+Mutex, so an instance of GcpOauthTokenProvider may be .clone()d
/// liberally and shared across threads, and credentials obtained from the GCP
/// credentials API will be shared efficiently and safely.
#[derive(Clone)]
pub(crate) struct GcpOauthTokenProvider {
    /// The Oauth scope for which tokens should be requested.
    scope: String,
    /// Provides the default Oauth token, which may be used to directly access
    /// GCP services or may be used to impersonate some GCP service account.
    default_token_provider: Box<dyn ProvideDefaultToken>,
    /// Holds the service account email to impersonate, if one was provided to
    /// GcpOauthTokenProvider::new.
    account_to_impersonate: Identity,
    /// This field is None after instantiation and is Some after the first
    /// successful request for a token for the default service account, though
    /// the contained token may be expired.
    default_account_token: Arc<RwLock<Option<OauthToken>>>,
    /// This field is None after instantiation and is Some after the first
    /// successful request for a token for the impersonated service account,
    /// though the contained token may be expired. This will always be None if
    /// account_to_impersonate is None.
    impersonated_account_token: Arc<RwLock<Option<OauthToken>>>,
    /// The agent will be used when making HTTP requests to GCP APIs to fetch
    /// Oauth tokens.
    agent: RetryingAgent,
    /// Logger to which messages will be logged
    logger: Logger,
    /// Base URL at which to access GCP IAM service
    iam_service_base_url: &'static str,
}

impl fmt::Debug for GcpOauthTokenProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OauthTokenProvider")
            .field("account_to_impersonate", &self.account_to_impersonate)
            .field("default_token_provider", &self.default_token_provider)
            .field(
                "default_account_token",
                &self
                    .default_account_token
                    .read()
                    .unwrap()
                    .as_ref()
                    .map(|_| "redacted"),
            )
            .field(
                "impersonated_account_token",
                &self
                    .default_account_token
                    .read()
                    .unwrap()
                    .as_ref()
                    .map(|_| "redacted"),
            )
            .finish()
    }
}

impl OauthTokenProvider for GcpOauthTokenProvider {
    /// Returns the Oauth token to use with GCP API in an Authorization header,
    /// fetching it or renewing it if necessary. If a service account to
    /// impersonate was provided, the default service account is used to
    /// authenticate to the GCP IAM API to retrieve an Oauth token. If no
    /// impersonation is taking place, provides the default service account
    /// Oauth token.
    fn ensure_oauth_token(&mut self) -> Result<String> {
        match self.account_to_impersonate.as_str() {
            Some(_) => self.ensure_impersonated_service_account_oauth_token(),
            None => self.ensure_default_account_token(),
        }
    }
}

impl GcpOauthTokenProvider {
    /// Creates a token provider which can impersonate the specified service
    /// account.
    pub(crate) fn new(
        scope: &str,
        account_to_impersonate: Identity,
        key_file_reader: Option<Box<dyn Read>>,
        workload_identity_pool_params: Option<WorkloadIdentityPoolParameters>,
        parent_logger: &Logger,
    ) -> Result<Self> {
        let logger = parent_logger.new(o!(
            "scope" => scope.to_owned(),
            "account_to_impersonate" => account_to_impersonate.to_string(),
        ));
        let agent = RetryingAgent::default();

        let default_token_provider: Box<dyn ProvideDefaultToken> =
            match (key_file_reader, workload_identity_pool_params) {
                (Some(_), Some(_)) => {
                    return Err(anyhow!(
                        "either but not both of key_file_reader or aws_credentials may be provided"
                    ))
                }
                (Some(reader), None) => Box::new(ServiceAccountKeyFileDefaultTokenProvider {
                    key_file: serde_json::from_reader(reader)
                        .context("failed to deserialize JSON key file")?,
                    scope: scope.to_owned(),
                    agent: agent.clone(),
                    logger: logger.clone(),
                }),
                (None, Some(parameters)) => Box::new(
                    AwsIamFederationViaWorkloadIdentityPoolDefaultTokenProvider {
                        aws_credentials_provider: parameters.aws_credentials_provider,
                        workload_identity_pool_provider: parameters.workload_identity_pool_provider,
                        logger: logger.clone(),
                        agent: agent.clone(),
                    },
                ),
                (None, None) => Box::new(GkeMetadataServiceDefaultTokenProvider::new(
                    agent.clone(),
                    logger.clone(),
                )),
            };

        Ok(GcpOauthTokenProvider {
            scope: scope.to_owned(),
            default_token_provider,
            account_to_impersonate,
            default_account_token: Arc::new(RwLock::new(None)),
            impersonated_account_token: Arc::new(RwLock::new(None)),
            agent,
            logger,
            iam_service_base_url: DEFAULT_IAM_BASE_URL,
        })
    }

    /// Returns the current OAuth token for the default service account, if it
    /// is valid. Otherwise obtains and returns a new one.
    /// The returned value is an owned reference because the token owned by this
    /// struct could change while the caller is still holding the returned token
    fn ensure_default_account_token(&mut self) -> Result<String> {
        if let Some(token) = &*self.default_account_token.read().unwrap() {
            if !token.expired() {
                debug!(self.logger, "cached default account token is still valid");
                return Ok(token.token.clone());
            }
        }

        let mut default_account_token = self.default_account_token.write().unwrap();

        // Check if the token was updated between when we dropped the read lock
        // and when we acquired the write lock
        if let Some(token) = &*default_account_token {
            if !token.expired() {
                debug!(self.logger, "cached default account token is still valid");
                return Ok(token.token.clone());
            }
        }

        let http_response = self.default_token_provider.default_token()?;

        let response = http_response
            .into_json::<OauthTokenResponse>()
            .context("failed to deserialize response from GCP token service")?;

        if response.token_type != "Bearer" {
            return Err(anyhow!("unexpected token type {}", response.token_type));
        }

        *default_account_token = Some(OauthToken {
            token: response.access_token.clone(),
            expiration: Utc::now() + Duration::seconds(response.expires_in),
        });

        Ok(response.access_token)
    }

    /// Returns the current OAuth token for the impersonated service account, if
    /// it is valid. Otherwise obtains and returns a new one.
    fn ensure_impersonated_service_account_oauth_token(&mut self) -> Result<String> {
        if self.account_to_impersonate.as_str().is_none() {
            return Err(anyhow!("no service account to impersonate was provided"));
        }

        if let Some(token) = &*self.impersonated_account_token.read().unwrap() {
            if !token.expired() {
                debug!(
                    self.logger,
                    "cached token is still valid for impersonating service account"
                );
                return Ok(token.token.clone());
            }
        }

        let default_token = self.ensure_default_account_token()?;
        let mut impersonated_account_token = self.impersonated_account_token.write().unwrap();
        let service_account_to_impersonate = match self.account_to_impersonate.as_str() {
            Some(account) => account,
            None => return Err(anyhow!("no service account to impersonate was provided")),
        };

        let request = self.agent.prepare_request(RequestParameters {
            url: access_token_url_for_service_account(
                self.iam_service_base_url,
                &service_account_to_impersonate,
            )?,
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
        *impersonated_account_token = Some(OauthToken {
            token: response.access_token.clone(),
            expiration: response.expire_time,
        });

        Ok(response.access_token)
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
        let mocked_get = mock("GET", DEFAULT_TOKEN_PATH)
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

        let provider = GkeMetadataServiceDefaultTokenProvider {
            agent: RetryingAgent::default(),
            logger,
            metadata_service_base_url: leak_string(mockito::server_url()),
        };

        provider
            .default_token()
            .unwrap()
            .into_json::<OauthTokenResponse>()
            .unwrap();
        mocked_get.assert();
    }

    #[test]
    fn get_token_with_key_file() {
        let logger = setup_test_logging();

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
  "scope": "fake-scope",
  "token_type": "Bearer",
  "expires_in": 3600
}
"#,
            )
            .expect(1)
            .create();

        let provider = ServiceAccountKeyFileDefaultTokenProvider {
            key_file,
            scope: "fake-scope".to_owned(),
            agent: RetryingAgent::default(),
            logger,
        };
        provider
            .default_token()
            .unwrap()
            .into_json::<OauthTokenResponse>()
            .unwrap();

        mocked_post.assert();
    }

    #[derive(Clone, Debug)]
    struct FakeDefaultTokenProvider {}

    impl ProvideDefaultToken for FakeDefaultTokenProvider {
        fn default_token(&self) -> Result<Response> {
            Response::new(
                200,
                "OK",
                r#"{
  "access_token": "fake-default-token",
  "scope": "fake-scope",
  "token_type": "Bearer",
  "expires_in": 3600
}
"#,
            )
            .context("failed to create response")
        }
    }

    #[test]
    fn get_impersonated_token() {
        let logger = setup_test_logging();

        let access_token_path: &str =
            &access_token_path_for_service_account("fake-service-account");
        let mocked_post_impersonated = mock("POST", access_token_path)
            .match_header("Authorization", "Bearer fake-default-token")
            .match_body(Matcher::Json(json!({"scope": ["fake-scope"] })))
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

        let mut provider = GcpOauthTokenProvider {
            scope: "fake-scope".to_string(),
            default_token_provider: Box::new(FakeDefaultTokenProvider {}),
            account_to_impersonate: Identity::from_str("fake-service-account").unwrap(),
            default_account_token: Arc::new(RwLock::new(None)),
            impersonated_account_token: Arc::new(RwLock::new(None)),
            agent: RetryingAgent::default(),
            logger,
            iam_service_base_url: leak_string(mockito::server_url()),
        };

        assert_matches!(provider.ensure_impersonated_service_account_oauth_token(), Ok(token) => {
            assert_eq!(token, "fake-impersonated-token")
        });

        // Get the token again and we should not see any more network requests
        assert_matches!(provider.ensure_impersonated_service_account_oauth_token(), Ok(token) => {
            assert_eq!(token, "fake-impersonated-token")
        });

        mocked_post_impersonated.assert();
    }
}

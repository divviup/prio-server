use anyhow::{anyhow, Context, Result};
use chrono::{prelude::Utc, DateTime, Duration};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use slog_scope::debug;
use std::{
    fmt,
    io::Read,
    sync::{Arc, RwLock},
};
use ureq::Response;
use url::Url;

use crate::http::{
    Method, OauthTokenProvider, RequestParameters, RetryingAgent, StaticOauthTokenProvider,
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
/// service account token endpoint, or to oauth2.googleapis.com.token.
/// https://developers.google.com/identity/protocols/oauth2/service-account
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
    /// The parsed key file for the default GCP service account. If present,
    /// this will be used to obtain the default account OAuth token. If absent,
    /// the GKE metadata service is consulted.
    default_service_account_key_file: Option<ServiceAccountKeyFile>,
    /// Holds the service account email to impersonate, if one was provided to
    /// GcpOauthTokenProvider::new.
    account_to_impersonate: Option<String>,
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
    /// Base URL at which to access GKE metadata service
    metadata_service_base_url: &'static str,
    /// Base URL at which to access GCP IAM service
    iam_service_base_url: &'static str,
}

impl fmt::Debug for GcpOauthTokenProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OauthTokenProvider")
            .field("account_to_impersonate", &self.account_to_impersonate)
            .field(
                "default_service_account_key_file",
                &self
                    .default_service_account_key_file
                    .as_ref()
                    .map(|key_file| key_file.client_email.clone()),
            )
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
        match self.account_to_impersonate {
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
        account_to_impersonate: Option<String>,
        key_file_reader: Option<Box<dyn Read>>,
    ) -> Result<Self> {
        let key_file: Option<ServiceAccountKeyFile> = match key_file_reader {
            Some(reader) => {
                serde_json::from_reader(reader).context("failed to deserialize JSON key file")?
            }
            None => None,
        };

        Ok(GcpOauthTokenProvider {
            scope: scope.to_owned(),
            default_service_account_key_file: key_file,
            account_to_impersonate,
            default_account_token: Arc::new(RwLock::new(None)),
            impersonated_account_token: Arc::new(RwLock::new(None)),
            agent: RetryingAgent::default(),
            metadata_service_base_url: DEFAULT_METADATA_BASE_URL,
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
                return Ok(token.token.clone());
            }
        }

        let mut default_account_token = self.default_account_token.write().unwrap();

        let http_response = match &self.default_service_account_key_file {
            Some(key_file) => self.account_token_with_key_file(&key_file)?,
            None => self.account_token_from_gke_metadata_service()?,
        };

        let response = http_response
            .into_json::<OauthTokenResponse>()
            .context("failed to deserialize response from GKE metadata service")?;

        if response.token_type != "Bearer" {
            return Err(anyhow!("unexpected token type {}", response.token_type));
        }

        *default_account_token = Some(OauthToken {
            token: response.access_token.clone(),
            expiration: Utc::now() + Duration::seconds(response.expires_in),
        });

        Ok(response.access_token)
    }

    /// Fetches default account token from GKE metadata service. Returns the
    /// ureq::Response, whose body will be an OauthTokenResponse if the HTTP
    /// call was successful.
    fn account_token_from_gke_metadata_service(&self) -> Result<Response> {
        debug!(
            "obtaining default account token from GKE metadata service. Scope: {}",
            self.scope
        );

        let mut request = self.agent.prepare_request(RequestParameters {
            url: default_oauth_token_url(self.metadata_service_base_url),
            method: Method::Get,
            ..Default::default()
        })?;

        request = request.set("Metadata-Flavor", "Google");

        self.agent
            .call(&request)
            .context("failed to query GKE metadata service")
    }

    /// Fetches the default account token from Google OAuth API using a JWT
    /// constructed from the parameters in the provided key file. If the JWT is
    /// successfully constructed, returns the ureq::Response whose body will be
    /// an OauthTokenResponse if the HTTP call was successful, but may be an
    /// error.
    fn account_token_with_key_file(&self, key_file: &ServiceAccountKeyFile) -> Result<Response> {
        debug!(
            "obtaining account token from key file. Scope: {}",
            self.scope
        );
        // We construct the JWT per Google documentation:
        // https://developers.google.com/identity/protocols/oauth2/service-account#authorizingrequests
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(key_file.private_key_id.to_owned());

        // The iat and exp fields in a JWT are in seconds since UNIX epoch.
        let now = Utc::now().timestamp();
        let claims = Claims {
            iss: key_file.client_email.to_owned(),
            scope: self.scope.to_owned(),
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

        let request = self.agent.prepare_request(RequestParameters {
            url: Url::parse(&key_file.token_uri).context(format!(
                "failed to parse key_file.token_uri: {}",
                &key_file.token_uri
            ))?,
            method: Method::Post,
            ..Default::default()
        })?;

        let request = request.set("Content-Type", "application/x-www-form-urlencoded");
        self.agent
            .send_string(&request, &request_body)
            .context("failed to get account token with key file")
    }

    /// Returns the current OAuth token for the impersonated service account, if
    /// it is valid. Otherwise obtains and returns a new one.
    fn ensure_impersonated_service_account_oauth_token(&mut self) -> Result<String> {
        if self.account_to_impersonate.is_none() {
            return Err(anyhow!("no service account to impersonate was provided"));
        }

        if let Some(token) = &*self.impersonated_account_token.read().unwrap() {
            if !token.expired() {
                debug!(
                    "cached token is still valid for service account {:?} and scope {:?}",
                    self.account_to_impersonate, self.scope
                );
                return Ok(token.token.clone());
            }
        }

        let default_token = self.ensure_default_account_token()?;
        let mut impersonated_account_token = self.impersonated_account_token.write().unwrap();
        let service_account_to_impersonate = self.account_to_impersonate.clone().unwrap();

        let request = self.agent.prepare_request(RequestParameters {
            url: access_token_url_for_service_account(
                self.iam_service_base_url,
                &service_account_to_impersonate,
            )?,
            method: Method::Post,
            token_provider: Some(&mut StaticOauthTokenProvider::from(default_token)),
        })?;

        debug!(
            "obtaining token for service account {:?} and scope {:?}",
            self.account_to_impersonate, self.scope
        );

        let http_response = self
            .agent
            .send_json_request(
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
    use std::io::Cursor;

    use crate::config::leak_string;

    #[test]
    fn get_default_token() {
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
            .expect_at_most(1)
            .create();

        let mut provider = GcpOauthTokenProvider::new("fake-scope", None, None).unwrap();
        provider.metadata_service_base_url = leak_string(mockito::server_url());
        provider.iam_service_base_url = leak_string(mockito::server_url());

        assert_matches!(provider.ensure_default_account_token(), Ok(token) => {
            assert_eq!(token, "fake-token")
        });

        // Should fail because provider.account_to_impersonate = None
        assert_matches!(
            provider.ensure_impersonated_service_account_oauth_token(),
            Err(_)
        );

        // Get the token again and we should not see any more network requests
        assert_matches!(provider.ensure_default_account_token(), Ok(token) => {
            assert_eq!(token, "fake-token")
        });

        mocked_get.assert();
    }

    #[test]
    fn get_impersonated_token() {
        let mocked_get_default = mock("GET", DEFAULT_TOKEN_PATH)
            .match_header("Metadata-Flavor", "Google")
            .with_status(200)
            .with_body(
                r#"{
  "access_token": "fake-default-token",
  "scope": "fake-scope",
  "token_type": "Bearer",
  "expires_in": 3600
}
"#,
            )
            .expect_at_most(1)
            .create();

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
            .expect_at_most(1)
            .create();

        let mut provider = GcpOauthTokenProvider::new(
            "fake-scope",
            Some("fake-service-account".to_string()),
            None,
        )
        .unwrap();
        provider.metadata_service_base_url = leak_string(mockito::server_url());
        provider.iam_service_base_url = leak_string(mockito::server_url());

        assert_matches!(provider.ensure_impersonated_service_account_oauth_token(), Ok(token) => {
            assert_eq!(token, "fake-impersonated-token")
        });

        // Get the token again and we should not see any more network requests
        assert_matches!(provider.ensure_impersonated_service_account_oauth_token(), Ok(token) => {
            assert_eq!(token, "fake-impersonated-token")
        });

        mocked_get_default.assert();
        mocked_post_impersonated.assert();
    }

    #[test]
    fn get_token_with_key_file() {
        let key_file = format!(
            r#"{{
    "private_key": "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEAoEwmsVUxIOyq775Bmh2jPb6jtMR8BhWtLuT0O2YgrRMGkx6p\nLd9/svdZVs1AicgMOkv6FkLsXphFobJOyVU8E5E7d1Nk5YZKOdgDltYWtuL4hpAo\nTi2PaCqNDKM1JUG2ffIwNAUAkLoI3+7/aqNnc8pax9dSXknDKtK5Bn4KY0N4JFdo\nKXxGJG2jqnguI0hsOGdIpw1UtcQm/1KCMm5SiQcKwRD4gAyAKzkl8jiZzgo46JCK\nDRWFTjxC4TWjpD3t8CEZggWmaRB69cMXvrWzvKJTtno/ldI8cskhELJXEuvZ59p3\n5YVdHgV9+HycfGkLndei5yubeu9cpOi6moUXHwIDAQABAoIBACBd3/40lnvwbb+E\n6hglXd3MzZ9lgSl1XQe4ATyxLW3lBpHUQhLaKx3G5gop3Zs0gouO5cty7elX08+H\ngnMSu9Ozoo9AjoHt8LTnUio1xlZdVBNPrmPCvU8qMFrZ5ZRFRYT+zw7h57BRcBNP\nXdF5dx0hQd1SM/aH7FmMPQH7lztdhWNehmqVwtMTGq84uOWH+EqCM7bcTCJsG93E\nTMPFTcoUGTmpTo98gNSt/kTTf3zktfmTFL6CiwLaXaXl9LQc8XgTG+QZeOYkxk9g\nkzE7fn3r6f7N/Hfc2ZutIsFEC7VMX+cUwfKZLsQNeIWvszQOsPVXoQVbjKN9XEQP\nU5/oaXkCgYEAz2IhlQEvdMUfR+CX93vAJRHjEQ2lTkMTKSRYyVccaDfF/csurDtc\nS9MASiSojHtkiKPyudJxnBK7OMZHPxPx2VZVdfz/XFXEZ+jG6CkgE7suWa/4XJGm\nIVcPpMeR0mYQhzbF2HDHRow62ZPwPeflGOuTwtqnhakI2ehmBNfgG/sCgYEAxeA3\niAlTCgnlkXdb7Rj/KPFYZSgoOTxOZqcbVZzQJ1hlEPNICDH62lhwNREkLM9TWzgo\nJb6SUNvpCBaRGMkkLbr2Qfmm6ms/m2osylBfP/xMnCqHeO2XABxtCvOVseYFdCuL\nbGpe65Q+bwPpsdQvCj1AcanNxclZhl1+NWXcxC0CgYAc194p1jdee0glfBRGxHxt\n63X0WjyCjQuuLjL3FdmKmS89ZDQCmmL03Mzugvi6STMrWfoZZC6O8X/+nn0sRb7e\nZoaOWXi+w+MEPLjlc0rV07PXn4TggxVjD7PKTEN4yt9DnxeXSeA9bKWGu2+vfIA9\nng44DKc+DMuBWzRNOiUeXwKBgDcdQppjbnunUgf4ZORfSALRZjuWuc1nXLb+6IAq\nE1hCKLRV7sRJl4NliqtdQOQyQxdvRs9sizh2aCvWjUeIDsml/51UugclJCxXoG4h\ngMZDsdr1hZJLKvne8QhR3GoWlYJL9qOV5SZcvh8Rye+8F/YUJXUDRMtIT+U6+UJK\nQvlpAoGBAKff7Y0NethwzBEJJl9QGOstwgnsQKrqcDhj7wNX1PKa7r4zu6WdWSF8\n2/Hor4D6eW5dNFHhQgC4u/z9JgikHfeCaAgRPMJXxsKWnECP/i++NpiGdhXdtei7\njbxbE/VdW03+iXZyrnDNFAFAsRR+XgjeYheAUVLelg9qBjM7jYNf\n-----END RSA PRIVATE KEY-----",
    "private_key_id": "fake-key-id",
    "client_email": "fake@fake.fake",
    "token_uri": "{}/fake-token-uri"
}}
"#,
            mockito::server_url()
        );

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
            .expect_at_most(1)
            .create();

        let mut provider =
            GcpOauthTokenProvider::new("fake-scope", None, Some(Box::new(Cursor::new(key_file))))
                .unwrap();
        provider.metadata_service_base_url = leak_string(mockito::server_url());
        provider.iam_service_base_url = leak_string(mockito::server_url());

        assert_matches!(provider.ensure_default_account_token(), Ok(token) => {
            assert_eq!(token, "fake-token")
        });

        // Should fail because provider.account_to_impersonate = None
        assert_matches!(
            provider.ensure_impersonated_service_account_oauth_token(),
            Err(_)
        );

        // Get the token again and we should not see any more requests
        assert_matches!(provider.ensure_default_account_token(), Ok(token) => {
            assert_eq!(token, "fake-token")
        });

        mocked_post.assert();
    }
}

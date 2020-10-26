use crate::{
    config::GCSPath,
    transport::{Transport, TransportWriter},
    Error,
};
use anyhow::{anyhow, Context, Result};
use chrono::{prelude::Utc, DateTime, Duration};
use serde::Deserialize;
use std::{
    io,
    io::{Read, Write},
};

const STORAGE_API_BASE_URL: &str = "https://storage.googleapis.com";
const DEFAULT_OAUTH_TOKEN_URL: &str =
    "http://metadata.google.internal:80/computeMetadata/v1/instance/service-accounts/default/token";

/// A wrapper around an Oauth token and its expiration date.
#[derive(Debug)]
struct OauthToken {
    token: String,
    expiration: DateTime<Utc>,
}

impl OauthToken {
    /// Returns true if the token is not yet expired.
    fn expired(&self) -> bool {
        Utc::now() < self.expiration
    }
}

/// Represents the response from a GET request to the GKE metadata service's
/// service account token endpoint. Structure is derived from empirical
/// observation of the JSON scraped from inside a GKE job.
#[derive(Debug, Deserialize, PartialEq)]
struct MetadataServiceTokenResponse {
    access_token: String,
    expires_in: i64,
    token_type: String,
}

/// Represents the response from a POST request to the GCP IAM service's
/// generateAccessToken endpoint.
/// https://cloud.google.com/iam/docs/reference/credentials/rest/v1/projects.serviceAccounts/generateAccessToken
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct GenerateAccessTokenResponse {
    access_token: String,
    expire_time: DateTime<Utc>,
}

/// OauthTokenProvider manages a default service account Oauth token (i.e. the
/// one for a GCP service account mapped to a Kubernetes service account) and an
/// Oauth token used to impersonate another service account.
struct OauthTokenProvider {
    /// Holds the service account email to impersonate, if one was provided to
    /// OauthTokenProvider::new.
    service_account_to_impersonate: Option<String>,
    /// This field is None after instantiation and is Some after the first
    /// successful request for a token for the default service account, though
    /// the contained token may be expired.
    default_service_account_oauth_token: Option<OauthToken>,
    /// This field is None after instantiation and is Some after the first
    /// successful request for a token for the impersonated service account,
    /// though the contained token may be expired. This will always be None if
    /// service_account_to_impersonate is None.
    impersonated_service_account_oauth_token: Option<OauthToken>,
}

impl OauthTokenProvider {
    /// Creates a token provider which can impersonate the specified service
    /// account.
    fn new(service_account_to_impersonate: Option<String>) -> OauthTokenProvider {
        OauthTokenProvider {
            service_account_to_impersonate,
            default_service_account_oauth_token: None,
            impersonated_service_account_oauth_token: None,
        }
    }

    /// Returns the Oauth token to use with GCS storage API in an Authorization
    /// header, fetching it or renewing it if necessary. If a service account to
    /// impersonate was provided, the default service account is used to
    /// authenticate to the GCP IAM API to retrieve an Oauth token. If no
    /// impersonation is taking place, provides the default service account
    /// Oauth token.
    fn ensure_storage_access_oauth_token(&mut self) -> Result<String> {
        match self.service_account_to_impersonate {
            Some(_) => self.ensure_impersonated_service_account_oauth_token(),
            None => self.ensure_default_service_account_oauth_token(),
        }
    }

    /// Returns the current OAuth token for the default service account, if it
    /// is valid. Otherwise obtains and returns a new one.
    /// The returned value is an owned reference because the token owned by this
    /// struct could change while the caller is still holding the returned token
    fn ensure_default_service_account_oauth_token(&mut self) -> Result<String> {
        if let Some(token) = &self.default_service_account_oauth_token {
            if !token.expired() {
                return Ok(token.token.clone());
            }
        }

        let http_response = ureq::get(DEFAULT_OAUTH_TOKEN_URL)
            .set("Metadata-Flavor", "Google")
            // By default, ureq will wait forever to connect or read.
            .timeout_connect(10_000) // ten seconds
            .timeout_read(10_000) // ten seconds
            .call();
        if http_response.error() {
            return Err(anyhow!(
                "failed to query GKE metadata service: {:?}",
                http_response
            ));
        }

        let response = http_response
            .into_json_deserialize::<MetadataServiceTokenResponse>()
            .context("failed to deserialize response from GKE metadata service")?;

        if response.token_type != "Bearer" {
            return Err(anyhow!("unexpected token type {}", response.token_type));
        }

        self.default_service_account_oauth_token = Some(OauthToken {
            token: response.access_token.clone(),
            expiration: Utc::now() + Duration::seconds(response.expires_in),
        });

        Ok(response.access_token)
    }

    /// Returns the current OAuth token for the impersonated service account, if
    /// it is valid. Otherwise obtains and returns a new one.
    fn ensure_impersonated_service_account_oauth_token(&mut self) -> Result<String> {
        if self.service_account_to_impersonate.is_none() {
            return Err(anyhow!("no service account to impersonate was provided"));
        }

        if let Some(token) = &self.impersonated_service_account_oauth_token {
            if !token.expired() {
                return Ok(token.token.clone());
            }
        }

        let service_account_to_impersonate = self.service_account_to_impersonate.clone().unwrap();
        // API reference:
        // https://cloud.google.com/iam/docs/reference/credentials/rest/v1/projects.serviceAccounts/generateAccessToken
        let request_url = format!("https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/{}:generateAccessToken",
            service_account_to_impersonate);
        let auth = format!(
            "Bearer {}",
            self.ensure_default_service_account_oauth_token()?
        );
        let http_response = ureq::post(&request_url)
            .set("Authorization", &auth)
            .set("Content-Type", "application/json")
            // At the moment, these tokens are only used to read and write from
            // GCS buckets, so we request an appropriate scope. We could further
            // narrow this to a devstorage.read token if we knew what it was
            // going to be used for, but that would require this object to
            // manage two impersonation tokens which is more work than I want to
            // do right now.
            // https://cloud.google.com/storage/docs/authentication#oauth-scopes
            .send_json(ureq::json!({
                "scope": [
                    "https://www.googleapis.com/auth/devstorage.read_write"
                ]
            }));
        if http_response.error() {
            return Err(anyhow!(
                "failed to get Oauth token to impersonate service account {}: {:?}",
                service_account_to_impersonate,
                http_response
            ));
        }

        let response = http_response
            .into_json_deserialize::<GenerateAccessTokenResponse>()
            .context("failed to deserialize response from IAM API")?;
        self.impersonated_service_account_oauth_token = Some(OauthToken {
            token: response.access_token.clone(),
            expiration: response.expire_time,
        });

        Ok(response.access_token)
    }
}

/// GCSTransport manages reading and writing from GCS buckets, with
/// authenticatiom to the API by Oauth token in an Authorization header. This
/// struct can either use the default service account from the metadata service,
/// or can impersonate another GCP service account if one is provided to
/// GCSTransport::new.
pub struct GCSTransport {
    path: GCSPath,
    oauth_token_provider: OauthTokenProvider,
}

impl GCSTransport {
    /// Instantiate a new GCSTransport to read or write objects from or to the
    /// provided path. If impersonate is None, GCSTransport authenticates to GCS
    /// as the default service account. If impersonate contains a service
    /// account email, GCSTransport will use the GCP IAM API to obtain an Oauth
    /// token to impersonate that service account.
    pub fn new(path: GCSPath, impersonate: Option<String>) -> GCSTransport {
        GCSTransport {
            path: path.ensure_directory_prefix(),
            oauth_token_provider: OauthTokenProvider::new(impersonate),
        }
    }
}

impl Transport for GCSTransport {
    fn get(&mut self, key: &str) -> Result<Box<dyn Read>> {
        // Per API reference, the object key must be URL encoded.
        // API reference: https://cloud.google.com/storage/docs/json_api/v1/objects/get
        let encoded_key = urlencoding::encode(&[&self.path.key, key].concat());
        let url = format!(
            "{}/storage/v1/b/{}/o/{}",
            STORAGE_API_BASE_URL, self.path.bucket, encoded_key
        );

        let response = ureq::get(&url)
            // Ensures response body will be content and not JSON metadata.
            // https://cloud.google.com/storage/docs/json_api/v1/objects/get#parameters
            .query("alt", "media")
            .set(
                "Authorization",
                &format!(
                    "Bearer {}",
                    self.oauth_token_provider
                        .ensure_storage_access_oauth_token()?
                ),
            )
            // By default, ureq will wait forever to connect or read
            .timeout_connect(10_000) // ten seconds
            .timeout_read(10_000) // ten seconds
            .call();
        if response.error() {
            return Err(anyhow!(
                "failed to fetch object {} from GCS: {:?}",
                url,
                response
            ));
        }
        Ok(Box::new(response.into_reader()))
    }

    fn put(&mut self, key: &str) -> Result<Box<dyn TransportWriter>> {
        // The Oauth token will only be used once, during the call to
        // StreamingTransferWriter::new, so we don't have to worry about it
        // expiring during the lifetime of that object, and so obtain a token
        // here instead of passing the token provider into the
        // StreamingTransferWriter.
        let oauth_token = self
            .oauth_token_provider
            .ensure_storage_access_oauth_token()?;
        Ok(Box::new(StreamingTransferWriter::new(
            self.path.bucket.to_owned(),
            [&self.path.key, key].concat(),
            oauth_token,
        )?))
    }
}

// StreamingTransferWriter implements GCS's resumable, streaming upload feature,
// allowing us to stream data into the GCS buckets.
//
// The GCS resumable, streaming upload API is, frankly, diabolical. The idea is
// that you initiate a transfer with a POST request to an upload endpoint, as in
// StreamingTransferWriter::new_with_api_url, which gets you an upload session
// URI. Then, you perform multiple PUTs to the session URI that each have a
// Content-Range header indicating which chunk of the object they make up. As we
// don't know the final length of the object, we set Content-Range: bytes x-y/*,
// where x and y are the indices of the current slice. We indicate that an
// object upload is finished by setting the final, total size of the object in
// the last field of the Content-Range header in the last PUT request to the
// upload session URI.
// Now, Google mandates that upload chunks be at least 256 KiB, and recommend a
// chunk size of 8 MiB. Where it gets hair raising is that there's no guarantee
// a whole chunk will be uploaded at once: responses to the PUT requests include
// a Range header telling you how much of the chunk Google got, so you can build
// the next PUT request appropriately. So suppose you are trying to upload an 8
// MiB chunk from the middle of the overall object, and you succeed, but Google
// tells you they didn't get the last 100 KiB. You might right away want to
// upload that last 100 KiB, but if you try, you will fail because it's not the
// final chunk and it's less than 256 KiB. So we do two special things in
// upload_chunk when we know it's the last chunk: (1) we construct the Content-
// Range header without any asterisks (2) we drain self.buffer.
struct StreamingTransferWriter {
    upload_session_uri: String,
    minimum_upload_chunk_size: usize,
    object_upload_position: usize,
    buffer: Vec<u8>,
}

impl StreamingTransferWriter {
    /// Creates a new writer that streams content in chunks into GCS. Bucket is
    /// the name of the GCS bucket. Object is the full name of the object being
    /// uploaded, which may contain path separators or file extensions.
    /// oauth_token is used to initiate the initial resumable upload request.
    fn new(bucket: String, object: String, oauth_token: String) -> Result<StreamingTransferWriter> {
        StreamingTransferWriter::new_with_api_url(
            bucket,
            object,
            oauth_token,
            // GCP documentation recommends setting upload part size to 8 MiB.
            // https://cloud.google.com/storage/docs/performing-resumable-uploads#chunked-upload
            8_388_608,
            STORAGE_API_BASE_URL,
        )
    }

    fn new_with_api_url(
        bucket: String,
        object: String,
        oauth_token: String,
        minimum_upload_chunk_size: usize,
        storage_api_base_url: &str,
    ) -> Result<StreamingTransferWriter> {
        // Initiate the resumable, streaming upload.
        // https://cloud.google.com/storage/docs/performing-resumable-uploads#initiate-session
        let encoded_object = urlencoding::encode(&object);
        let upload_url = format!("{}/upload/storage/v1/b/{}/o/", storage_api_base_url, bucket);
        let http_response = ureq::post(&upload_url)
            .set("Authorization", &format!("Bearer {}", oauth_token))
            .query("uploadType", "resumable")
            .query("name", &encoded_object)
            // By default, ureq will wait forever to connect or read
            .timeout_connect(10_000) // ten seconds
            .timeout_read(10_000) // ten seconds
            .send_bytes(&[]);
        if http_response.error() {
            return Err(anyhow!(
                "failed to initiate streaming transfer: {:?}",
                http_response
            ));
        }

        // The upload session URI authenticates subsequent upload requests for
        // this upload, so we no longer need the impersonated service account's
        // Oauth token. Session URIs are valid for a week, which should be more
        // than enough for any upload we perform.
        // https://cloud.google.com/storage/docs/resumable-uploads#session-uris
        let upload_session_uri = http_response
            .header("Location")
            .context("no Location header in response when initiating streaming transfer")?;

        Ok(StreamingTransferWriter {
            minimum_upload_chunk_size,
            buffer: Vec::with_capacity(minimum_upload_chunk_size * 2),
            object_upload_position: 0,
            upload_session_uri: upload_session_uri.to_owned(),
        })
    }

    fn upload_chunk(&mut self, last_chunk: bool) -> Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        if !last_chunk && self.buffer.len() < self.minimum_upload_chunk_size {
            return Err(anyhow!(
                "insufficient content accumulated in buffer to upload chunk"
            ));
        }

        // We may not yet be at the last chunk, and hence may not know the total
        // length of eventual complete object, but this variable contains the
        // total length of all the bytes fed into write(), even if we haven't
        // uploaded them all yet.
        let object_length_thus_far = self.object_upload_position + self.buffer.len();

        // When this is the last piece being uploaded, the Content-Range header
        // should include the total object size, but otherwise should have * to
        // indicate to GCS that there is an unknown further amount to come.
        // https://cloud.google.com/storage/docs/streaming#streaming_uploads
        let (body, content_range_header_total_length_field) =
            if last_chunk && self.buffer.len() < self.minimum_upload_chunk_size {
                (self.buffer.as_ref(), format!("{}", object_length_thus_far))
            } else {
                (
                    &self.buffer[..self.minimum_upload_chunk_size],
                    "*".to_owned(),
                )
            };

        let content_range = format!(
            "bytes {}-{}/{}",
            self.object_upload_position,
            self.object_upload_position + body.len() - 1,
            content_range_header_total_length_field
        );

        let http_response = ureq::put(&self.upload_session_uri)
            .set("Content-Range", &content_range)
            // By default, ureq will wait forever to connect or read
            .timeout_connect(10_000) // ten seconds
            .timeout_read(10_000) // ten seconds
            .send_bytes(body);

        // On success we expect HTTP 308 Resume Incomplete and a Range: header,
        // unless this is the last part and the server accepts the entire
        // provided Content-Range, in which case it's HTTP 200, or 201 (?).
        // https://cloud.google.com/storage/docs/performing-resumable-uploads#chunked-upload
        match http_response.status() {
            200 | 201 if last_chunk => {
                // Truncate the buffer to "drain" it of uploaded bytes
                self.buffer.truncate(0);
                Ok(())
            }
            200 | 201 => Err(anyhow!(
                "received HTTP 200 or 201 response with chunks remaining"
            )),
            308 if !http_response.has("Range") => Err(anyhow!(
                "No range header in response from GCS: {:?}",
                http_response.into_string()
            )),
            308 => {
                let range_header = http_response.header("Range").unwrap();
                // The range header is like "bytes=0-222", and represents the
                // uploaded portion of the overall object, not the current chunk
                let end = range_header
                    .strip_prefix("bytes=0-")
                    .context(format!(
                        "Range header {} missing bytes prefix",
                        range_header
                    ))?
                    .parse::<usize>()
                    .context("End in range header {} not a valid usize")?;
                // end is usize and so parse would fail if the value in the
                // header was negative, but we still defend ourselves against
                // it being bigger than is possible given our position in the
                // object.
                if end >= object_length_thus_far {
                    return Err(anyhow!("End in range header {} is invalid", range_header));
                }

                // If we have a little content left over, we can't just make
                // another request, because if there's too little of it, Google
                // will reject it. Instead, leave the portion of the chunk that
                // we didn't manage to upload back in self.buffer so it can be
                // handled by a subsequent call to upload_chunk.
                let new_buf = self.buffer.split_off(end + 1 - self.object_upload_position);
                self.buffer = new_buf;
                self.object_upload_position = end + 1;
                Ok(())
            }
            _ => Err(anyhow!(
                "failed to upload part to GCS: {} synthetic: {}\n{:?}",
                http_response.status(),
                http_response.synthetic(),
                http_response.into_string()
            )),
        }
    }
}

impl Write for StreamingTransferWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Write into memory buffer, and upload to GCS if we have accumulated
        // enough content
        self.buffer.extend_from_slice(buf);
        while self.buffer.len() >= self.minimum_upload_chunk_size {
            self.upload_chunk(false)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, Error::AnyhowError(e)))?;
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        // It may not be possible to flush this if we have accumulated less
        // content in the buffer than Google will allow us to upload, and users
        // of a TransportWriter are expected to call complete_upload when they
        // know they are finished anyway, so we just report success.
        Ok(())
    }
}

impl TransportWriter for StreamingTransferWriter {
    fn complete_upload(&mut self) -> Result<()> {
        while !self.buffer.is_empty() {
            self.upload_chunk(true)?;
        }
        Ok(())
    }

    fn cancel_upload(&mut self) -> Result<()> {
        // https://cloud.google.com/storage/docs/performing-resumable-uploads#cancel-upload
        let http_response = ureq::delete(&self.upload_session_uri)
            .set("Content-Length", "0")
            // By default, ureq will wait forever to connect or read
            .timeout_connect(10_000) // ten seconds
            .timeout_read(10_000) // ten seconds
            .call();
        match http_response.status() {
            499 => Ok(()),
            _ => Err(anyhow!(
                "failed to cancel streaming transfer to GCS: {:?}",
                http_response
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::{mock, Matcher};

    #[test]
    fn simple_upload() {
        let fake_upload_session_uri = format!("{}/fake-session-uri", mockito::server_url());
        let mocked_post = mock("POST", "/upload/storage/v1/b/fake-bucket/o/")
            .match_header("Authorization", "Bearer fake-token")
            .match_header("Content-Length", "0")
            .match_query(Matcher::UrlEncoded(
                "uploadType".to_owned(),
                "resumable".to_owned(),
            ))
            .match_query(Matcher::UrlEncoded(
                "name".to_owned(),
                "fake-object".to_owned(),
            ))
            .with_status(200)
            .with_header("Location", &fake_upload_session_uri)
            .expect_at_most(1)
            .create();

        let mut writer = StreamingTransferWriter::new_with_api_url(
            "fake-bucket".to_string(),
            "fake-object".to_string(),
            "fake-token".to_string(),
            10,
            &mockito::server_url(),
        )
        .unwrap();

        mocked_post.assert();

        let mocked_put = mock("PUT", "/fake-session-uri")
            .match_header("Content-Length", "7")
            .match_header("Content-Range", "bytes 0-6/7")
            .match_body("content")
            .with_status(200)
            .expect_at_most(1)
            .create();

        assert_eq!(writer.write(b"content").unwrap(), 7);
        writer.complete_upload().unwrap();

        mocked_put.assert();
    }

    #[test]
    fn multi_chunk_upload() {
        let fake_upload_session_uri = format!("{}/fake-session-uri", mockito::server_url());
        let mocked_post = mock("POST", "/upload/storage/v1/b/fake-bucket/o/")
            .match_header("Authorization", "Bearer fake-token")
            .match_header("Content-Length", "0")
            .match_query(Matcher::UrlEncoded(
                "uploadType".to_owned(),
                "resumable".to_owned(),
            ))
            .match_query(Matcher::UrlEncoded(
                "name".to_owned(),
                "fake-object".to_owned(),
            ))
            .with_status(200)
            .with_header("Location", &fake_upload_session_uri)
            .expect_at_most(1)
            .create();

        let mut writer = StreamingTransferWriter::new_with_api_url(
            "fake-bucket".to_string(),
            "fake-object".to_string(),
            "fake-token".to_string(),
            4,
            &mockito::server_url(),
        )
        .unwrap();

        mocked_post.assert();

        let first_mocked_put = mock("PUT", "/fake-session-uri")
            .match_header("Content-Length", "4")
            .match_header("Content-Range", "bytes 0-3/*")
            .match_body("0123")
            .with_status(308)
            .with_header("Range", "bytes=0-3")
            .expect_at_most(1)
            .create();

        let second_mocked_put = mock("PUT", "/fake-session-uri")
            .match_header("Content-Length", "4")
            .match_header("Content-Range", "bytes 4-7/*")
            .match_body("4567")
            .with_status(308)
            .with_header("Range", "bytes=0-6")
            .expect_at_most(1)
            .create();

        let final_mocked_put = mock("PUT", "/fake-session-uri")
            .match_header("Content-Length", "3")
            .match_header("Content-Range", "bytes 7-9/10")
            .match_body("789")
            .with_status(200)
            .expect_at_most(1)
            .create();

        assert_eq!(writer.write(b"0123456789").unwrap(), 10);
        writer.complete_upload().unwrap();

        first_mocked_put.assert();
        second_mocked_put.assert();
        final_mocked_put.assert();
    }
}

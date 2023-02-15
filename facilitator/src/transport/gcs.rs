use crate::{
    config::{GcsPath, Identity, WorkloadIdentityPoolParameters},
    gcp_oauth::{AccessScope, GcpAccessTokenProvider, GcpAccessTokenProviderFactory, GcpAuthError},
    http::{
        AccessTokenProvider, Method, RequestParameters, RetryingAgent, StaticAccessTokenProvider,
    },
    logging::event,
    metrics::ApiClientMetricsCollector,
    parse_url,
    transport::{Transport, TransportWriter},
    UrlParseError,
};
use slog::{debug, info, o, warn, Logger};
use std::{
    io::{self, Read, Write},
    num::ParseIntError,
    time::Duration,
};
use ureq::AgentBuilder;
use url::Url;
use uuid::Uuid;

use super::TransportError;

fn storage_api_base_url() -> Url {
    Url::parse("https://storage.googleapis.com/").expect("unable to parse storage API url")
}

fn gcp_object_url(bucket: &str, encoded_key: &str) -> Result<Url, UrlParseError> {
    let request_url = format!(
        "{}storage/v1/b/{}/o/{}",
        storage_api_base_url(),
        bucket,
        encoded_key
    );

    parse_url(request_url)
}

fn gcp_upload_object_url(storage_api_url: &str, bucket: &str) -> Result<Url, UrlParseError> {
    let request_url = format!("{storage_api_url}upload/storage/v1/b/{bucket}/o/");

    parse_url(request_url)
}

#[derive(Debug, thiserror::Error)]
pub enum GcsRangeHeaderError {
    #[error("No range header in response from GCS: {0:?}")]
    Missing(std::io::Result<String>),
    #[error("Range header {0} missing bytes prefix")]
    BadPrefix(String),
    #[error("End in range header {1} not a valid usize: {0}")]
    InvalidEnd(ParseIntError, String),
    #[error("End in range header {0} is invalid")]
    OutOfRangeEnd(String),
}

#[derive(Debug, thiserror::Error)]
pub enum GcsError {
    #[error(transparent)]
    Auth(#[from] GcpAuthError),
    #[error("failed to fetch object {1} from GCS: {0}")]
    Get(ureq::Error, Url),
    #[error(transparent)]
    Url(#[from] UrlParseError),
    #[error("received HTTP 200 or 201 response with chunks remaining")]
    EarlyChunkOk,
    #[error("insufficient content accumulated in buffer to upload chunk")]
    ChunkUnderrun,
    #[error("no Location header in response when initiating streaming transfer")]
    NoLocationHeader,
    #[error(transparent)]
    Range(#[from] GcsRangeHeaderError),
    #[error("uploading to gs://{1} failed: {0}")]
    Upload(ureq::Error, String),
    #[error("failed in sending bytes to gcs upload session {1}: {0}")]
    ChunkUpload(ureq::Error, Url),
    #[error("failed to upload part to GCS: {0}\n{1:?}")]
    ChunkUploadStatus(u16, std::io::Result<String>),
    #[error("failed to cancel upload: {0}")]
    Cancel(ureq::Error),
    #[error("failed to cancel streaming transfer to GCS: {0}\n{1:?}")]
    CancelStatus(u16, std::io::Result<String>),
}

/// GCSTransport manages reading and writing from GCS buckets, with
/// authenticatiom to the API by Oauth token in an Authorization header. This
/// struct can either use the default service account from the metadata service,
/// or can impersonate another GCP service account if one is provided to
/// GCSTransport::new.
#[derive(Clone, Debug)]
pub struct GcsTransport {
    path: GcsPath,
    oauth_token_provider: GcpAccessTokenProvider,
    agent: RetryingAgent,
    logger: Logger,
}

impl GcsTransport {
    /// Instantiate a new GCSTransport to read or write objects from or to the
    /// provided path. If identity is None, GCSTransport authenticates to GCS
    /// as the default service account. If identity contains a service
    /// account email, GCSTransport will use the GCP IAM API to obtain an Oauth
    /// token to impersonate that service account.
    pub fn new(
        path: GcsPath,
        identity: Identity,
        key_file_reader: Option<Box<dyn Read>>,
        workload_identity_pool_params: Option<WorkloadIdentityPoolParameters>,
        gcp_access_token_provider_factory: &mut GcpAccessTokenProviderFactory,
        parent_logger: &Logger,
        api_metrics: &ApiClientMetricsCollector,
    ) -> anyhow::Result<Self> {
        let logger = parent_logger.new(o!(
            event::STORAGE_PATH => path.to_string(),
            event::IDENTITY => identity.to_string(),
        ));
        let ureq_agent = AgentBuilder::new()
            // We set an unusually long timeout for uploads to GCS, per Google's
            // recommendation:
            // https://cloud.google.com/storage/docs/best-practices#uploading
            .timeout(Duration::from_secs(120))
            .build();
        let retrying_agent = RetryingAgent::new_with_agent(
            ureq_agent,
            // Per Google documentation, HTTP 408 Request Timeout and HTTP 429
            // Too Many Requests shouldbe retried
            // https://cloud.google.com/storage/docs/retry-strategy
            vec![408, 429],
            "storage.googleapis.com",
            api_metrics,
        );
        Ok(GcsTransport {
            path: path.ensure_directory_prefix(),
            oauth_token_provider: gcp_access_token_provider_factory.get(
                AccessScope::DevStorageReadWrite,
                identity,
                key_file_reader,
                workload_identity_pool_params,
            )?,
            agent: retrying_agent,
            logger,
        })
    }
}

impl Transport for GcsTransport {
    fn path(&self) -> String {
        self.path.to_string()
    }

    fn get(&self, key: &str, trace_id: &Uuid) -> Result<Box<dyn Read>, TransportError> {
        let logger = self.logger.new(o!(
            event::TRACE_ID => trace_id.to_string(),
            event::STORAGE_KEY => key.to_owned(),
            event::ACTION => "get GCS object"
        ));
        info!(logger, "get");

        // Per API reference, the object key must be URL encoded.
        // API reference: https://cloud.google.com/storage/docs/json_api/v1/objects/get
        let mut url = gcp_object_url(
            &self.path.bucket,
            &urlencoding::encode(&[&self.path.key, key].concat()),
        )
        .map_err(GcsError::Url)?;

        // Ensures response body will be content and not JSON metadata.
        // https://cloud.google.com/storage/docs/json_api/v1/objects/get#parameters
        url.query_pairs_mut().append_pair("alt", "media").finish();

        let request = self
            .agent
            .prepare_request(RequestParameters {
                url: url.clone(),
                method: Method::Get,
                token_provider: Some(&self.oauth_token_provider),
            })
            .map_err(|e| TransportError::Gcs(e.into()))?;

        let response = self.agent.call(&logger, &request, "get").map_err(|err| {
            if matches!(err, ureq::Error::Status(404, _)) {
                return TransportError::ObjectNotFoundError(
                    key.to_owned(),
                    anyhow::Error::new(err),
                );
            }
            TransportError::Gcs(GcsError::Get(err, url))
        })?;

        Ok(Box::new(response.into_reader()))
    }

    fn put(&self, key: &str, trace_id: &Uuid) -> Result<Box<dyn TransportWriter>, TransportError> {
        let logger = self.logger.new(o!(
            event::TRACE_ID => trace_id.to_string(),
            event::STORAGE_KEY => key.to_owned(),
            event::ACTION => "put GCS object",
        ));
        info!(logger, "put");

        // The Oauth token will only be used once, during the call to
        // StreamingTransferWriter::new, so we don't have to worry about it
        // expiring during the lifetime of that object, and so obtain a token
        // here instead of passing the token provider into the
        // StreamingTransferWriter.
        let oauth_token = self
            .oauth_token_provider
            .ensure_access_token()
            .map_err(|e| TransportError::Gcs(e.into()))?;
        let writer = StreamingTransferWriter::new(
            self.path.bucket.to_owned(),
            [&self.path.key, key].concat(),
            oauth_token,
            self.agent.clone(),
            &logger,
        )?;
        Ok(Box::new(writer))
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
    upload_session_uri: Url,
    minimum_upload_chunk_size: usize,
    object_upload_position: usize,
    buffer: Vec<u8>,
    agent: RetryingAgent,
    logger: Logger,
    is_finished: bool,
    did_upload_chunk: bool,
}

impl StreamingTransferWriter {
    /// Creates a new writer that streams content in chunks into GCS. Bucket is
    /// the name of the GCS bucket. Object is the full name of the object being
    /// uploaded, which may contain path separators or file extensions.
    /// oauth_token is used to initiate the initial resumable upload request.
    #[allow(clippy::result_large_err)]
    fn new(
        bucket: String,
        object: String,
        oauth_token: String,
        agent: RetryingAgent,
        parent_logger: &Logger,
    ) -> Result<StreamingTransferWriter, GcsError> {
        StreamingTransferWriter::new_with_api_url(
            bucket,
            object,
            oauth_token,
            // GCP documentation recommends setting upload part size to 8 MiB.
            // https://cloud.google.com/storage/docs/performing-resumable-uploads#chunked-upload
            8_388_608,
            storage_api_base_url(),
            agent,
            parent_logger,
        )
    }

    #[allow(clippy::result_large_err)]
    fn new_with_api_url(
        bucket: String,
        object: String,
        oauth_token: String,
        minimum_upload_chunk_size: usize,
        storage_api_base_url: Url,
        agent: RetryingAgent,
        parent_logger: &Logger,
    ) -> Result<StreamingTransferWriter, GcsError> {
        // Initiate the resumable, streaming upload.
        // https://cloud.google.com/storage/docs/performing-resumable-uploads#initiate-session
        let mut upload_url = gcp_upload_object_url(storage_api_base_url.as_ref(), &bucket)?;
        upload_url
            .query_pairs_mut()
            .append_pair("uploadType", "resumable")
            .append_pair("name", &object)
            .finish();

        debug!(parent_logger, "initiating multi-part upload");
        let request = agent.prepare_request(RequestParameters {
            url: upload_url,
            method: Method::Post,
            token_provider: Some(&StaticAccessTokenProvider::from(oauth_token)),
        })?;

        let http_response = agent
            .send_bytes(parent_logger, &request, "insert", &[])
            .map_err(|e| GcsError::Upload(e, bucket))?;

        // The upload session URI authenticates subsequent upload requests for
        // this upload, so we no longer need the impersonated service account's
        // Oauth token. Session URIs are valid for a week, which should be more
        // than enough for any upload we perform.
        // https://cloud.google.com/storage/docs/resumable-uploads#session-uris
        let upload_session_uri = http_response
            .header("Location")
            .ok_or(GcsError::NoLocationHeader)?;

        let logger = parent_logger.new(o!("upload_session_uri" => upload_session_uri.to_owned()));

        Ok(StreamingTransferWriter {
            minimum_upload_chunk_size,
            buffer: Vec::with_capacity(minimum_upload_chunk_size * 2),
            object_upload_position: 0,
            upload_session_uri: Url::parse(upload_session_uri)
                .map_err(|e| UrlParseError(e, upload_session_uri.to_owned()))?,
            agent,
            logger,
            is_finished: false,
            did_upload_chunk: false,
        })
    }

    #[allow(clippy::result_large_err)]
    fn upload_chunk(&mut self, last_chunk: bool) -> Result<(), GcsError> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        if !last_chunk && self.buffer.len() < self.minimum_upload_chunk_size {
            return Err(GcsError::ChunkUnderrun);
        }

        debug!(self.logger, "uploading object chunk");

        // When this is the last piece being uploaded, the Content-Range header
        // should include the total object size, but otherwise should have * to
        // indicate to GCS that there is an unknown further amount to come.
        // https://cloud.google.com/storage/docs/streaming#streaming_uploads
        let (body, content_range_header_total_length_field) =
            if last_chunk && self.buffer.len() < self.minimum_upload_chunk_size {
                (
                    self.buffer.as_ref(),
                    format!("{}", self.object_upload_position + self.buffer.len()),
                )
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

        let mut request = self
            .agent
            .prepare_anonymous_request(self.upload_session_uri.clone(), Method::Put);
        request = request.set("Content-Range", &content_range);

        let http_response = self
            .agent
            .send_bytes(&self.logger, &request, "insert-chunk", body)
            .map_err(|e| GcsError::ChunkUpload(e, self.upload_session_uri.clone()))?;

        self.did_upload_chunk = true;

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
            200 | 201 => Err(GcsError::EarlyChunkOk),
            308 if !http_response.has("Range") => {
                Err(GcsRangeHeaderError::Missing(http_response.into_string()).into())
            }
            308 => {
                let range_header = http_response.header("Range").unwrap();
                // The range header is like "bytes=0-222", and represents the
                // uploaded portion of the overall object, not the current chunk
                let end = range_header
                    .strip_prefix("bytes=0-")
                    .ok_or_else(|| GcsRangeHeaderError::BadPrefix(range_header.to_owned()))?
                    .parse::<usize>()
                    .map_err(|e| GcsRangeHeaderError::InvalidEnd(e, range_header.to_owned()))?;
                // end is usize and so parse would fail if the value in the
                // header was negative, but we still defend ourselves against
                // it being less than it was before this chunk was uploaded, or
                // being bigger than is possible given our position in the
                // overall object.
                if end < self.object_upload_position
                    || end > self.object_upload_position + body.len() - 1
                {
                    return Err(GcsRangeHeaderError::OutOfRangeEnd(range_header.to_owned()).into());
                }

                // If we have a little content left over, we can't just make
                // another request, because if there's too little of it, Google
                // will reject it. Instead, leave the portion of the chunk that
                // we didn't manage to upload back in self.buffer so it can be
                // handled by a subsequent call to upload_chunk.
                self.buffer = self.buffer.split_off(end + 1 - self.object_upload_position);
                self.object_upload_position = end + 1;
                Ok(())
            }
            status => Err(GcsError::ChunkUploadStatus(
                status,
                http_response.into_string(),
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
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
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
    fn complete_upload(&mut self) -> Result<(), TransportError> {
        while !self.buffer.is_empty() {
            self.upload_chunk(true)?;
        }

        self.is_finished = true;

        if !self.did_upload_chunk {
            // Nothing was ever written to this writer, so cancel the upload (to
            // clean up dangling uploads in GCS) and report success to the
            // caller. No object will be written to GCS.
            debug!(self.logger, "canceling empty upload");
            self.cancel_upload()?;
        }

        Ok(())
    }

    fn cancel_upload(&mut self) -> Result<(), TransportError> {
        self.is_finished = true;

        debug!(self.logger, "canceling upload");

        // https://cloud.google.com/storage/docs/performing-resumable-uploads#cancel-upload
        let request = self
            .agent
            .prepare_anonymous_request(self.upload_session_uri.clone(), Method::Delete);

        match self.agent.call(
            &self.logger,
            &request.set("Content-Length", "0"),
            "cancel-upload",
        ) {
            // GCS returns HTTP 499 on successful cancellation, and ureq returns
            // HTTP statuses in the 400 or 500 range as Err.
            Err(ureq::Error::Status(499, _)) => Ok(()),
            Ok(http_response) => Err(GcsError::CancelStatus(
                http_response.status(),
                http_response.into_string(),
            )
            .into()),
            Err(ureq::Error::Status(status, body)) => {
                Err(GcsError::CancelStatus(status, body.into_string()).into())
            }
            Err(e) => Err(GcsError::Cancel(e).into()),
        }
    }
}

impl Drop for StreamingTransferWriter {
    fn drop(&mut self) {
        if !self.is_finished {
            self.is_finished = true;
            if let Err(err) = self.cancel_upload() {
                warn!(self.logger, "Couldn't cancel upload: {}", err);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::setup_test_logging;
    use assert_matches::assert_matches;
    use mockito::{Matcher, Server};

    #[test]
    fn simple_upload() {
        let logger = setup_test_logging();
        let api_metrics = ApiClientMetricsCollector::new_with_metric_name("simple_upload").unwrap();
        let mut server = Server::new();

        let fake_upload_session_uri = format!("{}/fake-session-uri", server.url());
        let mocked_post = server
            .mock("POST", "/upload/storage/v1/b/fake-bucket/o/")
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
            Url::parse(&server.url()).expect("unable to parse mockito server url"),
            RetryingAgent::new("simple_upload", &api_metrics),
            &logger,
        )
        .unwrap();

        mocked_post.assert();

        let mocked_put = server
            .mock("PUT", "/fake-session-uri")
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
        let logger = setup_test_logging();
        let api_metrics =
            ApiClientMetricsCollector::new_with_metric_name("multi_chunk_upload").unwrap();
        let mut server = Server::new();

        let fake_upload_session_uri = format!("{}/fake-session-uri", server.url());
        let mocked_post = server
            .mock("POST", "/upload/storage/v1/b/fake-bucket/o/")
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
            Url::parse(&server.url()).expect("unable to parse mockito server url"),
            RetryingAgent::new("multi_chunk_upload", &api_metrics),
            &logger,
        )
        .unwrap();

        mocked_post.assert();

        let first_mocked_put = server
            .mock("PUT", "/fake-session-uri")
            .match_header("Content-Length", "4")
            .match_header("Content-Range", "bytes 0-3/*")
            .match_body("0123")
            .with_status(308)
            .with_header("Range", "bytes=0-3")
            .expect_at_most(1)
            .create();

        let second_mocked_put = server
            .mock("PUT", "/fake-session-uri")
            .match_header("Content-Length", "4")
            .match_header("Content-Range", "bytes 4-7/*")
            .match_body("4567")
            .with_status(308)
            .with_header("Range", "bytes=0-6")
            .expect_at_most(1)
            .create();

        let final_mocked_put = server
            .mock("PUT", "/fake-session-uri")
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

    #[test]
    fn empty_multi_part_upload() {
        let logger = setup_test_logging();
        let api_metrics =
            ApiClientMetricsCollector::new_with_metric_name("empty_multi_chunk_upload").unwrap();
        let mut server = Server::new();

        let fake_upload_session_uri = format!("{}/fake-session-uri", server.url());
        let mocked_post = server
            .mock("POST", "/upload/storage/v1/b/fake-bucket/o/")
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
            8_388_608,
            Url::parse(&server.url()).expect("unable to parse mockito server url"),
            RetryingAgent::new("multi_chunk_upload", &api_metrics),
            &logger,
        )
        .unwrap();

        mocked_post.assert();

        let mocked_cancel = server
            .mock("DELETE", "/fake-session-uri")
            .match_header("Content-Length", "0")
            .with_status(499)
            .expect_at_most(1)
            .create();

        writer.complete_upload().unwrap();

        mocked_cancel.assert();
    }

    #[test]
    fn upload_cancel_failure() {
        let logger = setup_test_logging();
        let api_metrics =
            ApiClientMetricsCollector::new_with_metric_name("upload_cancel_failure").unwrap();
        let mut server = Server::new();

        let fake_upload_session_uri = format!("{}/fake-session-uri", server.url());
        let mocked_post = server
            .mock("POST", "/upload/storage/v1/b/fake-bucket/o/")
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
            8_388_608,
            Url::parse(&server.url()).expect("unable to parse mockito server url"),
            RetryingAgent::new("multi_chunk_upload", &api_metrics),
            &logger,
        )
        .unwrap();

        mocked_post.assert();

        // Respond to cancel with HTTP status that ureq represents as an error
        // but is not 499
        let mocked_cancel = server
            .mock("DELETE", "/fake-session-uri")
            .match_header("Content-Length", "0")
            .with_status(404)
            .with_body("error body")
            .expect_at_most(1)
            .create();

        assert_matches!(
            writer.cancel_upload().unwrap_err(),
            TransportError::Gcs(GcsError::CancelStatus(404, body)) => {
                assert_eq!(body.unwrap(), "error body".to_string());
            }
        );

        mocked_cancel.assert();

        // Respond to cancel with HTTP status that indicates cancel failed but
        // is not handled as an error by ureq
        let mocked_cancel = server
            .mock("DELETE", "/fake-session-uri")
            .match_header("Content-Length", "0")
            .with_status(200)
            .with_body("error body")
            .expect_at_most(1)
            .create();

        assert_matches!(
            writer.cancel_upload().unwrap_err(),
            TransportError::Gcs(GcsError::CancelStatus(200, body)) => {
                assert_eq!(body.unwrap(), "error body".to_string());
            }
        );

        mocked_cancel.assert();
    }
}

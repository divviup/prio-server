use crate::aws_credentials;
use crate::metrics::ApiClientMetricsCollector;
use crate::{
    aws_credentials::retry_request,
    config::S3Path,
    logging::event,
    transport::{Transport, TransportWriter},
};
use bytes::Bytes;
use derivative::Derivative;
use futures::future::FutureExt;
use http::{HeaderMap, StatusCode};
use hyper_rustls::HttpsConnectorBuilder;
use rusoto_core::{request::BufferedHttpResponse, ByteStream, RusotoError};
use rusoto_s3::{
    AbortMultipartUploadError, AbortMultipartUploadRequest, CompleteMultipartUploadError,
    CompleteMultipartUploadRequest, CompletedMultipartUpload, CompletedPart,
    CreateMultipartUploadError, CreateMultipartUploadRequest, GetObjectError, GetObjectRequest,
    S3Client, UploadPartError, UploadPartRequest, S3,
};
use slog::{debug, info, o, warn, Logger};
use std::{
    io::{Read, Write},
    mem,
    pin::Pin,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    runtime::Handle,
};
use uuid::Uuid;

use super::TransportError;

/// Errors encountered when using S3 as a batch transport.
#[derive(Debug, thiserror::Error)]
pub enum S3Error {
    #[error("error getting S3 object: {0}")]
    GetObject(RusotoError<GetObjectError>),
    #[error("no body in GetObjectResponse")]
    GetObjectNoBody,
    #[error("error creating multipart upload to s3://{1}, {0}")]
    CreateMultipartUpload(RusotoError<CreateMultipartUploadError>, String),
    #[error("no upload ID in CreateMultipartUploadResponse")]
    MissingUploadId,
    #[error("error completing upload: {0}")]
    CompleteMultipartUpload(RusotoError<CompleteMultipartUploadError>),
    #[error(transparent)]
    AbortMultipartUpload(RusotoError<AbortMultipartUploadError>),
    #[error("failed to upload part: {0}")]
    UploadPart(RusotoError<UploadPartError>),
    #[error("no ETag in UploadPartOutput")]
    MissingETag,
}

/// Implementation of Transport that reads and writes objects from Amazon S3.
#[derive(Clone, Derivative)]
#[derivative(Debug)]
pub struct S3Transport {
    path: S3Path,
    runtime_handle: Handle,
    #[derivative(Debug = "ignore")]
    credentials_provider: aws_credentials::Provider,
    logger: Logger,
    api_metrics: ApiClientMetricsCollector,
}

impl S3Transport {
    pub fn new(
        path: S3Path,
        credentials_provider: aws_credentials::Provider,
        runtime_handle: &Handle,
        parent_logger: &Logger,
        api_metrics: &ApiClientMetricsCollector,
    ) -> Self {
        let logger = parent_logger.new(o!(
            event::STORAGE_PATH => path.to_string(),
            event::IDENTITY => credentials_provider.to_string(),
        ));
        Self {
            path: path.ensure_directory_prefix(),
            runtime_handle: runtime_handle.clone(),
            credentials_provider,
            logger,
            api_metrics: api_metrics.clone(),
        }
    }

    /// Construct an S3Client for this transport
    fn client(&self) -> Result<S3Client, S3Error> {
        // Rusoto uses Hyper which uses connection pools. The default
        // timeout for those connections is 90 seconds[1]. Amazon S3's
        // API closes idle client connections after 20 seconds[2]. If we
        // use a default client via S3Client::new, this mismatch causes
        // uploads to fail when Hyper tries to re-use a connection that
        // has been idle too long. Until this is fixed in Rusoto[3], we
        // construct our own HTTP request dispatcher whose underlying
        // hyper::Client is configured to timeout idle connections after
        // 10 seconds.
        //
        // [1]: https://docs.rs/hyper/0.13.8/hyper/client/struct.Builder.html#method.pool_idle_timeout
        // [2]: https://aws.amazon.com/premiumsupport/knowledge-center/s3-socket-connection-timeout-error/
        // [3]: https://github.com/rusoto/rusoto/issues/1686
        let mut builder = hyper::Client::builder();
        builder.pool_idle_timeout(Duration::from_secs(10));
        let connector = HttpsConnectorBuilder::new()
            .with_native_roots()
            // We connect over HTTP in tests and so must allow either protocol
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .build();
        let http_client = rusoto_core::HttpClient::from_builder(builder, connector);

        Ok(S3Client::new_with(
            http_client,
            self.credentials_provider.clone(),
            self.path.region.clone(),
        ))
    }

    fn service() -> &'static str {
        "s3.amazonaws.com"
    }
}

impl Transport for S3Transport {
    fn path(&self) -> String {
        self.path.to_string()
    }

    fn get(&self, key: &str, trace_id: &Uuid) -> Result<Box<dyn Read>, TransportError> {
        let logger = self.logger.new(o!(
            event::STORAGE_KEY => key.to_owned(),
            event::TRACE_ID => trace_id.to_string(),
            event::ACTION => "get s3 object",
        ));
        info!(logger, "get");
        let client = self.client()?;

        let get_output = retry_request(
            &logger,
            &self.api_metrics,
            Self::service(),
            "GetObject",
            &self.runtime_handle,
            || {
                client.get_object(GetObjectRequest {
                    bucket: self.path.bucket.to_owned(),
                    key: [&self.path.key, key].concat(),
                    ..Default::default()
                })
            },
        )
        .map_err(|err| {
            if matches!(err, RusotoError::Service(GetObjectError::NoSuchKey(_))) {
                return TransportError::ObjectNotFoundError(
                    key.to_owned(),
                    anyhow::Error::from(err),
                );
            }
            S3Error::GetObject(err).into()
        })?;

        let body = get_output.body.ok_or(S3Error::GetObjectNoBody)?;

        Ok(Box::new(StreamingBodyReader::new(
            body,
            &self.runtime_handle,
        )))
    }

    fn put(&self, key: &str, trace_id: &Uuid) -> Result<Box<dyn TransportWriter>, TransportError> {
        let logger = self.logger.new(o!(
            event::STORAGE_KEY => key.to_owned(),
            event::TRACE_ID => trace_id.to_string(),
        ));
        info!(logger, "put");
        let writer = MultipartUploadWriter::new(
            self.path.bucket.to_owned(),
            format!("{}{}", &self.path.key, key),
            // Set buffer size to 5 MB, which is the minimum required by Amazon
            // https://docs.aws.amazon.com/AmazonS3/latest/dev/qfacts.html
            5_242_880,
            self.client()?,
            &self.runtime_handle,
            &logger,
            &self.api_metrics,
        )?;
        Ok(Box::new(writer))
    }
}

/// StreamingBodyReader is an std::io::Read implementation which reads from the
/// tokio::io::AsyncRead inside the StreamingBody in a Rusoto API request
/// response.
struct StreamingBodyReader {
    body_reader: Pin<Box<dyn AsyncRead + Send>>,
    runtime_handle: Handle,
}

impl StreamingBodyReader {
    fn new(body: ByteStream, runtime_handle: &Handle) -> StreamingBodyReader {
        StreamingBodyReader {
            body_reader: Box::pin(body.into_async_read()),
            runtime_handle: runtime_handle.clone(),
        }
    }
}

impl Read for StreamingBodyReader {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
        self.runtime_handle.block_on(self.body_reader.read(buf))
    }
}

/// MultipartUploadWriter is a TransportWriter implementation that uses AWS S3
/// multi part uploads to permit streaming of objects into S3. On creation, it
/// initiates a multipart upload. It maintains a memory buffer into which it
/// writes the buffers passed by std::io::Write::write, and when there is more
/// than buffer_capacity bytes in it, performs an UploadPart call. On
/// TransportWrite::complete_upload, it calls CompleteMultipartUpload to finish
/// the upload. If any part of the upload fails, it cleans up by calling
/// AbortMultipartUpload as otherwise we would be billed for partial uploads.
/// https://docs.aws.amazon.com/AmazonS3/latest/dev/uploadobjusingmpu.html
#[derive(Derivative)]
#[derivative(Debug)]
struct MultipartUploadWriter {
    runtime_handle: Handle,
    #[derivative(Debug = "ignore")]
    client: S3Client,
    bucket: String,
    key: String,
    upload_id: String,
    completed_parts: Vec<CompletedPart>,
    minimum_upload_part_size: usize,
    buffer: Vec<u8>,
    logger: Logger,
    api_metrics: ApiClientMetricsCollector,
    is_finished: bool,
}

impl MultipartUploadWriter {
    /// Creates a new MultipartUploadWriter with the provided parameters. A real
    /// instance of this will fail if buffer_capacity is less than 5 MB, but we
    /// allow smaller values for testing purposes. Larger values are also
    /// acceptable but smaller values prevent excessive memory usage.
    fn new(
        bucket: String,
        key: String,
        minimum_upload_part_size: usize,
        client: S3Client,
        runtime_handle: &Handle,
        parent_logger: &Logger,
        api_metrics: &ApiClientMetricsCollector,
    ) -> Result<MultipartUploadWriter, S3Error> {
        let logger = parent_logger.new(o!());
        let runtime_handle = runtime_handle.clone();

        // We use the "bucket-owner-full-control" canned ACL to ensure that
        // objects we send to peers will be owned by them.
        // https://docs.aws.amazon.com/AmazonS3/latest/dev/about-object-ownership.html
        let create_output = retry_request(
            &logger.new(o!(event::ACTION => "create multipart upload")),
            api_metrics,
            S3Transport::service(),
            "CreateMultipartUpload",
            &runtime_handle,
            || {
                client.create_multipart_upload(CreateMultipartUploadRequest {
                    bucket: bucket.to_string(),
                    key: key.to_string(),
                    acl: Some("bucket-owner-full-control".to_owned()),
                    ..Default::default()
                })
            },
        )
        .map_err(|e| S3Error::CreateMultipartUpload(e, bucket.clone()))?;

        Ok(MultipartUploadWriter {
            runtime_handle,
            client,
            bucket,
            key,
            upload_id: create_output.upload_id.ok_or(S3Error::MissingUploadId)?,
            completed_parts: Vec::new(),
            // Upload parts must be at least buffer_capacity, but it's fine if
            // they're bigger, so overprovision the buffer to make it unlikely
            // that the caller will overflow it.
            minimum_upload_part_size,
            buffer: Vec::with_capacity(minimum_upload_part_size * 2),
            logger,
            api_metrics: api_metrics.clone(),
            is_finished: false,
        })
    }

    /// Upload content in internal buffer, if any, to S3 in an UploadPart call.
    fn upload_part(&mut self) -> Result<(), TransportError> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        let part_number = (self.completed_parts.len() + 1) as i64;

        // Move internal buffer into request object and replace it with a new,
        // empty buffer. UploadPartRequest assumes ownership of the request body
        // so sadly we have to clone the buffer in order to be able to retry.
        let body = mem::replace(
            &mut self.buffer,
            Vec::with_capacity(self.minimum_upload_part_size * 2),
        );

        let upload_output = retry_request(
            &self.logger.new(o!(event::ACTION => "upload part")),
            &self.api_metrics,
            S3Transport::service(),
            "UploadPart",
            &self.runtime_handle,
            || {
                self.client.upload_part(UploadPartRequest {
                    bucket: self.bucket.to_string(),
                    key: self.key.to_string(),
                    upload_id: self.upload_id.clone(),
                    part_number,
                    body: Some(body.clone().into()),
                    ..Default::default()
                })
            },
        )
        .map_err(|e| {
            // Clean up botched uploads
            if let Err(cancel) = self.cancel_upload() {
                return TransportError::Cancellation {
                    original: Box::new(S3Error::UploadPart(e).into()),
                    cancellation: Box::new(cancel),
                };
            }
            S3Error::UploadPart(e).into()
        })?;

        let e_tag = upload_output
            .e_tag
            .ok_or(S3Error::MissingETag)
            .map_err(|e| {
                if let Err(cancel) = self.cancel_upload() {
                    return TransportError::Cancellation {
                        original: Box::new(e.into()),
                        cancellation: Box::new(cancel),
                    };
                }
                e.into()
            })?;

        let completed_part = CompletedPart {
            e_tag: Some(e_tag),
            part_number: Some(part_number),
        };
        self.completed_parts.push(completed_part);
        Ok(())
    }
}

impl Write for MultipartUploadWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        debug!(self.logger, "uploading part");
        // Write into memory buffer, and upload to S3 if we have accumulated
        // enough content.
        self.buffer.extend_from_slice(buf);
        if self.buffer.len() >= self.minimum_upload_part_size {
            self.upload_part()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl TransportWriter for MultipartUploadWriter {
    fn complete_upload(&mut self) -> Result<(), TransportError> {
        // Write last part, if any
        self.upload_part()?;

        if self.completed_parts.is_empty() {
            info!(self.logger, "canceling empty upload");
            // Nothing was ever written to this writer, so cancel the upload (to
            // clean up dangling uploads in S3) and report success to the
            // caller. No object will be written to S3.
            return self.cancel_upload();
        }

        // Ignore output for now, but we might want the e_tag to check the
        // digest
        let completed_parts = mem::take(&mut self.completed_parts);
        retry_request(
            &self.logger.new(o!(event::ACTION => "complete upload")),
            &self.api_metrics,
            S3Transport::service(),
            "CompleteMultipartUpload",
            &self.runtime_handle,
            || {
                self.client
                    .complete_multipart_upload(CompleteMultipartUploadRequest {
                        bucket: self.bucket.to_string(),
                        key: self.key.to_string(),
                        upload_id: self.upload_id.clone(),
                        multipart_upload: Some(CompletedMultipartUpload {
                            parts: Some(completed_parts.clone()),
                        }),
                        ..Default::default()
                    })
                    .map(|rslt| {
                        if let Ok(output) = rslt {
                            if output.location.is_none()
                                && output.e_tag.is_none()
                                && output.bucket.is_none()
                                && output.key.is_none()
                            {
                                // Due to an oddity in S3's CompleteMultipartUpload API, some
                                // failed uploads can cause complete_multipart_upload to return
                                // Ok(). To work around this, we check if key fields of the
                                // output are None, and if so, synthesize a RusotoError::Unknown
                                // wrapping an HTTP response with status code 500 so that our
                                // retry logic will gracefully try again.
                                // https://github.com/rusoto/rusoto/issues/1936
                                return Err(RusotoError::<CompleteMultipartUploadError>::Unknown(
                                    BufferedHttpResponse {
                                        status: StatusCode::from_u16(500).unwrap(),
                                        headers: HeaderMap::with_capacity(0),
                                        body: Bytes::from_static(b"synthetic HTTP 500 response"),
                                    },
                                ));
                            }
                            return Ok(output);
                        }
                        rslt
                    })
            },
        )
        .map_err(|e| TransportError::S3(S3Error::CompleteMultipartUpload(e)))?;

        self.is_finished = true;
        Ok(())
    }

    fn cancel_upload(&mut self) -> Result<(), TransportError> {
        self.is_finished = true;

        debug!(self.logger, "canceling upload");
        // There's nothing useful in the output so discard it
        self.runtime_handle
            .block_on(
                self.client
                    .abort_multipart_upload(AbortMultipartUploadRequest {
                        bucket: self.bucket.to_string(),
                        key: self.key.to_string(),
                        upload_id: self.upload_id.clone(),
                        ..Default::default()
                    }),
            )
            .map_err(|e| TransportError::S3(S3Error::AbortMultipartUpload(e)))?;
        Ok(())
    }
}

impl Drop for MultipartUploadWriter {
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
    use crate::{
        logging::setup_test_logging,
        metrics::ApiClientMetricsCollector,
        test_utils::{test_runtime, DEFAULT_TRACE_ID},
    };
    use mockito::{mock, Matcher, Mock};
    use regex;
    use rusoto_core::{request::HttpClient, Region};
    use std::io::Read;

    // The testing strategy is to wire Rusoto to talk to a Mockito-managed HTTP
    // endpoint. Mockito's standard mock/matching functionality is used to
    // play canned API responses to the expected requests, which allows e.g.
    // checking that we call AbortMultipartUpload if an UploadPart call fails.
    // The `mock_*_request` functions below are used to create a mock which
    // matches the expected HTTP method & URL parameters against the Amazon API
    // specification to figure out what API call we are looking at. (Note that
    // these functions do not call `Mock::create`, allowing further
    // modification of the mock by the caller; but the caller is also
    // responsible for eventually calling `Mock::create`.)

    const TEST_BUCKET: &str = "fake-bucket";
    const TEST_KEY: &str = "fake-key";
    const TEST_UPLOAD_ID: &str = "fake-upload-id";
    const TEST_ETAG: &str = "fake-etag-1";
    const TEST_ETAG_2: &str = "fake-etag-2";
    const TEST_REGION: &str = "fake-region";

    // has_query_parameter returns a Matcher which matches if a query string
    // includes the given key (with any value).
    fn has_query_parameter(param: &str) -> Matcher {
        let regex_text = format!("(^|&){}(=|&|$)", regex::escape(param));
        Matcher::Regex(regex_text)
    }

    fn mock_create_multipart_upload_request(bucket: &str, key: &str, upload_id: &str) -> Mock {
        // https://docs.aws.amazon.com/AmazonS3/latest/API/API_CreateMultipartUpload.html
        let path = format!("/{}/{}", bucket, key);
        let response_body = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<InitiateMultipartUploadResult>
   <Bucket>{}</Bucket>
   <Key>{}</Key>
   <UploadId>{}</UploadId>
</InitiateMultipartUploadResult>"#,
            bucket, key, upload_id
        );
        mock("POST", Matcher::Exact(path))
            .match_query(has_query_parameter("uploads"))
            .with_body(response_body)
    }

    // Unfortunately, headers can't be removed from mocks, so we need to provide mocks with a
    // minimal number of headers.
    fn mock_upload_part_request_without_etag(
        bucket: &str,
        key: &str,
        upload_id: &str,
        part_number: u64,
    ) -> Mock {
        // https://docs.aws.amazon.com/AmazonS3/latest/API/API_UploadPart.html
        let path = format!("/{}/{}", bucket, key);
        mock("PUT", Matcher::Exact(path)).match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("partNumber".to_string(), part_number.to_string()),
            Matcher::UrlEncoded("uploadId".to_string(), upload_id.to_string()),
        ]))
    }

    fn mock_upload_part_request(
        bucket: &str,
        key: &str,
        upload_id: &str,
        part_number: u64,
        etag: &str,
    ) -> Mock {
        // https://docs.aws.amazon.com/AmazonS3/latest/API/API_UploadPart.html
        mock_upload_part_request_without_etag(bucket, key, upload_id, part_number)
            .with_header("ETag", etag)
    }

    fn mock_abort_multipart_upload_request(bucket: &str, key: &str, upload_id: &str) -> Mock {
        // https://docs.aws.amazon.com/AmazonS3/latest/API/API_AbortMultipartUpload.html
        let path = format!("/{}/{}", bucket, key);
        mock("DELETE", Matcher::Exact(path))
            .match_query(Matcher::UrlEncoded(
                "uploadId".to_string(),
                upload_id.to_string(),
            ))
            .with_status(204)
    }

    fn mock_complete_multipart_upload_request(
        bucket: &str,
        key: &str,
        upload_id: &str,
        part_etags: Vec<&str>,
    ) -> Mock {
        // https://docs.aws.amazon.com/AmazonS3/latest/API/API_CompleteMultipartUpload.html
        let path = format!("/{}/{}", bucket, key);
        let response_body = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<CompleteMultipartUploadResult>
   <Location>fake-final-location</Location>
   <Bucket>{}</Bucket>
   <Key>{}</Key>
   <ETag>fake-final-etag</ETag>
</CompleteMultipartUploadResult>"#,
            bucket, key
        );
        let body_matchers: Vec<Matcher> = part_etags
            .into_iter()
            .map(|etag| Matcher::Regex(regex::escape(etag)))
            .collect();
        mock("POST", Matcher::Exact(path))
            .match_query(Matcher::UrlEncoded(
                "uploadId".to_string(),
                upload_id.to_string(),
            ))
            .match_body(Matcher::AllOf(body_matchers))
            .with_body(response_body)
    }

    fn mock_get_object_request(bucket: &str, key: &str) -> Mock {
        let path = format!("/{}/{}", bucket, key);
        mock("GET", Matcher::Exact(path)).match_query(Matcher::Missing)
    }

    #[test]
    fn multipart_upload_create_fails() {
        let logger = setup_test_logging();
        let runtime = test_runtime();
        let api_metrics = ApiClientMetricsCollector::new_with_metric_name(
            "mock_complete_multipart_upload_request",
        )
        .unwrap();

        let mocked_upload =
            mock_create_multipart_upload_request(TEST_BUCKET, TEST_KEY, TEST_UPLOAD_ID)
                .with_status(401)
                .with_body("")
                .create();

        let err = MultipartUploadWriter::new(
            String::from(TEST_BUCKET),
            String::from(TEST_KEY),
            50,
            S3Client::new_with(
                HttpClient::from_connector(
                    HttpsConnectorBuilder::new()
                        .with_native_roots()
                        .https_or_http()
                        .enable_http1()
                        .build(),
                ),
                aws_credentials::Provider::new_mock(&logger, &api_metrics),
                Region::Custom {
                    name: TEST_REGION.into(),
                    endpoint: mockito::server_url(),
                },
            ),
            runtime.handle(),
            &logger,
            &api_metrics,
        )
        .expect_err("expected error");
        assert!(
            matches!(err, S3Error::CreateMultipartUpload(_, _)),
            "found unexpected error {:?}",
            err
        );

        mocked_upload.assert();
    }

    #[test]
    fn multipart_upload_create_no_upload_id() {
        let logger = setup_test_logging();
        let runtime = test_runtime();
        let api_metrics =
            ApiClientMetricsCollector::new_with_metric_name("multipart_upload_create_no_upload_id")
                .unwrap();

        let mocked_upload =
            mock_create_multipart_upload_request(TEST_BUCKET, TEST_KEY, TEST_UPLOAD_ID)
                .with_body(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<InitiateMultipartUploadResult>
   <Bucket>fake-bucket</Bucket>
   <Key>fake-key</Key>
</InitiateMultipartUploadResult>"#,
                )
                .create();

        // Response body format from
        // https://docs.aws.amazon.com/AmazonS3/latest/API/API_Operations_Amazon_Simple_Storage_Service.html
        MultipartUploadWriter::new(
            String::from(TEST_BUCKET),
            String::from(TEST_KEY),
            50,
            S3Client::new_with(
                HttpClient::from_connector(
                    HttpsConnectorBuilder::new()
                        .with_native_roots()
                        .https_or_http()
                        .enable_http1()
                        .build(),
                ),
                aws_credentials::Provider::new_mock(&logger, &api_metrics),
                Region::Custom {
                    name: TEST_REGION.into(),
                    endpoint: mockito::server_url(),
                },
            ),
            runtime.handle(),
            &logger,
            &api_metrics,
        )
        .expect_err("expected error");

        mocked_upload.assert();
    }

    #[test]
    fn multipart_upload_fails_with_http_200_ok() {
        // This test exercises our workaround for a known issue in Rusoto and
        // should be removed if/when the issue is resolved.
        // https://github.com/rusoto/rusoto/issues/1936
        let logger = setup_test_logging();
        let runtime = test_runtime();
        let api_metrics = ApiClientMetricsCollector::new_with_metric_name(
            "multipart_upload_fails_with_http_200_ok",
        )
        .unwrap();

        // Response body format from
        // https://docs.aws.amazon.com/AmazonS3/latest/API/API_Operations_Amazon_Simple_Storage_Service.html
        let mocks: Vec<Mock> = vec![
            mock_create_multipart_upload_request(TEST_BUCKET, TEST_KEY, TEST_UPLOAD_ID),
            mock_upload_part_request(TEST_BUCKET, TEST_KEY, TEST_UPLOAD_ID, 1, TEST_ETAG),
            // HTTP 200 response to CompleteMultipartUpload that contains an
            // error. Should cause us to retry.
            // https://docs.aws.amazon.com/AmazonS3/latest/API/API_CompleteMultipartUpload.html#API_CompleteMultipartUpload_Examples
            mock_complete_multipart_upload_request(
                TEST_BUCKET,
                TEST_KEY,
                TEST_UPLOAD_ID,
                vec![TEST_ETAG],
            )
            .with_body(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<Error>
    <Code>InternalError</Code>
    <Message>We encountered an internal error. Please try again.</Message>
    <RequestId>656c76696e6727732072657175657374</RequestId>
    <HostId>Uuag1LuByRx9e6j5Onimru9pO4ZVKnJ2Qz7/C1NPcfTWAtRPfTaOFg==</HostId>
</Error>"#,
            ),
            mock_complete_multipart_upload_request(
                TEST_BUCKET,
                TEST_KEY,
                TEST_UPLOAD_ID,
                vec![TEST_ETAG],
            ),
        ]
        .into_iter()
        .map(Mock::create)
        .collect();

        let mut writer = MultipartUploadWriter::new(
            String::from(TEST_BUCKET),
            String::from(TEST_KEY),
            50,
            S3Client::new_with(
                HttpClient::from_connector(
                    HttpsConnectorBuilder::new()
                        .with_native_roots()
                        .https_or_http()
                        .enable_http1()
                        .build(),
                ),
                aws_credentials::Provider::new_mock(&logger, &api_metrics),
                Region::Custom {
                    name: TEST_REGION.into(),
                    endpoint: mockito::server_url(),
                },
            ),
            runtime.handle(),
            &logger,
            &api_metrics,
        )
        .unwrap();

        writer.write_all(&[0; 50]).unwrap();
        writer.complete_upload().unwrap();

        for mock in &mocks {
            mock.assert();
        }
    }

    #[test]
    fn multipart_upload() {
        let logger = setup_test_logging();
        let runtime = test_runtime();
        let api_metrics =
            ApiClientMetricsCollector::new_with_metric_name("multipart_upload").unwrap();

        // Response body format from
        // https://docs.aws.amazon.com/AmazonS3/latest/API/API_Operations_Amazon_Simple_Storage_Service.html
        let mocks: Vec<Mock> = vec![
            mock_create_multipart_upload_request(TEST_BUCKET, TEST_KEY, TEST_UPLOAD_ID),
            // 500 status will cause a retry.
            mock_upload_part_request_without_etag(TEST_BUCKET, TEST_KEY, TEST_UPLOAD_ID, 1)
                .with_status(500),
            // HTTP 401 will cause failure.
            mock_upload_part_request_without_etag(TEST_BUCKET, TEST_KEY, TEST_UPLOAD_ID, 1)
                .with_status(401),
            // Expected because of previous UploadPart failure.
            mock_abort_multipart_upload_request(TEST_BUCKET, TEST_KEY, TEST_UPLOAD_ID),
            // HTTP 200 but no ETag header will cause failure.
            mock_upload_part_request_without_etag(TEST_BUCKET, TEST_KEY, TEST_UPLOAD_ID, 1),
            // Expected because of previous UploadPart failure.
            mock_abort_multipart_upload_request(TEST_BUCKET, TEST_KEY, TEST_UPLOAD_ID),
            // Well-formed response to UploadPart.
            mock_upload_part_request(TEST_BUCKET, TEST_KEY, TEST_UPLOAD_ID, 1, TEST_ETAG),
            mock_upload_part_request(TEST_BUCKET, TEST_KEY, TEST_UPLOAD_ID, 2, TEST_ETAG_2),
            // Well-formed response to CompleteMultipartUpload.
            mock_complete_multipart_upload_request(
                TEST_BUCKET,
                TEST_KEY,
                TEST_UPLOAD_ID,
                vec![TEST_ETAG, TEST_ETAG_2],
            ),
            // Expected because of final complete_upload call
            mock_abort_multipart_upload_request(TEST_BUCKET, TEST_KEY, TEST_UPLOAD_ID),
        ]
        .into_iter()
        .map(Mock::create)
        .collect();

        let mut writer = MultipartUploadWriter::new(
            String::from(TEST_BUCKET),
            String::from(TEST_KEY),
            50,
            S3Client::new_with(
                HttpClient::from_connector(
                    HttpsConnectorBuilder::new()
                        .with_native_roots()
                        .https_or_http()
                        .enable_http1()
                        .build(),
                ),
                aws_credentials::Provider::new_mock(&logger, &api_metrics),
                Region::Custom {
                    name: TEST_REGION.into(),
                    endpoint: mockito::server_url(),
                },
            ),
            runtime.handle(),
            &logger,
            &api_metrics,
        )
        .expect("failed to create multipart upload writer");

        // First write will fail due to HTTP 401
        writer.write_all(&[0; 51]).unwrap_err();
        // Second write will fail because response is missing ETag
        writer.write_all(&[0; 51]).unwrap_err();
        // Third write will work
        writer.write_all(&[0; 51]).unwrap();
        // This write will put some content in the buffer, but not enough to
        // cause an UploadPart
        writer.write_all(&[0; 25]).unwrap();
        // Flush will cause writer to UploadPart the last part and then complete
        // upload
        writer.complete_upload().unwrap();
        // Further call to complete_upload fails because there are no uploaded
        // parts
        writer.complete_upload().unwrap();

        for mock in &mocks {
            mock.assert();
        }
    }

    #[test]
    fn roundtrip_s3_transport_failed_get_object() {
        let logger = setup_test_logging();
        let runtime = test_runtime();
        let api_metrics = ApiClientMetricsCollector::new_with_metric_name(
            "roundtrip_s3_transport_failed_get_object",
        )
        .unwrap();

        let s3_path = S3Path {
            region: Region::Custom {
                name: TEST_REGION.into(),
                endpoint: mockito::server_url(),
            },
            bucket: TEST_BUCKET.into(),
            key: "".into(),
        };

        let mocked_get_object = mock_get_object_request(TEST_BUCKET, TEST_KEY)
            .with_status(404)
            .create();

        let transport = S3Transport::new(
            s3_path,
            aws_credentials::Provider::new_mock(&logger, &api_metrics),
            runtime.handle(),
            &logger,
            &api_metrics,
        );

        let ret = transport.get(TEST_KEY, &DEFAULT_TRACE_ID);
        assert!(ret.is_err(), "unexpected return value {:?}", ret.err());

        mocked_get_object.assert();
    }

    #[test]
    fn roundtrip_s3_transport_successful_get_object() {
        let logger = setup_test_logging();
        let runtime = test_runtime();
        let api_metrics = ApiClientMetricsCollector::new_with_metric_name(
            "roundtrip_s3_transport_successful_get_object",
        )
        .unwrap();

        let s3_path = S3Path {
            region: Region::Custom {
                name: TEST_REGION.into(),
                endpoint: mockito::server_url(),
            },
            bucket: TEST_BUCKET.into(),
            key: "".into(),
        };

        let mocked_get_object = mock_get_object_request(TEST_BUCKET, TEST_KEY)
            .with_body("fake-content")
            .create();

        let transport = S3Transport::new(
            s3_path,
            aws_credentials::Provider::new_mock(&logger, &api_metrics),
            runtime.handle(),
            &logger,
            &api_metrics,
        );

        let mut reader = transport
            .get(TEST_KEY, &DEFAULT_TRACE_ID)
            .expect("unexpected error getting reader");
        let mut content = Vec::new();
        reader.read_to_end(&mut content).expect("failed to read");
        assert_eq!(Vec::from("fake-content"), content);

        mocked_get_object.assert();
    }

    #[test]
    fn roundtrip_s3_transport_create_multipart_upload() {
        let logger = setup_test_logging();
        let runtime = test_runtime();
        let api_metrics = ApiClientMetricsCollector::new_with_metric_name(
            "roundtrip_s3_transport_create_multipart_upload",
        )
        .unwrap();

        let s3_path = S3Path {
            region: Region::Custom {
                name: TEST_REGION.into(),
                endpoint: mockito::server_url(),
            },
            bucket: TEST_BUCKET.into(),
            key: "".into(),
        };

        let mocks: Vec<Mock> = vec![
            mock_create_multipart_upload_request(TEST_BUCKET, TEST_KEY, TEST_UPLOAD_ID),
            mock_upload_part_request(TEST_BUCKET, TEST_KEY, TEST_UPLOAD_ID, 1, TEST_ETAG),
            mock_complete_multipart_upload_request(
                TEST_BUCKET,
                TEST_KEY,
                TEST_UPLOAD_ID,
                vec![TEST_ETAG],
            ),
            // Expected because of cancel_upload call.
            mock_abort_multipart_upload_request(TEST_BUCKET, TEST_KEY, TEST_UPLOAD_ID),
        ]
        .into_iter()
        .map(Mock::create)
        .collect();

        let transport = S3Transport::new(
            s3_path,
            aws_credentials::Provider::new_mock(&logger, &api_metrics),
            runtime.handle(),
            &logger,
            &api_metrics,
        );

        let mut writer = transport.put(TEST_KEY, &DEFAULT_TRACE_ID).unwrap();
        writer.write_all(b"fake-content").unwrap();
        writer.complete_upload().unwrap();
        writer.cancel_upload().unwrap();

        for mock in &mocks {
            mock.assert()
        }
    }

    #[test]
    fn s3_transport_empty_multipart_upload() {
        let logger = setup_test_logging();
        let runtime = test_runtime();
        let api_metrics =
            ApiClientMetricsCollector::new_with_metric_name("s3_transport_empty_multipart_upload")
                .unwrap();

        let s3_path = S3Path {
            region: Region::Custom {
                name: TEST_REGION.into(),
                endpoint: mockito::server_url(),
            },
            bucket: TEST_BUCKET.into(),
            key: "".into(),
        };

        let mocks: Vec<Mock> = vec![
            mock_create_multipart_upload_request(TEST_BUCKET, TEST_KEY, TEST_UPLOAD_ID),
            // Expected because of completing upload immediately after put
            mock_abort_multipart_upload_request(TEST_BUCKET, TEST_KEY, TEST_UPLOAD_ID),
        ]
        .into_iter()
        .map(Mock::create)
        .collect();

        let transport = S3Transport::new(
            s3_path,
            aws_credentials::Provider::new_mock(&logger, &api_metrics),
            runtime.handle(),
            &logger,
            &api_metrics,
        );

        let mut writer = transport.put(TEST_KEY, &DEFAULT_TRACE_ID).unwrap();
        writer.complete_upload().unwrap();

        for mock in &mocks {
            mock.assert();
        }
    }
}

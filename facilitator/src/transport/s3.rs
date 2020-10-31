use crate::{
    config::{Identity, S3Path},
    transport::{Transport, TransportWriter},
    Error,
};
use anyhow::{Context, Result};
use derivative::Derivative;
use hyper_rustls::HttpsConnector;
use log::debug;
use rusoto_core::{
    credential::{
        AutoRefreshingProvider, CredentialsError, DefaultCredentialsProvider, Secret, Variable,
    },
    ByteStream, Region,
};
use rusoto_s3::{
    AbortMultipartUploadRequest, CompleteMultipartUploadRequest, CompletedMultipartUpload,
    CompletedPart, CreateMultipartUploadRequest, GetObjectRequest, S3Client, UploadPartRequest, S3,
};
use rusoto_sts::WebIdentityProvider;
use std::{
    boxed::Box,
    env,
    io::{Read, Write},
    mem,
    pin::Pin,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    runtime::{Builder, Runtime},
};

// We use workload identity to map GCP service accounts to Kubernetes service
// accounts and make an auth token for the GCP account available to containers
// in the GKE metadata service. Sadly we can't use the Kubernetes feature to
// automount a service account token, because that would provide the token for
// the *Kubernetes* service account, not the GCP one.
// See terraform/modules/gke/gke.tf and terraform/modules/kuberenetes/kubernetes.tf
const METADATA_SERVICE_TOKEN_URL: &str = "http://metadata.google.internal:80/computeMetadata/v1/instance/service-accounts/default/identity";

// When running in GCP, we are provided the AWS account ID that owns our buckets
// via environment variable.
const AWS_ACCOUNT_ID_ENVIRONMENT_VARIABLE: &str = "AWS_ACCOUNT_ID";

/// Constructs a basic runtime suitable for use in our single threaded context
fn basic_runtime() -> Result<Runtime> {
    Ok(Builder::new().basic_scheduler().enable_all().build()?)
}

/// Implementation of Transport that reads and writes objects from Amazon S3.
pub struct S3Transport {
    path: S3Path,
    iam_role: Option<String>,
    // client_provider allows injection of mock S3Client for testing purposes
    client_provider: ClientProvider,
}

impl S3Transport {
    pub fn new(path: S3Path, identity: Identity) -> S3Transport {
        S3Transport::new_with_client(
            path,
            identity,
            Box::new(|region: &Region, iam_role: Option<String>| {
                // Rusoto uses Hyper which uses connection pools. The default
                // timeout for those connections is 90 seconds[1]. Amazon S3's
                // API closes idle client connections after 20 seconds[2]. If we
                // use a default client via S3Client::new, this mismatch causes
                // uploads to fail when Hyper tries to re-use a connection that
                // has been idle too long. Until this is fixed in Rusoto[3], we
                // construct our own HTTP request dispatcher whose underlying
                // hyper::Client is configured to timeout idle connections after
                // 10 seconds. We could also implement retries on our layer[4].
                //
                // [1]: https://docs.rs/hyper/0.13.8/hyper/client/struct.Builder.html#method.pool_idle_timeout
                // [2]: https://aws.amazon.com/premiumsupport/knowledge-center/s3-socket-connection-timeout-error/
                // [3]: https://github.com/rusoto/rusoto/issues/1686
                // [4]: https://github.com/abetterinternet/prio-server/issues/41
                let mut builder = hyper::Client::builder();
                builder.pool_idle_timeout(Duration::from_secs(10));
                let connector = HttpsConnector::new();
                let http_client = rusoto_core::HttpClient::from_builder(builder, connector);

                if let Some(iam_role) = iam_role {
                    // When running in GKE, the token used to authenticate to
                    // AWS S3 is made available via the instance metadata
                    // service. See terraform/modules/kubernetes/kubernetes.tf
                    // for discussion.
                    // This dynamic variable lets us provide a callback for
                    // fetching tokens, allowing Rusoto to automatically get new
                    // credentials if they expire (which they do every hour).
                    let oidc_token_variable = Variable::dynamic(|| {
                        let aws_account_id = env::var(AWS_ACCOUNT_ID_ENVIRONMENT_VARIABLE)
                            .map_err(|e| {
                                CredentialsError::new(format!(
                                    "could not read {} from environment: {}",
                                    AWS_ACCOUNT_ID_ENVIRONMENT_VARIABLE, e
                                ))
                            })?;
                        // We use ureq for this request because it is designed
                        // to use purely synchronous Rust. Ironically, we are in
                        // the context of an async runtime when this callback is
                        // invoked, but because the closure is not declared
                        // async, we cannot use .await to work with Futures.
                        let response = ureq::get(METADATA_SERVICE_TOKEN_URL)
                            .query(
                                "audience",
                                format!("sts.amazonaws.com/{}", aws_account_id).as_ref(),
                            )
                            .set("Metadata-Flavor", "Google")
                            // By default, ureq will wait forever to connect or
                            // read.
                            .timeout_connect(10_000) // ten seconds
                            .timeout_read(10_000) // ten seconds
                            .call();
                        if response.error() {
                            return Err(CredentialsError::new(format!(
                                "failed to fetch auth token from metadata service: {:?}",
                                response
                            )));
                        }
                        let token = response.into_string().map_err(|e| {
                            CredentialsError::new(format!(
                                "failed to fetch auth token from metadata service: {}",
                                e
                            ))
                        })?;
                        Ok(Secret::from(token))
                    });

                    let credentials_provider =
                        AutoRefreshingProvider::new(WebIdentityProvider::new(
                            oidc_token_variable,
                            // The AWS role that we are assuming is provided via
                            // environment variable. See
                            // terraform/modules/kubernetes/kubernetes.tf
                            iam_role,
                            // The role ARN we assume is already bound to a
                            // specific facilitator instance, so we don't get
                            // much from further scoping the role assumption,
                            // unless we eventually introduce something like a
                            // job ID that could then show up in AWS-side logs.
                            // https://docs.aws.amazon.com/credref/latest/refdocs/setting-global-role_session_name.html
                            Some(Variable::from_env_var_optional("AWS_ROLE_SESSION_NAME")),
                        ))?;

                    Ok(S3Client::new_with(
                        http_client,
                        credentials_provider,
                        region.clone(),
                    ))
                } else {
                    // Credentials for authenticating to AWS are automatically
                    // sourced from environment variables or ~/.aws/credentials.
                    // https://github.com/rusoto/rusoto/blob/master/AWS-CREDENTIALS.md
                    let credentials_provider = DefaultCredentialsProvider::new()
                        .context("failed to create credentials provider")?;
                    Ok(S3Client::new_with(
                        http_client,
                        credentials_provider,
                        region.clone(),
                    ))
                }
            }),
        )
    }

    fn new_with_client(
        path: S3Path,
        identity: Identity,
        client_provider: ClientProvider,
    ) -> S3Transport {
        S3Transport {
            path: path.ensure_directory_prefix(),
            iam_role: identity.map(|x| x.to_string()),
            client_provider,
        }
    }
}

// ClientProvider allows mocking out a client for testing.
type ClientProvider = Box<dyn Fn(&Region, Option<String>) -> Result<S3Client>>;

impl Transport for S3Transport {
    fn get(&mut self, key: &str) -> Result<Box<dyn Read>> {
        debug!("get {} as {:?}", self.path, self.iam_role);
        let mut runtime = basic_runtime()?;
        let client = (self.client_provider)(&self.path.region, self.iam_role.clone())?;
        let get_output = runtime
            .block_on(client.get_object(GetObjectRequest {
                bucket: self.path.bucket.to_owned(),
                key: [&self.path.key, key].concat(),
                ..Default::default()
            }))
            .context("error getting S3 object")?;

        let body = get_output.body.context("no body in GetObjectResponse")?;

        Ok(Box::new(StreamingBodyReader::new(body, runtime)))
    }

    fn put(&mut self, key: &str) -> Result<Box<dyn TransportWriter>> {
        debug!("put {} as {:?}", self.path, self.iam_role);
        let writer = MultipartUploadWriter::new(
            self.path.bucket.to_owned(),
            format!("{}{}", &self.path.key, key),
            // Set buffer size to 5 MB, which is the minimum required by Amazon
            // https://docs.aws.amazon.com/AmazonS3/latest/dev/qfacts.html
            5_242_880,
            (self.client_provider)(&self.path.region, self.iam_role.clone())?,
        )?;
        Ok(Box::new(writer))
    }
}

/// StreamingBodyReader is an std::io::Read implementation which reads from the
/// tokio::io::AsyncRead inside the StreamingBody in a Rusoto API request
/// response.
struct StreamingBodyReader {
    body_reader: Pin<Box<dyn AsyncRead + Send + Sync>>,
    runtime: Runtime,
}

impl StreamingBodyReader {
    fn new(body: ByteStream, runtime: Runtime) -> StreamingBodyReader {
        StreamingBodyReader {
            body_reader: Box::pin(body.into_async_read()),
            runtime,
        }
    }
}

impl Read for StreamingBodyReader {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
        self.runtime.block_on(self.body_reader.read(buf))
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
    runtime: Runtime,
    #[derivative(Debug = "ignore")]
    client: S3Client,
    bucket: String,
    key: String,
    upload_id: String,
    completed_parts: Vec<CompletedPart>,
    minimum_upload_part_size: usize,
    buffer: Vec<u8>,
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
    ) -> Result<MultipartUploadWriter> {
        let mut runtime = basic_runtime()?;

        let create_output = runtime
            .block_on(
                client.create_multipart_upload(CreateMultipartUploadRequest {
                    bucket: bucket.to_string(),
                    key: key.to_string(),
                    ..Default::default()
                }),
            )
            .context("error creating multipart upload")?;

        Ok(MultipartUploadWriter {
            runtime,
            client,
            bucket,
            key,
            upload_id: create_output
                .upload_id
                .context("no upload ID in CreateMultipartUploadResponse")?,
            completed_parts: Vec::new(),
            // Upload parts must be at least buffer_capacity, but it's fine if
            // they're bigger, so overprovision the buffer to make it unlikely
            // that the caller will overflow it.
            minimum_upload_part_size,
            buffer: Vec::with_capacity(minimum_upload_part_size * 2),
        })
    }

    /// Upload content in internal buffer, if any, to S3 in an UploadPart call.
    fn upload_part(&mut self) -> Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        let part_number = (self.completed_parts.len() + 1) as i64;

        let upload_output = self
            .runtime
            .block_on(
                self.client.upload_part(UploadPartRequest {
                    bucket: self.bucket.to_string(),
                    key: self.key.to_string(),
                    upload_id: self.upload_id.clone(),
                    part_number: part_number as i64,
                    body: Some(
                        // Move internal buffer into request object and replace
                        // it with a new, empty buffer.
                        mem::replace(
                            &mut self.buffer,
                            Vec::with_capacity(self.minimum_upload_part_size * 2),
                        )
                        .into(),
                    ),
                    ..Default::default()
                }),
            )
            .context("failed to upload_part")
            .map_err(|e| {
                // Clean up botched uploads
                if let Err(cancel) = self.cancel_upload() {
                    return cancel.context(e);
                }
                e
            })?;

        let e_tag = upload_output
            .e_tag
            .context("no ETag in UploadPartOutput")
            .map_err(|e| {
                if let Err(cancel) = self.cancel_upload() {
                    return cancel.context(e);
                }
                e
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
        // Write into memory buffer, and upload to S3 if we have accumulated
        // enough content.
        self.buffer.extend_from_slice(buf);
        if self.buffer.len() >= self.minimum_upload_part_size {
            self.upload_part().map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::Other, Error::AnyhowError(e))
            })?;
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl TransportWriter for MultipartUploadWriter {
    fn complete_upload(&mut self) -> Result<()> {
        // Write last part, if any
        self.upload_part()?;

        // Ignore output for now, but we might want the e_tag to check the
        // digest
        self.runtime
            .block_on(
                self.client
                    .complete_multipart_upload(CompleteMultipartUploadRequest {
                        bucket: self.bucket.to_string(),
                        key: self.key.to_string(),
                        upload_id: self.upload_id.clone(),
                        multipart_upload: Some(CompletedMultipartUpload {
                            parts: Some(mem::take(&mut self.completed_parts)),
                        }),
                        ..Default::default()
                    }),
            )
            .context("error completing upload")?;
        Ok(())
    }

    fn cancel_upload(&mut self) -> Result<()> {
        // There's nothing useful in the output so discard it
        self.runtime.block_on(
            self.client
                .abort_multipart_upload(AbortMultipartUploadRequest {
                    bucket: self.bucket.to_string(),
                    key: self.key.to_string(),
                    upload_id: self.upload_id.clone(),
                    ..Default::default()
                }),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusoto_core::signature::SignedRequest;
    use rusoto_mock::{
        MockCredentialsProvider, MockRequestDispatcher, MultipleMockRequestDispatcher,
    };
    use rusoto_s3::CreateMultipartUploadError;
    use std::io::Read;

    // Rusoto provides us the ability to create mock clients and play canned
    // responses to API requests. Besides that, we want to verify that we get
    // the expected sequence of API requests, for instance to verify that we
    // call AbortMultipartUpload if an UploadPart call fails. The mock client
    // will crash if it runs out of canned responses to give the client, which
    // will catch excess API calls. To make sure we issue the correct API calls,
    // we use with_request_checker to examine the outgoing requests. We only get
    // to examine a rusoto::signature::SignedRequest, which does not contain an
    // explicit indication of which API call the HTTP request represents. So we
    // have the `is_*_request` functions below, which compare HTTP method and
    // URL parameters against the Amazon API specification to figure out what
    // API call we are looking at.

    const TEST_BUCKET: &str = "fake-bucket";
    const TEST_KEY: &str = "fake-key";

    fn is_create_multipart_upload_request(request: &SignedRequest) {
        // https://docs.aws.amazon.com/AmazonS3/latest/API/API_CreateMultipartUpload.html
        assert_eq!(
            request.method, "POST",
            "expected CreateMultipartUpload request, found {:?}",
            request
        );
        assert!(
            request.params.contains_key("uploads"),
            "expected CreateMultipartUpload request, found {:?}",
            request
        );
    }

    fn is_upload_part_request(request: &SignedRequest) {
        // https://docs.aws.amazon.com/AmazonS3/latest/API/API_UploadPart.html
        assert_eq!(
            request.method, "PUT",
            "expected UploadPart request, found {:?}",
            request
        );
        assert!(
            request.params.contains_key("partNumber"),
            "expected UploadPart request, found {:?}",
            request
        );
        assert!(
            request.params.contains_key("uploadId"),
            "expected UploadPart request, found {:?}",
            request
        );
    }

    fn is_abort_multipart_upload_request(request: &SignedRequest) {
        // https://docs.aws.amazon.com/AmazonS3/latest/API/API_AbortMultipartUpload.html
        assert_eq!(
            request.method, "DELETE",
            "expected AbortMultipartUpload request, found {:?}",
            request
        );
        assert!(
            request.params.contains_key("uploadId"),
            "expected AbortMultipartUpload request, found {:?}",
            request
        );
    }

    fn is_complete_multipart_upload_request(request: &SignedRequest) {
        // https://docs.aws.amazon.com/AmazonS3/latest/API/API_CompleteMultipartUpload.html
        assert_eq!(
            request.method, "POST",
            "expected CompleteMultipartUpload request, found {:?}",
            request
        );
        assert!(
            request.params.contains_key("uploadId"),
            "expected CompleteMultipartUpload request, found {:?}",
            request
        );
    }

    fn is_get_object_request(request: &SignedRequest) {
        // https://docs.aws.amazon.com/AmazonS3/latest/API/API_GetObject.html
        assert_eq!(
            request.method, "GET",
            "expected GetObject request, found {:?}",
            request
        );
        assert_eq!(
            request.path, "/fake-bucket/fake-key",
            "expected GetObject request, found {:?}",
            request
        );
        // Because we pass no options to get_options, our requests end up with
        // no parameters, which is what we use to distinguish from e.g.
        // GetObjectAcl with would have a param "acl" in it.
        assert!(
            request.params.is_empty(),
            "expected GetObject request, found {:?}",
            request
        );
    }

    #[test]
    fn multipart_upload_create_fails() {
        let err = MultipartUploadWriter::new(
            String::from(TEST_BUCKET),
            String::from(TEST_KEY),
            50,
            S3Client::new_with(
                MockRequestDispatcher::with_status(401)
                    .with_request_checker(is_create_multipart_upload_request),
                MockCredentialsProvider,
                Region::UsWest2,
            ),
        )
        .expect_err("expected error");
        assert!(
            err.is::<rusoto_core::RusotoError<CreateMultipartUploadError>>(),
            "found unexpected error {:?}",
            err
        );
    }

    #[test]
    fn multipart_upload_create_no_upload_id() {
        // Response body format from
        // https://docs.aws.amazon.com/AmazonS3/latest/API/API_Operations_Amazon_Simple_Storage_Service.html
        MultipartUploadWriter::new(
            String::from(TEST_BUCKET),
            String::from(TEST_KEY),
            50,
            S3Client::new_with(
                MockRequestDispatcher::with_status(200)
                    .with_body(
                        r#"<?xml version="1.0" encoding="UTF-8"?>
<InitiateMultipartUploadResult>
   <Bucket>fake-bucket</Bucket>
   <Key>fake-key</Key>
</InitiateMultipartUploadResult>"#,
                    )
                    .with_request_checker(is_create_multipart_upload_request),
                MockCredentialsProvider,
                Region::UsWest2,
            ),
        )
        .expect_err("expected error");
    }

    #[test]
    fn multipart_upload() {
        // Response body format from
        // https://docs.aws.amazon.com/AmazonS3/latest/API/API_Operations_Amazon_Simple_Storage_Service.html
        let mut writer =
            MultipartUploadWriter::new(String::from(TEST_BUCKET), String::from(TEST_KEY), 50, {
                let requests = vec![
                    // Response to CreateMultipartUpload
                    MockRequestDispatcher::with_status(200)
                        .with_body(
                            r#"<?xml version="1.0" encoding="UTF-8"?>
<InitiateMultipartUploadResult>
   <Bucket>fake-bucket</Bucket>
   <Key>fake-key</Key>
   <UploadId>upload-id</UploadId>
</InitiateMultipartUploadResult>"#,
                        )
                        .with_request_checker(is_create_multipart_upload_request),
                    // Response to UploadPart. HTTP 401 will cause failure.
                    MockRequestDispatcher::with_status(401)
                        .with_request_checker(is_upload_part_request),
                    // Response to AbortMultipartUpload, expected because of
                    // previous UploadPart failure
                    MockRequestDispatcher::with_status(204)
                        .with_request_checker(is_abort_multipart_upload_request),
                    // Response to UploadPart. HTTP 200 but no ETag header will
                    // cause failure.
                    MockRequestDispatcher::with_status(200)
                        .with_request_checker(is_upload_part_request),
                    // Response to AbortMultipartUpload, expected because of
                    // previous UploadPart failure
                    MockRequestDispatcher::with_status(204)
                        .with_request_checker(is_abort_multipart_upload_request),
                    // Well formed response to UploadPart.
                    MockRequestDispatcher::with_status(200)
                        .with_request_checker(is_upload_part_request)
                        .with_header("ETag", "fake-etag"),
                    // Well formed response to UploadPart
                    MockRequestDispatcher::with_status(200)
                        .with_request_checker(is_upload_part_request)
                        .with_header("ETag", "fake-etag"),
                    // Well formed response to CompleteMultipartUpload
                    MockRequestDispatcher::with_status(200)
                        .with_request_checker(is_complete_multipart_upload_request)
                        .with_body(
                            r#"<?xml version="1.0" encoding="UTF-8"?>
<CompleteMultipartUploadResult>
   <Location>string</Location>
   <Bucket>fake-bucket</Bucket>
   <Key>fake-key</Key>
   <ETag>fake-etag</ETag>
</CompleteMultipartUploadResult>"#,
                        ),
                    // Well formed response to CompleteMultipartUpload
                    MockRequestDispatcher::with_status(200)
                        .with_request_checker(is_complete_multipart_upload_request)
                        .with_body(
                            r#"<?xml version="1.0" encoding="UTF-8"?>
<CompleteMultipartUploadResult>
   <Location>string</Location>
   <Bucket>fake-bucket</Bucket>
   <Key>fake-key</Key>
   <ETag>fake-etag</ETag>
</CompleteMultipartUploadResult>"#,
                        ),
                    // Failure response to CompleteMultipartUpload
                    MockRequestDispatcher::with_status(400)
                        .with_request_checker(is_complete_multipart_upload_request),
                ];
                S3Client::new_with(
                    MultipleMockRequestDispatcher::new(requests),
                    MockCredentialsProvider,
                    Region::UsWest2,
                )
            })
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
        // The buffer is empty now, so another flush will not cause an
        // UploadPart call
        writer.complete_upload().unwrap();
        writer.complete_upload().unwrap_err();
    }

    #[test]
    fn roundtrip_s3_transport() {
        let s3_path = S3Path {
            region: Region::UsWest2,
            bucket: TEST_BUCKET.into(),
            key: "".into(),
        };

        let provider = Box::new(|region: &Region, _: Option<String>| {
            Ok(S3Client::new_with(
                // Failed GetObject request
                MockRequestDispatcher::with_status(404).with_request_checker(is_get_object_request),
                MockCredentialsProvider,
                region.clone(),
            ))
        });
        let mut transport = S3Transport::new_with_client(s3_path.clone(), None, provider);

        let ret = transport.get(TEST_KEY);
        assert!(ret.is_err(), "unexpected return value {:?}", ret.err());

        let mut transport = S3Transport::new_with_client(
            s3_path.clone(),
            None,
            Box::new(|region: &Region, _: Option<String>| {
                Ok(S3Client::new_with(
                    // Successful GetObject request
                    MockRequestDispatcher::with_status(200)
                        .with_request_checker(is_get_object_request)
                        .with_body("fake-content"),
                    MockCredentialsProvider,
                    region.clone(),
                ))
            }),
        );

        let mut reader = transport
            .get(TEST_KEY)
            .expect("unexpected error getting reader");
        let mut content = Vec::new();
        reader.read_to_end(&mut content).expect("failed to read");
        assert_eq!(Vec::from("fake-content"), content);

        let mut transport = S3Transport::new_with_client(
            s3_path,
            None,
            Box::new(|region: &Region, _: Option<String>| {
                let requests = vec![
                    // Response to CreateMultipartUpload
                    MockRequestDispatcher::with_status(200)
                        .with_body(
                            r#"<?xml version="1.0" encoding="UTF-8"?>
<InitiateMultipartUploadResult>
   <Bucket>fake-bucket</Bucket>
   <Key>fake-key</Key>
   <UploadId>upload-id</UploadId>
</InitiateMultipartUploadResult>"#,
                        )
                        .with_request_checker(is_create_multipart_upload_request),
                    // Well formed response to UploadPart
                    MockRequestDispatcher::with_status(200)
                        .with_request_checker(is_upload_part_request)
                        .with_header("ETag", "fake-etag"),
                    // Well formed response to CompleteMultipartUpload
                    MockRequestDispatcher::with_status(200)
                        .with_request_checker(is_complete_multipart_upload_request)
                        .with_body(
                            r#"<?xml version="1.0" encoding="UTF-8"?>
<CompleteMultipartUploadResult>
   <Location>string</Location>
   <Bucket>fake-bucket</Bucket>
   <Key>fake-key</Key>
   <ETag>fake-etag</ETag>
</CompleteMultipartUploadResult>"#,
                        ),
                    // Response to AbortMultipartUpload, expected because of
                    // cancel_upload call
                    MockRequestDispatcher::with_status(204)
                        .with_request_checker(is_abort_multipart_upload_request),
                ];
                Ok(S3Client::new_with(
                    MultipleMockRequestDispatcher::new(requests),
                    MockCredentialsProvider,
                    region.clone(),
                ))
            }),
        );

        let mut writer = transport.put(TEST_KEY).unwrap();
        writer.write_all(b"fake-content").unwrap();
        writer.complete_upload().unwrap();
        writer.cancel_upload().unwrap();
    }
}

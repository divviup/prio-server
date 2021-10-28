use anyhow::{Context, Result};
use dyn_clone::DynClone;
use slog::Logger;
use std::{
    convert::From,
    default::Default,
    fmt::Debug,
    time::{Duration, Instant},
};
use ureq::{Agent, AgentBuilder, Request, Response, SerdeValue};
use url::Url;

use crate::{metrics::ApiClientMetricsCollector, retries::retry_request, Error};

/// Method contains the HTTP methods supported by this crate.
#[derive(Debug)]
pub(crate) enum Method {
    Get,
    Post,
    Put,
    Delete,
}

impl Method {
    /// Converts the enum to a primitive string to be used by the ureq::Agent
    fn to_primitive_string(&self) -> &str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
        }
    }
}

/// An HTTP agent that can be configured to manage "Authorization" headers and
/// retries using exponential backoff.
#[derive(Debug, Clone)]
pub(crate) struct RetryingAgent {
    /// Agent to use for constructing HTTP requests.
    agent: Agent,
    /// Requests which fail due to transport problems or which return any HTTP
    /// status code in this list or in the 5xx range will be retried with
    /// exponential backoff.
    additional_retryable_http_status_codes: Vec<u16>,
    service: String,
    api_metrics: ApiClientMetricsCollector,
}

impl RetryingAgent {
    /// Create a `RetryingAgent` with a customized `ureq::Agent` and a list of
    /// retryable HTTP status codes.
    pub fn new_with_agent(
        agent: Agent,
        additional_retryable_http_status_codes: Vec<u16>,
        service: &str,
        api_metrics: &ApiClientMetricsCollector,
    ) -> Self {
        Self {
            agent,
            additional_retryable_http_status_codes,
            service: service.to_string(),
            api_metrics: api_metrics.clone(),
        }
    }

    /// Create a `RetryingAgent` without any additional retryable HTTP status
    /// codes and a `ureq::Agent` suitable for most uses.
    pub fn new(service: &str, api_metrics: &ApiClientMetricsCollector) -> Self {
        Self::new_with_agent(
            AgentBuilder::new().timeout(Duration::from_secs(10)).build(),
            vec![],
            service,
            api_metrics,
        )
    }

    /// Prepares a request for the provided `RequestParameters`. Returns a
    /// `ureq::Request` permitting the caller to further customize the request
    /// (e.g., with HTTP headers or query parameters). Callers may use methods
    /// like `send()` or `send_bytes()` directly on the returned `Request`, but
    /// must use `RetryingAgent::send_json_request`, `::send_bytes` or
    /// `::send_string` to get retries.
    /// Returns an Error if the AccessTokenProvider returns an error when
    /// supplying the request with an access token.
    pub(crate) fn prepare_request(&self, parameters: RequestParameters) -> Result<Request, Error> {
        let mut request = self
            .agent
            .request_url(parameters.method.to_primitive_string(), &parameters.url);
        if let Some(token_provider) = parameters.token_provider {
            let token = token_provider.ensure_access_token()?;
            request = request.set("Authorization", &format!("Bearer {}", token));
        }
        Ok(request)
    }

    fn is_http_status_retryable(&self, http_status: u16) -> bool {
        http_status >= 500
            || self
                .additional_retryable_http_status_codes
                .contains(&http_status)
    }

    fn is_error_retryable(&self, error: &ureq::Error) -> bool {
        match error {
            ureq::Error::Status(http_status, _) => self.is_http_status_retryable(*http_status),
            ureq::Error::Transport(_) => true,
        }
    }

    /// Send the provided request with the provided JSON body.
    pub(crate) fn send_json_request(
        &self,
        logger: &Logger,
        request: &Request,
        endpoint: &'static str,
        body: &SerdeValue,
    ) -> Result<Response, Error> {
        Ok(retry_request(
            logger,
            || self.do_request_with_metrics(endpoint, || request.clone().send_json(body.clone())),
            |ureq_error| self.is_error_retryable(ureq_error),
        )?)
    }

    /// Send the provided request with the provided bytes as the body.
    pub(crate) fn send_bytes(
        &self,
        logger: &Logger,
        request: &Request,
        endpoint: &'static str,
        data: &[u8],
    ) -> Result<Response, Error> {
        Ok(retry_request(
            logger,
            || self.do_request_with_metrics(endpoint, || request.clone().send_bytes(data)),
            |ureq_error| self.is_error_retryable(ureq_error),
        )?)
    }

    /// Send the provided data as a form encoded body.
    pub(crate) fn send_form(
        &self,
        logger: &Logger,
        request: &Request,
        endpoint: &'static str,
        data: &[(&str, &str)],
    ) -> Result<Response, Error> {
        Ok(retry_request(
            logger,
            || self.do_request_with_metrics(endpoint, || request.clone().send_form(data)),
            |ureq_error| self.is_error_retryable(ureq_error),
        )?)
    }

    /// Send the provided request with no body.
    pub(crate) fn call(
        &self,
        logger: &Logger,
        request: &Request,
        endpoint: &'static str,
    ) -> Result<Response, Error> {
        Ok(retry_request(
            logger,
            || self.do_request_with_metrics(endpoint, || request.clone().call()),
            |ureq_error| self.is_error_retryable(ureq_error),
        )?)
    }

    /// Perform some operation `op`, logging metrics on the request status and
    /// latency.
    fn do_request_with_metrics<F>(
        &self,
        endpoint: &'static str,
        mut op: F,
    ) -> Result<Response, ureq::Error>
    where
        F: FnMut() -> Result<Response, ureq::Error>,
    {
        let before = Instant::now();
        let result = op();
        let latency = before.elapsed().as_millis();

        let http_status_label = match result {
            Ok(ref r) => r.status().to_string(),
            Err(ureq::Error::Status(http_status, _)) => http_status.to_string(),
            Err(_) => "unknown".to_owned(),
        };

        self.api_metrics
            .latency
            .with_label_values(&[&self.service, endpoint, &http_status_label])
            .observe(latency as f64);

        result
    }
}

/// Defines a behavior responsible for produing bearer authorization tokens
pub(crate) trait AccessTokenProvider: Debug + DynClone + Send + Sync {
    /// Returns a valid bearer authroization token
    fn ensure_access_token(&self) -> Result<String>;
}

dyn_clone::clone_trait_object!(AccessTokenProvider);

/// StaticAccessTokenProvider is an AccessTokenProvider that contains a String
/// as the token. This structure implements the AccessTokenProvider trait and can
/// be used in RequestParameters.
#[derive(Clone, Debug)]
pub(crate) struct StaticAccessTokenProvider {
    pub token: String,
}

impl AccessTokenProvider for StaticAccessTokenProvider {
    fn ensure_access_token(&self) -> Result<String> {
        Ok(self.token.clone())
    }
}

impl From<String> for StaticAccessTokenProvider {
    fn from(token: String) -> Self {
        StaticAccessTokenProvider { token }
    }
}

/// Struct containing parameters for send_json_request
#[derive(Debug)]
pub(crate) struct RequestParameters<'a> {
    /// The url to request
    pub url: Url,
    /// The method of the request (GET, POST, etc)
    pub method: Method,
    /// If this field is set, the request with be sent with an "Authorization"
    /// header containing a bearer token obtained from the AccessTokenProvider.
    /// If unset, the request is sent unauthenticated.
    pub token_provider: Option<&'a dyn AccessTokenProvider>,
}

impl Default for RequestParameters<'_> {
    fn default() -> Self {
        let default_url = Url::parse("https://example.com").expect("example url did not parse");

        RequestParameters {
            url: default_url,
            method: Method::Get,
            token_provider: None,
        }
    }
}

/// simple_get_request does a HTTP request to a URL and returns the body as a
// string.
pub(crate) fn simple_get_request(
    url: Url,
    logger: &Logger,
    service: &str,
    api_metrics: &ApiClientMetricsCollector,
) -> Result<String, Error> {
    let agent = RetryingAgent::new(service, api_metrics);
    let request = agent
        .prepare_request(RequestParameters {
            url,
            method: Method::Get,
            ..Default::default()
        })
        .context("creating simple_get_request failed")?;

    Ok(agent
        .call(logger, &request, "simple_get_request")?
        .into_string()
        .context("failed to convert GET response body into string")?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::setup_test_logging;
    use mockito::{mock, Matcher};

    #[test]
    fn retryable_error() {
        let api_metrics =
            ApiClientMetricsCollector::new_with_metric_name("retryable_error").unwrap();

        let http_400 = ureq::Error::Status(400, Response::new(400, "", "").unwrap());
        let http_429 = ureq::Error::Status(429, Response::new(429, "", "").unwrap());
        let http_500 = ureq::Error::Status(500, Response::new(500, "", "").unwrap());
        let http_503 = ureq::Error::Status(503, Response::new(503, "", "").unwrap());
        // There is currently no way to create a ureq::Error::Transport so we
        // settle for testing different HTTP status codes.
        // https://github.com/algesten/ureq/issues/373

        let mut agent = RetryingAgent::new("retryable_error", &api_metrics);
        assert!(!agent.is_error_retryable(&http_400));
        assert!(!agent.is_error_retryable(&http_429));
        assert!(agent.is_error_retryable(&http_500));
        assert!(agent.is_error_retryable(&http_503));

        agent.additional_retryable_http_status_codes = vec![429];

        assert!(!agent.is_error_retryable(&http_400));
        assert!(agent.is_error_retryable(&http_429));
        assert!(agent.is_error_retryable(&http_500));
        assert!(agent.is_error_retryable(&http_503));
    }

    #[test]
    fn authenticated_request() {
        let logger = setup_test_logging();
        let api_metrics =
            ApiClientMetricsCollector::new_with_metric_name("authenticated_request").unwrap();

        let mocked_get = mock("GET", "/resource")
            .match_header("Authorization", "Bearer fake-token")
            .with_status(200)
            .with_body("fake body")
            .expect_at_most(1)
            .create();

        let oauth_token_provider = StaticAccessTokenProvider {
            token: "fake-token".to_string(),
        };

        let request_parameters = RequestParameters {
            url: Url::parse(&format!("{}/resource", mockito::server_url())).unwrap(),
            method: Method::Get,
            token_provider: Some(&oauth_token_provider),
        };

        let agent = RetryingAgent::new("authenticated_request", &api_metrics);

        let request = agent.prepare_request(request_parameters).unwrap();

        let response = agent.call(&logger, &request, "fake-endpoint").unwrap();

        mocked_get.assert();

        assert_eq!(response.status(), 200);
        assert_eq!(response.into_string().unwrap(), "fake body");
    }

    #[test]
    fn unauthenticated_request() {
        let logger = setup_test_logging();
        let api_metrics =
            ApiClientMetricsCollector::new_with_metric_name("unauthenticated_request").unwrap();

        let mocked_get = mock("GET", "/resource")
            .match_header("Authorization", Matcher::Missing)
            .with_status(200)
            .with_body("fake body")
            .expect_at_most(1)
            .create();

        let request_parameters = RequestParameters {
            url: Url::parse(&format!("{}/resource", mockito::server_url())).unwrap(),
            method: Method::Get,
            token_provider: None,
        };

        let agent = RetryingAgent::new("unauthenticated_request", &api_metrics);

        let request = agent.prepare_request(request_parameters).unwrap();

        let response = agent.call(&logger, &request, "fake-endpoint").unwrap();

        mocked_get.assert();

        assert_eq!(response.status(), 200);
        assert_eq!(response.into_string().unwrap(), "fake body");
    }
}

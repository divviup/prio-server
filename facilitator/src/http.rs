use anyhow::{Context, Result};
use std::fmt::Debug;
use std::time::Duration;
use ureq::{Agent, AgentBuilder, Request, Response, SerdeValue};
use url::Url;

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

/// Creates an agent with a timeout of 10 seconds
pub(crate) fn create_agent() -> Agent {
    AgentBuilder::new().timeout(Duration::from_secs(10)).build()
}

/// Prepares a request by taking in an agent and the RequestParameters. Could
/// return an Error if the OauthTokenProvider returns an error when supplying
/// the request with an Oauth token.
pub(crate) fn prepare_request(agent: &Agent, parameters: RequestParameters<'_>) -> Result<Request> {
    let request = agent.request_url(parameters.method.to_primitive_string(), &parameters.url);
    parameters.authenticated_request(request)
}

/// Prepares a request and wraps it with an agent that has a timeout of 10
/// seconds. Read prepare_request for more information.
pub(crate) fn prepare_request_without_agent(parameters: RequestParameters<'_>) -> Result<Request> {
    prepare_request(&create_agent(), parameters)
}

/// Defines a behavior responsible for produing bearer authorization tokens
pub(crate) trait OauthTokenProvider: Debug {
    /// Returns a valid bearer authroization token
    fn ensure_oauth_token(&mut self) -> Result<String>;
}

/// StaticOauthTokenProvider is an OauthTokenProvider that contains a String
/// as the token. This structure implements the OauthTokenProvider trait and can
/// be used in RequestParameters.
#[derive(Debug)]
pub(crate) struct StaticOauthTokenProvider {
    pub token: String,
}

impl OauthTokenProvider for StaticOauthTokenProvider {
    fn ensure_oauth_token(&mut self) -> Result<String> {
        Ok(self.token.clone())
    }
}

impl std::convert::From<String> for StaticOauthTokenProvider {
    fn from(token: String) -> Self {
        StaticOauthTokenProvider { token }
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
    /// header containing a bearer token obtained from the OauthTokenProvider.
    /// If unset, the request is sent unauthenticated.
    pub token_provider: Option<&'a mut dyn OauthTokenProvider>,
}

impl std::default::Default for RequestParameters<'_> {
    fn default() -> Self {
        let default_url = Url::parse("https://example.com").expect("example url did not parse");

        RequestParameters {
            url: default_url,
            method: Method::Get,
            token_provider: None,
        }
    }
}

impl RequestParameters<'_> {
    fn authenticated_request(self, request: Request) -> Result<Request> {
        let token = match self.token_provider {
            Some(token_provider) => match token_provider.ensure_oauth_token() {
                Ok(token) => Ok(Some(token)),
                Err(err) => Err(err),
            },
            None => Ok(None),
        }?;

        match token {
            Some(token) => Ok(request.set("Authorization", &format!("Bearer {}", token))),
            None => Ok(request),
        }
    }
}

/// Send the provided request with the provided JSON body. See
/// prepare_request for more details.
pub(crate) fn send_json_request(request: Request, body: SerdeValue) -> Result<Response> {
    request
        .send_json(body)
        .context("failed to send json request")
}

/// simple_get_request does a HTTP request to a URL and returns the body as a
// string.
pub(crate) fn simple_get_request(url: Url) -> Result<String> {
    let request = prepare_request_without_agent(RequestParameters {
        url: url,
        method: Method::Get,
        ..Default::default()
    })
    .context("creating simple_get_request failed")?;

    request
        .call()
        .context("Failed to GET resource")?
        .into_string()
        .context("failed to convert GET response body into string")
}

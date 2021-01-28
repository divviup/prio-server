use anyhow::{Context, Result};
use std::fmt::Debug;
use std::time::Duration;
use ureq::{Agent, AgentBuilder, Request, Response, SerdeValue};
use url::Url;

#[derive(Debug)]
pub(crate) enum Method {
    Get,
    Post,
    Put,
    Delete,
}

impl Method {
    fn to_primitive_string(&self) -> &str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
        }
    }
}

fn create_agent() -> Agent {
    AgentBuilder::new().timeout(Duration::from_secs(10)).build()
}

pub(crate) fn prepare_request(
    agent: Option<Agent>,
    parameters: RequestParameters<'_>,
) -> Result<Request> {
    let agent = agent.unwrap_or(create_agent());

    let request = agent.request_url(parameters.method.to_primitive_string(), &parameters.url);
    parameters.authenticated_request(request)
}

pub(crate) trait OauthTokenProvider: Debug {
    fn ensure_oauth_token(&mut self) -> Result<String>;
}

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
/// JsonRequestParameters for more details.
pub(crate) fn send_json_request(request: Request, body: SerdeValue) -> Result<Response> {
    request
        .send_json(body)
        .context("failed to send json request")
}

pub(crate) fn simple_get_request(url: Url) -> Result<String> {
    let request = prepare_request(
        None,
        RequestParameters {
            url: url.clone(),
            method: Method::Get,
            ..Default::default()
        },
    )
    .context("creating simple_get_request failed")?;

    request
        .call()
        .context("Failed to GET resource")?
        .into_string()
        .context("failed to convert GET response body into string")
}

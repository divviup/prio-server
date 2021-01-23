use anyhow::{Context, Result};
use lazy_static::lazy_static;
use std::time::Duration;
use ureq::{Agent, AgentBuilder, Error, Request, Response, SerdeValue};
use url::{ParseError, Url};

use crate::gcp_oauth::OauthTokenProvider;
lazy_static! {
    pub(crate) static ref HTTP: Http = Http::new();
    static ref EXAMPLE_URL: Url =
        Url::parse("https://example.com").expect("example url did not parse");
}
pub(crate) struct Http {
    agent: Agent,
}

impl Http {
    pub fn new() -> Self {
        Http {
            agent: AgentBuilder::new().timeout(Duration::from_secs(10)).build(),
        }
    }

    pub(crate) fn prepare_request(&self, mut parameters: RequestParameters) -> Request {
        let request = self.agent.request_url(parameters.method, parameters.url);
        parameters.get_authenticated_request(request)
    }
}

/// Struct containing parameters for send_json_request
#[derive(Debug)]
pub(crate) struct RequestParameters<'a> {
    /// The url to request
    pub url: &'a Url,
    /// The method of the request (GET, POST, etc)
    pub method: &'a str,
    /// If this field is set, the request with be sent with an "Authorization"
    /// header containing a bearer token obtained from the OauthTokenProvider.
    /// If unset, the request is sent unauthenticated if the token field is also not set.
    pub token_provider: Option<&'a mut OauthTokenProvider>,
    /// If the token_provider field isn't set, and this field is set, the request will
    /// be sent with an "Authorization" header containing a bearer token obtained from
    /// the OauthTokenProvider.
    pub token: Option<&'a str>,
}

impl std::default::Default for RequestParameters<'_> {
    fn default() -> Self {
        return RequestParameters {
            url: &EXAMPLE_URL,
            method: "",
            token_provider: None,
            token: None,
        };
    }
}

impl RequestParameters<'_> {
    fn get_authenticated_request(&mut self, request: Request) -> Request {
        let token = match &mut self.token_provider {
            Some(token_provider) => match token_provider.ensure_oauth_token() {
                Ok(token) => Some(token),
                Err(_err) => None,
            },
            None => match self.token {
                Some(token) => Some(String::from(token)),
                None => None,
            },
        };
        let authenticated_request = match token {
            Some(token) => request.set("Authorization", &format!("Bearer {}", token)),
            None => request,
        };

        authenticated_request
    }
}

/// Send the provided request with the provided JSON body. See
/// JsonRequestParameters for more details.
pub(crate) fn send_json_request(request: Request, body: SerdeValue) -> Result<Response, Error> {
    request.send_json(body)
}

pub(crate) fn simple_get_request(url: &Url) -> Result<String> {
    get_request(HTTP.prepare_request(RequestParameters {
        url: url,
        method: "GET",
        ..Default::default()
    }))
}

pub(crate) fn get_request(request: Request) -> Result<String> {
    let response = request.call();
    Ok(match response {
        Err(error) => format!("{}", error),
        Ok(response) => response.into_string().context("reading body failed")?,
    })
}

pub(crate) fn construct_url(base: &str, pieces: &[&str]) -> Result<Url, ParseError> {
    let mut url = Url::parse(base)?;

    for piece in pieces {
        url = url.join(&piece)?;
    }

    Ok(url)
}

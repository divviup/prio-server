use anyhow::{anyhow, Context, Result};
use ureq::{Request, Response, SerdeValue};

use crate::gcp_oauth::OauthTokenProvider;

/// Struct containing parameters for send_json_request
#[derive(Debug, Default)]
pub(crate) struct JsonRequestParameters<'a> {
    /// The ureq::Request to be sent
    pub request: Request,
    /// If this field is set, the request with be sent with an "Authorization"
    /// header containing a bearer token obtained from the OauthTokenProvider.
    /// If unset, the request is sent unauthenticated.
    pub token_provider: Option<&'a mut OauthTokenProvider>,
    /// Timeout in milliseconds for connections. If unset, a default timeout of
    /// 10 seconds is applied.
    pub connect_timeout_millis: Option<u64>,
    /// Timeout in milliseconds for reads. If unset, a default timeout of 10
    /// seconds is applied.
    pub read_timeout_millis: Option<u64>,
    /// The JSON body to be sent in the request, constructed with ureq::json!
    pub body: SerdeValue,
}

/// Send the provided request with the provided JSON body. See
/// JsonRequestParameters for more details.
pub(crate) fn send_json_request(mut parameters: JsonRequestParameters) -> Result<Response> {
    let authenticated_request = match &mut parameters.token_provider {
        Some(token_provider) => parameters.request.set(
            "Authorization",
            &format!("Bearer {}", token_provider.ensure_oauth_token()?),
        ),
        None => &mut parameters.request,
    };
    let connect_timeout_millis = parameters.connect_timeout_millis.unwrap_or(10_000);
    let read_timeout_millis = parameters.read_timeout_millis.unwrap_or(10_000);

    Ok(authenticated_request
        // By default, ureq will wait forever to connect or read.
        .timeout_connect(connect_timeout_millis)
        .timeout_read(read_timeout_millis)
        .set("Content-Type", "application/json")
        .send_json(parameters.body))
}

pub(crate) fn get_url(url: &str) -> Result<String> {
    let resp = ureq::get(url)
        // By default, ureq will wait forever to connect or
        // read.
        .timeout_connect(10_000) // ten seconds
        .timeout_read(10_000) // ten seconds
        .call();
    if resp.synthetic_error().is_some() {
        Err(anyhow!(
            "fetching {}: {}",
            url,
            resp.into_synthetic_error().unwrap()
        ))
    } else if !resp.ok() {
        Err(anyhow!("fetching {}: status {}", url, resp.status()))
    } else {
        resp.into_string()
            .context(format!("reading body of {}", url))
    }
}

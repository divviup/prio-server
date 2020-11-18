use anyhow::{anyhow, Context, Result};

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

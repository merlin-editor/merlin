//! `fetch`: HTTPS requests a plugin makes to the hosts its manifest's
//! `permissions.network` lists (plugin-registry.md 12.5). Each runs on a
//! thread of its own; the response is queued with process output and
//! handed to the plugin between editor events.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::bindings::merlin::plugin::types::{FetchRequest, FetchResponse};
use crate::needs::host_allowed;

/// The most a request or response body may be.
pub(crate) const MAX_BODY: u64 = 10 << 20;
/// The most requests one plugin may have open at once.
pub(crate) const MAX_OPEN: usize = 8;
/// Redirects followed, each to a listed host.
const MAX_REDIRECTS: usize = 5;

/// The host of an `https://` URL, lowercased, if it has one and no user
/// name or password in it.
pub(crate) fn https_host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.contains('@') || authority.is_empty() {
        return None;
    }
    let host = match authority.rsplit_once(':') {
        Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) => h,
        _ => authority,
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Checks a request before it starts: HTTPS, to a listed host, a method
/// merlin sends, and a body that isn't too large.
pub(crate) fn check(request: &FetchRequest, allowed: &[String]) -> Result<(), String> {
    let host =
        https_host(&request.url).ok_or_else(|| format!("{} isn't an https:// URL", request.url))?;
    if !host_allowed(allowed, &host) {
        return Err(format!("permission denied: network ({host})"));
    }
    if !matches!(
        request.method.as_str(),
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
    ) {
        return Err(format!("{} isn't a method fetch sends", request.method));
    }
    if request
        .body
        .as_ref()
        .is_some_and(|b| b.len() as u64 > MAX_BODY)
    {
        return Err("the request's body is over 10 MB".into());
    }
    Ok(())
}

/// Runs `request`, following redirects only to hosts in `allowed`. Blocks;
/// call it on a thread of its own.
pub(crate) fn run(request: FetchRequest, allowed: &[String]) -> Result<FetchResponse, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .max_redirects(0)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .build()
        .into();
    let mut url = request.url.clone();
    let mut method = request.method.clone();
    let mut body = request.body.clone();
    for _ in 0..=MAX_REDIRECTS {
        let mut builder = ureq::http::Request::builder()
            .method(method.as_str())
            .uri(url.as_str())
            .header("User-Agent", concat!("merlin/", env!("CARGO_PKG_VERSION")));
        for (k, v) in &request.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }
        let built = builder
            .body(body.clone().unwrap_or_default())
            .map_err(|e| format!("the request can't be made: {e}"))?;
        let mut response = agent.run(built).map_err(|e| format!("{e}"))?;
        let status = response.status().as_u16();
        if (300..400).contains(&status)
            && let Some(location) = response
                .headers()
                .get("location")
                .and_then(|l| l.to_str().ok())
        {
            let next = if location.starts_with("https://") {
                location.to_string()
            } else if location.starts_with('/') {
                let host = https_host(&url).unwrap_or_default();
                format!("https://{host}{location}")
            } else {
                return Err(format!("redirected to {location}, which isn't https"));
            };
            let host = https_host(&next).ok_or_else(|| format!("redirected to {next}"))?;
            if !host_allowed(allowed, &host) {
                return Err(format!(
                    "redirected to {host}, which the plugin may not connect to"
                ));
            }
            url = next;
            // 303, and 301 and 302 after a POST, as browsers do them.
            if status == 303 || ((status == 301 || status == 302) && method == "POST") {
                method = "GET".into();
                body = None;
            }
            continue;
        }
        let headers = response
            .headers()
            .iter()
            .filter_map(|(k, v)| Some((k.to_string(), v.to_str().ok()?.to_string())))
            .collect();
        let mut bytes = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(MAX_BODY + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("reading the response: {e}"))?;
        if bytes.len() as u64 > MAX_BODY {
            return Err("the response is over 10 MB".into());
        }
        return Ok(FetchResponse {
            status,
            headers,
            body: bytes,
        });
    }
    Err("too many redirects".into())
}

/// Counts a plugin's open requests against [`MAX_OPEN`].
#[derive(Clone, Default)]
pub(crate) struct Open(Arc<AtomicUsize>);

impl Open {
    /// Takes a place, or says none is free.
    pub(crate) fn take(&self) -> Option<OpenGuard> {
        let n = self.0.fetch_add(1, Ordering::AcqRel);
        if n >= MAX_OPEN {
            self.0.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(OpenGuard(self.0.clone()))
    }
}

/// Frees its place when the request ends.
pub(crate) struct OpenGuard(Arc<AtomicUsize>);

impl Drop for OpenGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(url: &str) -> FetchRequest {
        FetchRequest {
            method: "GET".into(),
            url: url.into(),
            headers: Vec::new(),
            body: None,
        }
    }

    #[test]
    fn only_https_to_listed_hosts() {
        let allowed = vec!["api.github.com".to_string()];
        assert!(check(&req("https://api.github.com/repos"), &allowed).is_ok());
        assert!(check(&req("https://API.github.com:443/x"), &allowed).is_ok());
        assert!(check(&req("http://api.github.com/"), &allowed).is_err());
        assert!(check(&req("https://evil.com/"), &allowed).is_err());
        assert!(check(&req("https://api.github.com@evil.com/"), &allowed).is_err());
        let mut post = req("https://api.github.com/");
        post.method = "TRACE".into();
        assert!(check(&post, &allowed).is_err());
    }

    #[test]
    fn eight_at_once() {
        let open = Open::default();
        let held: Vec<_> = (0..MAX_OPEN).map(|_| open.take().unwrap()).collect();
        assert!(open.take().is_none());
        drop(held);
        assert!(open.take().is_some());
    }
}

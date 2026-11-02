//! Small blocking HTTP surface so providers can be tested without the network.

use std::time::Duration;

pub(crate) struct HttpResponse {
    pub status: u16,
    pub body: String,
    pub headers: Vec<(String, String)>,
}

pub(crate) trait Http {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse, String>;
    fn post(&self, url: &str, headers: &[(&str, &str)], body: &str)
        -> Result<HttpResponse, String>;
}

pub(crate) struct ReqwestHttp {
    client: reqwest::blocking::Client,
}

impl ReqwestHttp {
    pub fn new() -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(8))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        Self { client }
    }
}

impl Http for ReqwestHttp {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse, String> {
        self.send(self.client.get(url), headers, None)
    }

    fn post(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Result<HttpResponse, String> {
        self.send(self.client.post(url), headers, Some(body))
    }
}

impl ReqwestHttp {
    fn send(
        &self,
        mut builder: reqwest::blocking::RequestBuilder,
        headers: &[(&str, &str)],
        body: Option<&str>,
    ) -> Result<HttpResponse, String> {
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        if let Some(body) = body {
            builder = builder.body(body.to_string());
        }
        let response = builder.send().map_err(|err| sanitize(&err.to_string()))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_string(),
                    value.to_str().unwrap_or("").to_string(),
                )
            })
            .collect();
        let body = response.text().unwrap_or_default();
        Ok(HttpResponse {
            status,
            body,
            headers,
        })
    }
}

#[cfg(test)]
pub(crate) struct ScriptedRoute {
    pub method: &'static str,
    pub url_contains: &'static str,
    pub status: u16,
    pub body: String,
}

#[cfg(test)]
pub(crate) struct ScriptedHttp {
    pub routes: Vec<ScriptedRoute>,
}

#[cfg(test)]
impl Http for ScriptedHttp {
    fn get(&self, url: &str, _headers: &[(&str, &str)]) -> Result<HttpResponse, String> {
        self.find("GET", url)
    }

    fn post(
        &self,
        url: &str,
        _headers: &[(&str, &str)],
        _body: &str,
    ) -> Result<HttpResponse, String> {
        self.find("POST", url)
    }
}

#[cfg(test)]
impl ScriptedHttp {
    fn find(&self, method: &str, url: &str) -> Result<HttpResponse, String> {
        self.routes
            .iter()
            .find(|route| route.method == method && url.contains(route.url_contains))
            .map(|route| HttpResponse {
                status: route.status,
                body: route.body.clone(),
                headers: Vec::new(),
            })
            .ok_or_else(|| format!("no scripted {method} route for {url}"))
    }
}

pub(crate) fn http_failure(context: &str, status: u16, body: &str) -> String {
    let snippet: String = body.chars().take(180).collect();
    let snippet = sanitize(&snippet);
    if snippet.is_empty() {
        format!("{context} failed (HTTP {status})")
    } else {
        format!("{context} failed (HTTP {status}): {snippet}")
    }
}

/// Drop credential-shaped substrings before anything is stored on a snapshot.
pub(crate) fn sanitize(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if rest_starts(bytes, index, b"sk-") {
            out.push_str("sk-redacted");
            index = skip_token_end(bytes, index);
            continue;
        }
        if rest_starts(bytes, index, b"eyJ") {
            out.push_str("jwt-redacted");
            index = skip_token_end(bytes, index);
            continue;
        }
        let ch = bytes[index] as char;
        out.push(ch);
        index += 1;
    }
    out
}

fn rest_starts(bytes: &[u8], index: usize, prefix: &[u8]) -> bool {
    bytes
        .get(index..)
        .is_some_and(|rest| rest.starts_with(prefix))
}

fn skip_token_end(bytes: &[u8], start: usize) -> usize {
    let mut end = start;
    while end < bytes.len() && is_token_byte(bytes[end]) {
        end += 1;
    }
    end
}

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'%' | b'|')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_keys_and_jwts() {
        let raw = "denied sk-proj-abcDEF123456 and eyJhbGciOi.payload.sig";
        let clean = sanitize(raw);
        assert!(!clean.contains("abcDEF"));
        assert!(clean.contains("sk-redacted"));
        assert!(clean.contains("jwt-redacted"));
        assert!(!clean.contains("eyJhbGci"));
    }
}

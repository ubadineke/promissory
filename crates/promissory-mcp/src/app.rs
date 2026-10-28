//! The only thing promissory-mcp talks to: promissory-app's local payer API. Only the endpoints the tools need
//! are reachable from here; nothing can approve, cancel or create orders.

use std::time::Duration;

use serde_json::Value;

/// Default promissory-app address.
pub const DEFAULT_APP: &str = "http://127.0.0.1:8738";

#[derive(Clone)]
pub struct App {
    base: String,
    http: reqwest::Client,
}

/// Accepts only an `http://` URL on a loopback host: promissory-app's API is local by design.
pub fn check_app_url(url: &str) -> Result<String, String> {
    let base = url.trim().trim_end_matches('/');
    let rest = base
        .strip_prefix("http://")
        .ok_or_else(|| format!("--app must be an http:// URL on this machine, got {url}"))?;
    if rest.contains(['/', '?', '#', '@']) {
        return Err(format!("--app must be a bare origin such as {DEFAULT_APP}, got {url}"));
    }
    let host = match rest.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => rest.rsplit_once(':').map_or(rest, |(h, _)| h),
    };
    let loopback = host == "localhost" || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback());
    if !loopback {
        return Err(format!("--app must point at this machine (127.0.0.1, ::1 or localhost), got {url}"));
    }
    Ok(base.to_string())
}

/// An error from promissory-app, with its message.
#[derive(Debug)]
pub struct AppError(pub String);

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl App {
    pub fn new(base: &str) -> Result<Self, String> {
        let base = check_app_url(base)?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { base, http })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// Where the payer approves or rejects a request.
    pub fn approve_url(&self, request_id: i64) -> String {
        format!("{}/requests/{request_id}", self.base)
    }

    async fn read(resp: Result<reqwest::Response, reqwest::Error>) -> Result<Value, AppError> {
        let resp = resp.map_err(|e| {
            AppError(format!("cannot reach promissory-app ({e}); is it running? The payer starts it with `promissory-app`."))
        })?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            return Ok(body);
        }
        let msg = body["error"].as_str().map(str::to_string).unwrap_or_else(|| status.to_string());
        Err(AppError(format!("promissory-app refused ({}): {msg}", status.as_u16())))
    }

    pub async fn get(&self, path: &str) -> Result<Value, AppError> {
        Self::read(self.http.get(format!("{}/api/v1{path}", self.base)).send().await).await
    }

    /// The only write: `POST /requests`, which records a pending request and moves nothing.
    pub async fn create_request(&self, body: &Value) -> Result<Value, AppError> {
        let resp = self
            .http
            .post(format!("{}/api/v1/requests", self.base))
            .header("X-Promissory-App", "1")
            .json(body)
            .send()
            .await;
        Self::read(resp).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_url_must_be_local_http() {
        assert_eq!(check_app_url("http://127.0.0.1:8738/").unwrap(), "http://127.0.0.1:8738");
        assert_eq!(check_app_url("http://localhost:9000").unwrap(), "http://localhost:9000");
        assert_eq!(check_app_url("http://[::1]:8738").unwrap(), "http://[::1]:8738");
        assert_eq!(check_app_url("http://127.0.0.2").unwrap(), "http://127.0.0.2");
        for bad in [
            "https://127.0.0.1:8738",
            "http://example.com:8738",
            "http://10.0.0.5:8738",
            "http://127.0.0.1.evil.com",
            "http://user@127.0.0.1:8738",
            "http://127.0.0.1:8738/api",
            "127.0.0.1:8738",
        ] {
            assert!(check_app_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn approve_url() {
        let app = App::new("http://127.0.0.1:8788/").unwrap();
        assert_eq!(app.approve_url(4), "http://127.0.0.1:8788/requests/4");
    }
}

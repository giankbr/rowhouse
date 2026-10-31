//! Sending a dashboard snapshot (PNG) somewhere: Slack, Discord or any webhook.
//!
//! Secrets (Slack bot token, Discord / webhook URL) live in the OS keychain
//! under `delivery:<id>`; dashboards.json only keeps the kind, the Slack
//! channel and a masked label for the UI.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{Error, Result};

const KEYRING_SERVICE: &str = "rowhouse";
pub const SLACK_API: &str = "https://slack.com/api";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryKind {
    Slack,
    Discord,
    Webhook,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    pub id: String,
    pub kind: DeliveryKind,
    /// Slack channel ID (C0123…)
    #[serde(default)]
    pub channel: Option<String>,
    /// shown in the UI, e.g. "discord.com/…/a1b2" (never the full secret)
    #[serde(default)]
    pub label: String,
}

fn err(msg: impl std::fmt::Display) -> Error {
    Error::Other(msg.to_string())
}

impl Delivery {
    pub fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            return Err(err("delivery without an id"));
        }
        if self.kind == DeliveryKind::Slack
            && self
                .channel
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
        {
            return Err(err("Slack needs a channel ID (C0123…)"));
        }
        Ok(())
    }

    fn entry(&self) -> Result<keyring::Entry> {
        Ok(keyring::Entry::new(
            KEYRING_SERVICE,
            &format!("delivery:{}", self.id),
        )?)
    }

    pub fn secret(&self) -> Result<Option<String>> {
        match self.entry()?.get_password() {
            Ok(s) => Ok(Some(s)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set_secret(&self, secret: Option<&str>) -> Result<()> {
        let e = self.entry()?;
        match secret.map(str::trim).filter(|s| !s.is_empty()) {
            Some(s) => e.set_password(s)?,
            None => match e.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(x) => return Err(x.into()),
            },
        }
        Ok(())
    }
}

/// Check a secret before it's stored, and make the label the UI shows.
pub fn check_secret(kind: DeliveryKind, secret: &str) -> Result<String> {
    let s = secret.trim();
    match kind {
        DeliveryKind::Slack => {
            if !s.starts_with("xoxb-") {
                return Err(err(
                    "use a Slack bot token (xoxb-…) with the files:write scope; incoming webhooks can't post images",
                ));
            }
            Ok(format!("bot token …{}", &s[s.len().saturating_sub(4)..]))
        }
        DeliveryKind::Discord | DeliveryKind::Webhook => {
            let u = url::Url::parse(s).map_err(|_| err("that isn't a URL"))?;
            if !matches!(u.scheme(), "https" | "http") {
                return Err(err("the URL must start with https://"));
            }
            if kind == DeliveryKind::Discord && !u.path().contains("/api/webhooks/") {
                return Err(err(
                    "use a Discord webhook URL (Channel settings → Integrations → Webhooks)",
                ));
            }
            let tail: String = u
                .path()
                .chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            Ok(format!("{}/…{tail}", u.host_str().unwrap_or("")))
        }
    }
}

fn http() -> Result<reqwest::Client> {
    crate::init_crypto();
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| err(e.to_string()))
}

async fn check(res: reqwest::Response, what: &str) -> Result<Value> {
    let status = res.status();
    let text = res.text().await.map_err(|e| err(e.to_string()))?;
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = v["message"].as_str().unwrap_or(text.trim());
        return Err(err(format!("{what}: HTTP {status} {msg}")));
    }
    if v["ok"] == json!(false) {
        return Err(err(format!(
            "{what}: {}",
            v["error"].as_str().unwrap_or("request failed")
        )));
    }
    Ok(v)
}

fn png_part(png: &[u8], filename: &str) -> Result<reqwest::multipart::Part> {
    reqwest::multipart::Part::bytes(png.to_vec())
        .file_name(filename.to_string())
        .mime_str("image/png")
        .map_err(|e| err(e.to_string()))
}

/// Send one PNG. `caption` is the message text next to it.
pub async fn deliver(
    d: &Delivery,
    secret: &str,
    caption: &str,
    filename: &str,
    png: &[u8],
) -> Result<()> {
    let http = http()?;
    match d.kind {
        DeliveryKind::Discord => {
            let form = reqwest::multipart::Form::new()
                .text("payload_json", json!({ "content": caption }).to_string())
                .part("files[0]", png_part(png, filename)?);
            let url = secret.trim();
            let url = if url.contains('?') {
                format!("{url}&wait=true")
            } else {
                format!("{url}?wait=true")
            };
            let res = http
                .post(url)
                .multipart(form)
                .send()
                .await
                .map_err(|e| err(format!("discord: {e}")))?;
            check(res, "discord").await.map(|_| ())
        }
        DeliveryKind::Webhook => {
            let form = reqwest::multipart::Form::new()
                .text(
                    "payload",
                    json!({ "text": caption, "filename": filename, "source": "rowhouse" })
                        .to_string(),
                )
                .part("file", png_part(png, filename)?);
            let res = http
                .post(secret.trim())
                .multipart(form)
                .send()
                .await
                .map_err(|e| err(format!("webhook: {e}")))?;
            check(res, "webhook").await.map(|_| ())
        }
        DeliveryKind::Slack => {
            let api = std::env::var("ROWHOUSE_SLACK_API").unwrap_or_else(|_| SLACK_API.into());
            let token = secret.trim();
            let channel = d.channel.as_deref().unwrap_or("").trim();
            // 1. ask for an upload URL
            let len = png.len().to_string();
            let v = check(
                http.post(format!("{api}/files.getUploadURLExternal"))
                    .bearer_auth(token)
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(format!(
                        "filename={}&length={len}",
                        url::form_urlencoded::byte_serialize(filename.as_bytes())
                            .collect::<String>()
                    ))
                    .send()
                    .await
                    .map_err(|e| err(format!("slack: {e}")))?,
                "slack",
            )
            .await?;
            let (Some(url), Some(file_id)) = (v["upload_url"].as_str(), v["file_id"].as_str())
            else {
                return Err(err("slack: no upload URL in the response"));
            };
            // 2. upload the bytes
            let res = http
                .post(url)
                .multipart(reqwest::multipart::Form::new().part("file", png_part(png, filename)?))
                .send()
                .await
                .map_err(|e| err(format!("slack upload: {e}")))?;
            if !res.status().is_success() {
                return Err(err(format!("slack upload: HTTP {}", res.status())));
            }
            // 3. share it in the channel
            check(
                http.post(format!("{api}/files.completeUploadExternal"))
                    .bearer_auth(token)
                    .json(&json!({
                        "files": [{ "id": file_id, "title": filename }],
                        "channel_id": channel,
                        "initial_comment": caption,
                    }))
                    .send()
                    .await
                    .map_err(|e| err(format!("slack: {e}")))?,
                "slack",
            )
            .await
            .map(|_| ())
        }
    }
}

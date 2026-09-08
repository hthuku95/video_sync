// MNS (Alibaba Cloud Message Service) HTTP client — the cloud SQS-equivalent on
// the Alibaba era (GAP E hybrid queue strategy: MNS for FFmpeg/clipping dispatch +
// Postgres lease-claim for agentic workflows).
//
// Env-gated: `from_env()` returns None when MNS is not configured, so the existing
// SQS path (CLIPPING_SQS_QUEUE_URL) and the Postgres lease-claim worker pool are
// completely untouched. Dispatch preference: MNS → SQS → no-op.
//
// Config:
//   MNS_ENDPOINT         — e.g. https://960066381428.mns.eu-central-1.aliyuncs.com
//   MNS_QUEUE_NAME       — queue to send/receive on
//   MNS_ACCESS_KEY_ID    — falls back to ALIBABA_ACCESS_KEY_ID
//   MNS_ACCESS_KEY_SECRET— falls back to ALIBABA_ACCESS_KEY_SECRET
//
// Signature spec (HMAC-SHA1):
//   https://help.aliyun.com/en/mns/developer-reference/request-protocol-description
//   StringToSign = METHOD\n + Content-MD5\n + Content-Type\n + Date\n
//                + CanonicalizedMNSHeaders + CanonicalizedResource
//   Signature    = Base64(HMAC-SHA1(AccessKeySecret, UTF-8(StringToSign)))
//   Authorization= "MNS " + AccessKeyId + ":" + Signature
//   Headers must include Date (<=15 min skew) and x-mns-version: 2015-06-06.
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chrono::Utc;
use openssl::hash::{hash as openssl_digest, MessageDigest};
use openssl::pkey::PKey;
use openssl::sign::Signer;
use reqwest::Client;
use std::time::Duration;

const MNS_VERSION: &str = "2015-06-06";

#[derive(Debug, Clone)]
pub struct MnsClient {
    client: Client,
    endpoint: String, // e.g. https://960066381428.mns.eu-central-1.aliyuncs.com
    queue_name: String,
    access_key_id: String,
    access_key_secret: String,
}

/// A single message received from an MNS queue.
#[derive(Debug, Clone)]
pub struct MnsMessage {
    pub id: String,
    pub body: String,
    pub receipt_handle: String,
}

impl MnsClient {
    /// Builds a client only when `MNS_ENDPOINT` + `MNS_QUEUE_NAME` are set.
    /// Returns None otherwise so callers can fall back to SQS/no-op.
    pub fn from_env() -> Option<Self> {
        let endpoint = std::env::var("MNS_ENDPOINT").ok().filter(|s| !s.is_empty())?;
        let queue_name = std::env::var("MNS_QUEUE_NAME").ok().filter(|s| !s.is_empty())?;
        let access_key_id = std::env::var("MNS_ACCESS_KEY_ID")
            .ok()
            .or_else(|| std::env::var("ALIBABA_ACCESS_KEY_ID").ok())
            .filter(|s| !s.is_empty())?;
        let access_key_secret = std::env::var("MNS_ACCESS_KEY_SECRET")
            .ok()
            .or_else(|| std::env::var("ALIBABA_ACCESS_KEY_SECRET").ok())
            .filter(|s| !s.is_empty())?;
        Some(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .ok()?,
            endpoint,
            queue_name,
            access_key_id,
            access_key_secret,
        })
    }

    /// True if both MNS endpoint and queue name are configured.
    pub fn is_configured() -> bool {
        std::env::var("MNS_ENDPOINT").ok().filter(|s| !s.is_empty()).is_some()
            && std::env::var("MNS_QUEUE_NAME").ok().filter(|s| !s.is_empty()).is_some()
    }

    /// Builds the Date, Content-MD5 and Authorization headers for a request.
    /// `body` present ⇒ Content-MD5 computed; absent ⇒ empty per the spec.
    fn build_headers(
        &self,
        method: &str,
        resource: &str,
        body: Option<&[u8]>,
        content_type: &str,
    ) -> (String, String, String) {
        let date = Utc::now().format("%a, %d %b %Y %H:%M:%S GMT").to_string();
        let content_md5 = body
            .map(|b| openssl_digest(MessageDigest::md5(), b).ok())
            .flatten()
            .map(|d| B64.encode(d.as_ref()))
            .unwrap_or_default();
        // CanonicalizedMNSHeaders: every x-mns-* header, lowercased name, sorted, key:value\n
        let canonical_headers = format!("x-mns-version:{}\n", MNS_VERSION);
        let string_to_sign = format!(
            "{}\n{}\n{}\n{}\n{}{}",
            method, content_md5, content_type, date, canonical_headers, resource
        );
        let sig = PKey::hmac(self.access_key_secret.as_bytes())
            .ok()
            .and_then(|pkey| {
                Signer::new(MessageDigest::sha1(), &pkey)
                    .ok()
                    .map(|mut s| (s, pkey))
                    .map(|(mut s, _)| {
                        let _ = s.update(string_to_sign.as_bytes());
                        s.sign_to_vec().ok()
                    })
            })
            .flatten()
            .map(|s| B64.encode(s))
            .unwrap_or_default();
        let authorization = format!("MNS {}:{}", self.access_key_id, sig);
        (date, content_md5, authorization)
    }

    /// POST /queues/{queue}/messages — enqueue one message. Returns MessageId.
    pub async fn send_message(&self, body: &str) -> Result<String, String> {
        let resource = format!("/queues/{}/messages", self.queue_name);
        let content_type = "text/plain;charset=utf-8";
        let (date, content_md5, authorization) =
            self.build_headers("POST", &resource, Some(body.as_bytes()), content_type);
        let url = format!("{}{}", self.endpoint, resource);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", authorization)
            .header("Date", date)
            .header("x-mns-version", MNS_VERSION)
            .header("Content-Type", content_type)
            .header("Content-MD5", content_md5)
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| format!("MNS send_message HTTP error: {}", e))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("MNS send_message read error: {}", e))?;
        if !status.is_success() {
            return Err(format!(
                "MNS send_message failed ({}): {}",
                status,
                extract_error(&text)
            ));
        }
        Ok(xml_tag(&text, "MessageId").unwrap_or_default().to_string())
    }

    /// GET /queues/{queue}/messages?numOfMessages=N&waitSeconds=W — long polling
    /// receive. Returns an empty vec on MessageNotExist (404).
    pub async fn receive_message(
        &self,
        max_num: u32,
        wait_seconds: u32,
    ) -> Result<Vec<MnsMessage>, String> {
        let query = format!("numOfMessages={}&waitSeconds={}", max_num, wait_seconds);
        let resource = format!("/queues/{}/messages?{}", self.queue_name, query);
        let (date, _, authorization) = self.build_headers("GET", &resource, None, "");
        let url = format!("{}{}", self.endpoint, resource);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", authorization)
            .header("Date", date)
            .header("x-mns-version", MNS_VERSION)
            // long poll can block up to waitSeconds; give a generous timeout
            .timeout(Duration::from_secs(u64::from(wait_seconds) + 30))
            .send()
            .await
            .map_err(|e| format!("MNS receive_message HTTP error: {}", e))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("MNS receive_message read error: {}", e))?;
        if status.as_u16() == 404 {
            // MessageNotExist — empty queue, nothing to do
            return Ok(Vec::new());
        }
        if !status.is_success() {
            return Err(format!(
                "MNS receive_message failed ({}): {}",
                status,
                extract_error(&text)
            ));
        }
        Ok(parse_messages(&text))
    }

    /// DELETE /queues/{queue}/messages?ReceiptHandle=... — ack a received message.
    pub async fn delete_message(&self, receipt_handle: &str) -> Result<(), String> {
        let enc = urlencoding::encode(receipt_handle);
        let resource = format!("/queues/{}/messages?ReceiptHandle={}", self.queue_name, enc);
        let (date, _, authorization) = self.build_headers("DELETE", &resource, None, "");
        let url = format!("{}{}", self.endpoint, resource);
        let resp = self
            .client
            .delete(&url)
            .header("Authorization", authorization)
            .header("Date", date)
            .header("x-mns-version", MNS_VERSION)
            .send()
            .await
            .map_err(|e| format!("MNS delete_message HTTP error: {}", e))?;
        let status = resp.status();
        if status.as_u16() == 204 || status.as_u16() == 404 {
            // 204 = ok; 404 = already consumed (idempotent)
            return Ok(());
        }
        let text = resp.text().await.unwrap_or_default();
        Err(format!(
            "MNS delete_message failed ({}): {}",
            status,
            extract_error(&text)
        ))
    }

    /// GET /queues/{queue} — queue attributes. Returns active (visible) message count.
    pub async fn active_message_count(&self) -> Result<i64, String> {
        let resource = format!("/queues/{}", self.queue_name);
        let (date, _, authorization) = self.build_headers("GET", &resource, None, "");
        let url = format!("{}{}", self.endpoint, resource);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", authorization)
            .header("Date", date)
            .header("x-mns-version", MNS_VERSION)
            .send()
            .await
            .map_err(|e| format!("MNS active_message_count HTTP error: {}", e))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("MNS active_message_count read error: {}", e))?;
        if !status.is_success() {
            return Err(format!(
                "MNS active_message_count failed ({}): {}",
                status,
                extract_error(&text)
            ));
        }
        xml_tag(&text, "ActiveMessages")
            .and_then(|v| v.trim().parse::<i64>().ok())
            .ok_or_else(|| format!("MNS response missing <ActiveMessages>: {}", text.lines().next().unwrap_or("")))
    }
}

/// Extract the text inside the first <tag>...</tag> in `xml`.
fn xml_tag<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(&xml[start..end])
}

/// Parse a ReceiveMessage XML response into MnsMessage records.
/// MessageBody is Base64-encoded by MNS.
fn parse_messages(xml: &str) -> Vec<MnsMessage> {
    let mut out = Vec::new();
    let mut rest = xml;
    loop {
        // Find each <Message>...</Message> block
        let block_start = match rest.find("<Message>") {
            Some(i) => i,
            None => return out,
        };
        let block_end = match rest[block_start..].find("</Message>") {
            Some(i) => i,
            None => return out,
        };
        let block = &rest[block_start..block_start + block_end];
        let id = xml_tag(block, "MessageId").unwrap_or_default().to_string();
        let body_b64 = xml_tag(block, "MessageBody").unwrap_or_default();
        let body = B64.decode(body_b64.trim()).ok().filter(|b| !b.is_empty())
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_default();
        let receipt_handle = xml_tag(block, "ReceiptHandle").unwrap_or_default().to_string();
        out.push(MnsMessage { id, body, receipt_handle });
        rest = &rest[block_start + block_end + "</Message>".len()..];
    }
}

/// Extract a useful error message out of an MNS error XML body.
fn extract_error(xml: &str) -> String {
    xml_tag(xml, "Message")
        .or_else(|| xml_tag(xml, "Code"))
        .unwrap_or(xml)
        .trim()
        .chars()
        .take(300)
        .collect()
}
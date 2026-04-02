use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;

use crate::models::{
    HealthStatus, SearchError, SearchQuery, SearchResult, SourceHealth,
};
use super::SearchSource;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Configuration for the Slack search source.
///
/// `user_token` must be a user token (xoxp-...) — bot tokens (xoxb-...) do not
/// have permission to use `search.messages`.
#[derive(Debug, Clone)]
pub struct SlackConfig {
    pub user_token: String,
    pub max_results: usize,
    /// Base URL for the Slack API. Defaults to `https://slack.com` in
    /// production; override with a wiremock URL in tests.
    pub base_url: String,
}

impl Default for SlackConfig {
    fn default() -> Self {
        Self {
            user_token: String::new(),
            max_results: 20,
            base_url: "https://slack.com".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Source
// ---------------------------------------------------------------------------

pub struct SlackSource {
    config: SlackConfig,
    client: Client,
}

impl SlackSource {
    pub fn new(config: SlackConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to build HTTP client");
        Self { config, client }
    }

    /// Fetch a full Slack thread and format it as Markdown.
    ///
    /// Calls three Slack API endpoints:
    /// - `conversations.info` — to get the channel name
    /// - `conversations.replies` — to get all messages in the thread
    ///
    /// Returns a Markdown string with the channel name, original message,
    /// thread replies, and a deduplicated participant list.
    pub async fn get_detail_thread(
        &self,
        channel: &str,
        ts: &str,
    ) -> Result<String, SearchError> {
        // --- 1. Fetch channel info ---
        let info_url = format!("{}/api/conversations.info", self.config.base_url);
        let info_resp = self
            .client
            .get(&info_url)
            .header(
                "Authorization",
                format!("Bearer {}", self.config.user_token),
            )
            .query(&[("channel", channel)])
            .send()
            .await
            .map_err(SearchError::Http)?;

        let info_body: SlackConversationInfoResponse =
            info_resp.json().await.map_err(|e| SearchError::Source {
                source_name: "slack".to_string(),
                message: format!("Failed to parse conversations.info response: {e}"),
            })?;

        let channel_name = info_body
            .channel
            .and_then(|c| c.name)
            .unwrap_or_else(|| channel.to_string());

        // --- 2. Fetch thread replies ---
        let replies_url = format!("{}/api/conversations.replies", self.config.base_url);
        let replies_resp = self
            .client
            .get(&replies_url)
            .header(
                "Authorization",
                format!("Bearer {}", self.config.user_token),
            )
            .query(&[("channel", channel), ("ts", ts), ("limit", "200")])
            .send()
            .await
            .map_err(SearchError::Http)?;

        let replies_body: SlackConversationResponse =
            replies_resp.json().await.map_err(|e| SearchError::Source {
                source_name: "slack".to_string(),
                message: format!("Failed to parse conversations.replies response: {e}"),
            })?;

        if !replies_body.ok {
            return Err(SearchError::Source {
                source_name: "slack".to_string(),
                message: format!(
                    "conversations.replies failed: {}",
                    replies_body.error.unwrap_or_else(|| "unknown_error".to_string())
                ),
            });
        }

        let messages = replies_body.messages.unwrap_or_default();

        // --- 3. Build Markdown ---
        let mut md = String::new();

        // Header
        md.push_str(&format!("# Slack Thread in #{channel_name}\n\n"));

        // Original message (first in the list)
        if let Some(first) = messages.first() {
            let user_id = first.user.as_deref().unwrap_or("unknown");
            let text = first.text.as_deref().unwrap_or("");
            let date_str = first
                .ts
                .as_deref()
                .and_then(parse_slack_ts)
                .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
                .unwrap_or_else(|| "unknown date".to_string());

            md.push_str(&format!("**Started by**: {user_id} -- {date_str}\n\n"));
            md.push_str("## Original Message\n\n");
            md.push_str(text);
            md.push_str("\n\n");
        }

        // Replies (skip first message which is the parent)
        let replies: Vec<&SlackConversationMessage> = messages.iter().skip(1).collect();
        md.push_str(&format!("## Thread Replies ({})\n\n", replies.len()));

        for reply in &replies {
            let user_id = reply.user.as_deref().unwrap_or("unknown");
            let text = reply.text.as_deref().unwrap_or("");
            let date_str = reply
                .ts
                .as_deref()
                .and_then(parse_slack_ts)
                .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
                .unwrap_or_else(|| "unknown date".to_string());

            md.push_str(&format!("### {user_id} -- {date_str}\n"));
            md.push_str(text);
            md.push_str("\n\n");
        }

        // Participants (deduplicated, sorted)
        let mut participants: Vec<String> = messages
            .iter()
            .filter_map(|m| m.user.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        participants.sort();

        md.push_str("## Participants\n\n");
        for p in &participants {
            md.push_str(&format!("- {p}\n"));
        }

        Ok(md)
    }
}

// ---------------------------------------------------------------------------
// Slack API response types (private)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct SlackResponse {
    ok: bool,
    error: Option<String>,
    messages: Option<SlackMessages>,
}

#[derive(Debug, Deserialize)]
struct SlackMessages {
    matches: Vec<SlackMatch>,
}

#[derive(Debug, Deserialize)]
struct SlackMatch {
    text: Option<String>,
    permalink: Option<String>,
    channel: Option<SlackChannel>,
    username: Option<String>,
    ts: Option<String>,
    score: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct SlackChannel {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SlackAuthTestResponse {
    ok: bool,
    error: Option<String>,
}

/// Response from conversations.replies and conversations.history
#[derive(Debug, Deserialize)]
struct SlackConversationResponse {
    ok: bool,
    error: Option<String>,
    messages: Option<Vec<SlackConversationMessage>>,
}

#[derive(Debug, Deserialize)]
struct SlackConversationMessage {
    #[serde(rename = "type")]
    msg_type: Option<String>,
    user: Option<String>,
    text: Option<String>,
    ts: Option<String>,
}

/// Response from conversations.info
#[derive(Debug, Deserialize)]
struct SlackConversationInfoResponse {
    ok: bool,
    error: Option<String>,
    channel: Option<SlackChannelInfo>,
}

#[derive(Debug, Deserialize)]
struct SlackChannelInfo {
    id: Option<String>,
    name: Option<String>,
}

// ---------------------------------------------------------------------------
// Trait impl
// ---------------------------------------------------------------------------

#[async_trait]
impl SearchSource for SlackSource {
    fn name(&self) -> &str {
        "slack"
    }

    fn description(&self) -> &str {
        "Slack message search via search.messages API"
    }

    async fn health_check(&self) -> SourceHealth {
        let start = std::time::Instant::now();
        let url = format!("{}/api/auth.test", self.config.base_url);

        let result = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.user_token))
            .send()
            .await;

        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(resp) => {
                if let Ok(body) = resp.json::<SlackAuthTestResponse>().await {
                    if body.ok {
                        SourceHealth {
                            source: "slack".to_string(),
                            status: HealthStatus::Healthy,
                            message: Some("auth.test OK".to_string()),
                            latency_ms: Some(latency_ms),
                        }
                    } else {
                        SourceHealth {
                            source: "slack".to_string(),
                            status: HealthStatus::Unavailable,
                            message: Some(format!(
                                "auth.test failed: {}",
                                body.error.unwrap_or_default()
                            )),
                            latency_ms: Some(latency_ms),
                        }
                    }
                } else {
                    SourceHealth {
                        source: "slack".to_string(),
                        status: HealthStatus::Unavailable,
                        message: Some("Failed to parse auth.test response".to_string()),
                        latency_ms: Some(latency_ms),
                    }
                }
            }
            Err(e) => SourceHealth {
                source: "slack".to_string(),
                status: HealthStatus::Unavailable,
                message: Some(format!("auth.test request failed: {e}")),
                latency_ms: Some(latency_ms),
            },
        }
    }

    async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>, SearchError> {
        let url = format!("{}/api/search.messages", self.config.base_url);

        // Build the query string, appending time filters if provided
        let mut query_text = query.text.clone();
        if let Some(ref after) = query.filters.after {
            query_text.push_str(&format!(" after:{}", after.format("%Y-%m-%d")));
        }
        if let Some(ref before) = query.filters.before {
            query_text.push_str(&format!(" before:{}", before.format("%Y-%m-%d")));
        }

        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.config.user_token))
            .query(&[
                ("query", query_text.as_str()),
                ("count", &self.config.max_results.to_string()),
            ])
            .send()
            .await
            .map_err(SearchError::Http)?;

        // Check for rate limiting (429)
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = resp
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(60);

            return Err(SearchError::RateLimited {
                source_name: "slack".to_string(),
                retry_after_secs: retry_after,
            });
        }

        // Parse the JSON response
        let body: SlackResponse = resp.json().await.map_err(|e| SearchError::Source {
            source_name: "slack".to_string(),
            message: format!("Failed to parse response: {e}"),
        })?;

        // Check the "ok" field
        if !body.ok {
            let error_code = body.error.unwrap_or_else(|| "unknown_error".to_string());

            // Special hint for wrong token type
            if error_code == "not_allowed_token_type" {
                return Err(SearchError::Auth {
                    source_name: "slack".to_string(),
                    message: format!(
                        "{error_code}: search.messages requires a user token (xoxp-...), \
                         not a bot token (xoxb-...). Check your token type."
                    ),
                });
            }

            return Err(SearchError::Source {
                source_name: "slack".to_string(),
                message: error_code,
            });
        }

        // Extract matches
        let matches = match body.messages {
            Some(msgs) => msgs.matches,
            None => return Ok(vec![]),
        };

        if matches.is_empty() {
            return Ok(vec![]);
        }

        // Find max score for normalization
        let max_score = matches
            .iter()
            .filter_map(|m| m.score)
            .fold(0.0_f64, f64::max);

        let results: Vec<SearchResult> = matches
            .into_iter()
            .map(|m| {
                let snippet = m.text.unwrap_or_default();
                let url = m.permalink;
                let timestamp = m.ts.as_deref().and_then(parse_slack_ts);

                // Normalize score to [0.0, 1.0]
                let raw_score = m.score.unwrap_or(0.0);
                let relevance = if max_score > 0.0 && max_score > 1.0 {
                    (raw_score / max_score) as f32
                } else if raw_score >= 0.0 && raw_score <= 1.0 {
                    raw_score as f32
                } else {
                    (raw_score / max_score.max(1.0)) as f32
                };

                let mut metadata = HashMap::new();
                if let Some(ch) = m.channel {
                    if let Some(name) = ch.name {
                        metadata.insert("channel".to_string(), name);
                    }
                }
                if let Some(username) = m.username {
                    metadata.insert("user".to_string(), username);
                }

                // Use a sensible title — first 80 chars of snippet, or "Slack message"
                let title = if snippet.len() > 80 {
                    format!("{}...", &snippet[..80])
                } else if snippet.is_empty() {
                    "Slack message".to_string()
                } else {
                    snippet.clone()
                };

                SearchResult {
                    source: "slack".to_string(),
                    title,
                    snippet,
                    url,
                    timestamp,
                    relevance,
                    metadata,
                }
            })
            .collect();

        Ok(results)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a Slack `ts` field like `"1710700800.123456"` into a `DateTime<Utc>`.
///
/// The integer part is seconds since epoch; the fractional part is sub-second
/// precision (microseconds, typically 6 digits).
fn parse_slack_ts(ts: &str) -> Option<DateTime<Utc>> {
    let val: f64 = ts.parse().ok()?;
    let secs = val.trunc() as i64;
    // Convert fractional seconds to nanoseconds
    let frac = val.fract();
    let nanos = (frac * 1_000_000_000.0).round() as u32;
    DateTime::from_timestamp(secs, nanos)
}

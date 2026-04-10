use regex::Regex;
use std::sync::LazyLock;

static JIRA_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://([^/]+\.atlassian\.net)/browse/([A-Z][A-Z0-9]+-\d+)$").unwrap()
});

// Matches all known Confluence page URL patterns:
// - /wiki/spaces/SPACE/pages/12345/Title
// - /spaces/SPACE/pages/12345/Title           (no /wiki/)
// - /wiki/rest/api/content/12345              (v1 REST)
// - /wiki/api/v2/pages/12345                  (v2 REST)
static CONFLUENCE_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://[^/]+\.atlassian\.net(?:/wiki)?(?:/spaces/[^/]+/pages/(\d+)|/rest/api/content/(\d+)|/api/v2/pages/(\d+))"
    ).unwrap()
});

static SLACK_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://[^/]+\.slack\.com/archives/([A-Z0-9]+)/p(\d+)$").unwrap()
});

static GITHUB_PR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://github\.com/([^/]+)/([^/]+)/pull/(\d+)").unwrap());

static GITHUB_ISSUE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://github\.com/([^/]+)/([^/]+)/issues/(\d+)").unwrap());

static JIRA_KEY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Z][A-Z0-9]+-\d+$").unwrap());

static GITHUB_SHORTHAND_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([a-zA-Z0-9._-]+)#(\d+)$").unwrap());

#[derive(Debug, Clone, PartialEq)]
pub enum SourceType {
    Jira,
    Confluence,
    Slack,
    GitHub,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedIdentifier {
    JiraKey(String),
    JiraUrl {
        base_url: String,
        key: String,
    },
    ConfluencePageId(String),
    ConfluenceTitle {
        title: String,
        space: Option<String>,
    },
    SlackPermalink {
        channel: String,
        ts: String,
    },
    GitHubPR {
        owner: String,
        repo: String,
        number: u64,
    },
    GitHubIssue {
        owner: String,
        repo: String,
        number: u64,
    },
    GitHubShorthand {
        repo: String,
        number: u64,
    },
}

/// Detect the source type and parse the identifier.
/// Returns `None` if the identifier doesn't match any known pattern.
pub fn detect_source(identifier: &str) -> Option<(SourceType, ParsedIdentifier)> {
    let id = identifier.trim();
    if id.is_empty() {
        return None;
    }

    // Priority 1: Atlassian JIRA URL
    if let Some(caps) = JIRA_URL_RE.captures(id) {
        let base_url = format!("https://{}", &caps[1]);
        let key = caps[2].to_string();
        return Some((
            SourceType::Jira,
            ParsedIdentifier::JiraUrl { base_url, key },
        ));
    }

    // Priority 2: Confluence URL (multiple patterns — first non-None group wins)
    if let Some(caps) = CONFLUENCE_URL_RE.captures(id) {
        let page_id = caps
            .get(1)
            .or_else(|| caps.get(2))
            .or_else(|| caps.get(3))
            .map(|m| m.as_str().to_string());
        if let Some(pid) = page_id {
            return Some((
                SourceType::Confluence,
                ParsedIdentifier::ConfluencePageId(pid),
            ));
        }
    }

    // Priority 3: Slack archive URL
    if let Some(caps) = SLACK_URL_RE.captures(id) {
        let channel = caps[1].to_string();
        let raw_ts = &caps[2];
        let ts = if raw_ts.len() > 6 {
            let (secs, micros) = raw_ts.split_at(raw_ts.len() - 6);
            format!("{}.{}", secs, micros)
        } else {
            raw_ts.to_string()
        };
        return Some((
            SourceType::Slack,
            ParsedIdentifier::SlackPermalink { channel, ts },
        ));
    }

    // Priority 4: GitHub PR URL
    if let Some(caps) = GITHUB_PR_RE.captures(id) {
        let owner = caps[1].to_string();
        let repo = caps[2].to_string();
        let number: u64 = caps[3].parse().ok()?;
        return Some((
            SourceType::GitHub,
            ParsedIdentifier::GitHubPR {
                owner,
                repo,
                number,
            },
        ));
    }

    // Priority 5: GitHub Issue URL
    if let Some(caps) = GITHUB_ISSUE_RE.captures(id) {
        let owner = caps[1].to_string();
        let repo = caps[2].to_string();
        let number: u64 = caps[3].parse().ok()?;
        return Some((
            SourceType::GitHub,
            ParsedIdentifier::GitHubIssue {
                owner,
                repo,
                number,
            },
        ));
    }

    // Priority 6: JIRA key pattern (2+ uppercase letters, dash, 1+ digits)
    if JIRA_KEY_RE.is_match(id) {
        return Some((SourceType::Jira, ParsedIdentifier::JiraKey(id.to_string())));
    }

    None
}

/// Force-interpret an identifier as a specific source type.
/// Used when the caller provides an explicit `source` parameter.
pub fn force_source(identifier: &str, source: &str) -> Option<(SourceType, ParsedIdentifier)> {
    let id = identifier.trim();
    match source {
        "jira" => detect_source(id)
            .filter(|(st, _)| matches!(st, SourceType::Jira))
            .or_else(|| Some((SourceType::Jira, ParsedIdentifier::JiraKey(id.to_string())))),
        "confluence" => detect_source(id)
            .filter(|(st, _)| matches!(st, SourceType::Confluence))
            .or_else(|| {
                Some((
                    SourceType::Confluence,
                    ParsedIdentifier::ConfluenceTitle {
                        title: id.to_string(),
                        space: None,
                    },
                ))
            }),
        "slack" => detect_source(id).filter(|(st, _)| matches!(st, SourceType::Slack)),
        "github" => {
            detect_source(id)
                .filter(|(st, _)| matches!(st, SourceType::GitHub))
                .or_else(|| {
                    // Try repo#number shorthand
                    if let Some(caps) = GITHUB_SHORTHAND_RE.captures(id) {
                        let repo = caps[1].to_string();
                        let number: u64 = caps[2].parse().ok()?;
                        Some((
                            SourceType::GitHub,
                            ParsedIdentifier::GitHubShorthand { repo, number },
                        ))
                    } else {
                        None
                    }
                })
        }
        _ => None,
    }
}

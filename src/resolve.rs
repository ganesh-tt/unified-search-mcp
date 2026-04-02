use regex::Regex;

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
    JiraUrl { base_url: String, key: String },
    ConfluencePageId(String),
    ConfluenceTitle { title: String, space: Option<String> },
    SlackPermalink { channel: String, ts: String },
    GitHubPR { owner: String, repo: String, number: u64 },
    GitHubIssue { owner: String, repo: String, number: u64 },
    GitHubShorthand { repo: String, number: u64 },
}

/// Detect the source type and parse the identifier.
/// Returns `None` if the identifier doesn't match any known pattern.
pub fn detect_source(identifier: &str) -> Option<(SourceType, ParsedIdentifier)> {
    let id = identifier.trim();
    if id.is_empty() {
        return None;
    }

    // Priority 1: Atlassian JIRA URL
    let jira_url_re = Regex::new(
        r"^https?://([^/]+\.atlassian\.net)/browse/([A-Z][A-Z0-9]+-\d+)$"
    ).ok()?;
    if let Some(caps) = jira_url_re.captures(id) {
        let base_url = format!("https://{}", &caps[1]);
        let key = caps[2].to_string();
        return Some((SourceType::Jira, ParsedIdentifier::JiraUrl { base_url, key }));
    }

    // Priority 2: Confluence URL
    let confluence_url_re = Regex::new(
        r"^https?://[^/]+\.atlassian\.net/wiki/spaces/[^/]+/pages/(\d+)"
    ).ok()?;
    if let Some(caps) = confluence_url_re.captures(id) {
        let page_id = caps[1].to_string();
        return Some((SourceType::Confluence, ParsedIdentifier::ConfluencePageId(page_id)));
    }

    // Priority 3: Slack archive URL
    let slack_url_re = Regex::new(
        r"^https?://[^/]+\.slack\.com/archives/([A-Z0-9]+)/p(\d+)$"
    ).ok()?;
    if let Some(caps) = slack_url_re.captures(id) {
        let channel = caps[1].to_string();
        let raw_ts = &caps[2];
        let ts = if raw_ts.len() > 6 {
            let (secs, micros) = raw_ts.split_at(raw_ts.len() - 6);
            format!("{}.{}", secs, micros)
        } else {
            raw_ts.to_string()
        };
        return Some((SourceType::Slack, ParsedIdentifier::SlackPermalink { channel, ts }));
    }

    // Priority 4: GitHub PR URL
    let github_pr_re = Regex::new(
        r"^https?://github\.com/([^/]+)/([^/]+)/pull/(\d+)"
    ).ok()?;
    if let Some(caps) = github_pr_re.captures(id) {
        let owner = caps[1].to_string();
        let repo = caps[2].to_string();
        let number: u64 = caps[3].parse().ok()?;
        return Some((SourceType::GitHub, ParsedIdentifier::GitHubPR { owner, repo, number }));
    }

    // Priority 5: GitHub Issue URL
    let github_issue_re = Regex::new(
        r"^https?://github\.com/([^/]+)/([^/]+)/issues/(\d+)"
    ).ok()?;
    if let Some(caps) = github_issue_re.captures(id) {
        let owner = caps[1].to_string();
        let repo = caps[2].to_string();
        let number: u64 = caps[3].parse().ok()?;
        return Some((SourceType::GitHub, ParsedIdentifier::GitHubIssue { owner, repo, number }));
    }

    // Priority 6: JIRA key pattern (2+ uppercase letters, dash, 1+ digits)
    let jira_key_re = Regex::new(r"^[A-Z][A-Z0-9]+-\d+$").ok()?;
    if jira_key_re.is_match(id) {
        return Some((SourceType::Jira, ParsedIdentifier::JiraKey(id.to_string())));
    }

    None
}

/// Force-interpret an identifier as a specific source type.
/// Used when the caller provides an explicit `source` parameter.
pub fn force_source(identifier: &str, source: &str) -> Option<(SourceType, ParsedIdentifier)> {
    let id = identifier.trim();
    match source {
        "jira" => {
            detect_source(id)
                .filter(|(st, _)| matches!(st, SourceType::Jira))
                .or_else(|| Some((SourceType::Jira, ParsedIdentifier::JiraKey(id.to_string()))))
        }
        "confluence" => {
            detect_source(id)
                .filter(|(st, _)| matches!(st, SourceType::Confluence))
                .or_else(|| Some((
                    SourceType::Confluence,
                    ParsedIdentifier::ConfluenceTitle {
                        title: id.to_string(),
                        space: None,
                    },
                )))
        }
        "slack" => {
            detect_source(id)
                .filter(|(st, _)| matches!(st, SourceType::Slack))
        }
        "github" => {
            detect_source(id)
                .filter(|(st, _)| matches!(st, SourceType::GitHub))
                .or_else(|| {
                    // Try repo#number shorthand
                    let shorthand_re = Regex::new(r"^([a-zA-Z0-9._-]+)#(\d+)$").ok()?;
                    if let Some(caps) = shorthand_re.captures(id) {
                        let repo = caps[1].to_string();
                        let number: u64 = caps[2].parse().ok()?;
                        Some((SourceType::GitHub, ParsedIdentifier::GitHubShorthand { repo, number }))
                    } else {
                        None
                    }
                })
        }
        _ => None,
    }
}

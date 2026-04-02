use unified_search_mcp::resolve::{detect_source, force_source, SourceType, ParsedIdentifier};

#[test]
fn detects_jira_key() {
    let (source_type, parsed) = detect_source("FIN-1234").expect("Should detect JIRA key");
    assert!(matches!(source_type, SourceType::Jira));
    assert!(matches!(parsed, ParsedIdentifier::JiraKey(ref k) if k == "FIN-1234"));
}

#[test]
fn detects_jira_url() {
    let (source_type, parsed) =
        detect_source("https://tookitaki.atlassian.net/browse/FIN-1234")
            .expect("Should detect JIRA URL");
    assert!(matches!(source_type, SourceType::Jira));
    match parsed {
        ParsedIdentifier::JiraUrl { key, .. } => assert_eq!(key, "FIN-1234"),
        other => panic!("Expected JiraUrl, got {:?}", other),
    }
}

#[test]
fn detects_confluence_url() {
    let (source_type, parsed) =
        detect_source("https://tookitaki.atlassian.net/wiki/spaces/PROD/pages/123456/Page+Title")
            .expect("Should detect Confluence URL");
    assert!(matches!(source_type, SourceType::Confluence));
    match parsed {
        ParsedIdentifier::ConfluencePageId(id) => assert_eq!(id, "123456"),
        other => panic!("Expected ConfluencePageId, got {:?}", other),
    }
}

#[test]
fn detects_slack_permalink() {
    let (source_type, parsed) =
        detect_source("https://tookitaki.slack.com/archives/C06ABC123/p1712000000123456")
            .expect("Should detect Slack permalink");
    assert!(matches!(source_type, SourceType::Slack));
    match parsed {
        ParsedIdentifier::SlackPermalink { channel, ts } => {
            assert_eq!(channel, "C06ABC123");
            assert_eq!(ts, "1712000000.123456");
        }
        other => panic!("Expected SlackPermalink, got {:?}", other),
    }
}

#[test]
fn returns_none_for_unrecognized() {
    assert!(detect_source("just some random text").is_none());
    assert!(detect_source("").is_none());
    assert!(detect_source("https://google.com").is_none());
}

#[test]
fn jira_key_various_formats() {
    assert!(detect_source("PLAT-42").is_some());
    assert!(detect_source("A-1").is_none()); // Single letter — not valid
    assert!(detect_source("fin-1234").is_none()); // Lowercase — not valid
    assert!(detect_source("FIN-0").is_some());
}

#[test]
fn slack_permalink_ts_parsing() {
    let (_, parsed) =
        detect_source("https://foo.slack.com/archives/C123/p1712000000123456").unwrap();
    match parsed {
        ParsedIdentifier::SlackPermalink { ts, .. } => {
            assert_eq!(ts, "1712000000.123456");
        }
        other => panic!("Expected SlackPermalink, got {:?}", other),
    }
}

#[test]
fn force_source_jira_with_non_key_identifier() {
    let result = force_source("some random text", "jira");
    assert!(result.is_some());
    let (st, parsed) = result.unwrap();
    assert!(matches!(st, SourceType::Jira));
    assert!(matches!(parsed, ParsedIdentifier::JiraKey(ref k) if k == "some random text"));
}

#[test]
fn force_source_confluence_title() {
    let result = force_source("My Page Title", "confluence");
    assert!(result.is_some());
    let (st, parsed) = result.unwrap();
    assert!(matches!(st, SourceType::Confluence));
    match parsed {
        ParsedIdentifier::ConfluenceTitle { title, space } => {
            assert_eq!(title, "My Page Title");
            assert!(space.is_none());
        }
        other => panic!("Expected ConfluenceTitle, got {:?}", other),
    }
}

#[test]
fn force_source_slack_without_url_returns_none() {
    let result = force_source("some text", "slack");
    assert!(result.is_none()); // Slack requires a parseable URL
}

#[test]
fn force_source_unknown_returns_none() {
    assert!(force_source("FIN-1234", "unknown_source").is_none());
}

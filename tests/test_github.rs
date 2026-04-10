use std::io::Write;
use std::os::unix::fs::PermissionsExt;

use tempfile::NamedTempFile;

use unified_search_mcp::models::*;
use unified_search_mcp::sources::github::{GitHubConfig, GitHubSource};
use unified_search_mcp::sources::SearchSource;

// ===========================================================================
// Helpers
// ===========================================================================

/// Create an executable shell script that routes responses based on the gh
/// API endpoint being called. Supports search/issues, search/code, auth,
/// and individual resource endpoints (for get_detail tests).
fn make_detail_gh_script(endpoint_responses: &[(&str, &str)]) -> NamedTempFile {
    let mut script = NamedTempFile::new().expect("Failed to create temp script");

    let mut case_arms = String::new();
    for (pattern, response) in endpoint_responses {
        // Use a unique heredoc label per arm to avoid collisions
        let label = format!(
            "RESP_{}",
            pattern
                .replace('/', "_")
                .replace('.', "_")
                .replace('*', "STAR")
                .to_uppercase()
        );
        case_arms.push_str(&format!(
            r#"    *{pattern}*)
        cat << '{label}'
{response}
{label}
        exit 0
        ;;
"#,
            pattern = pattern,
            label = label,
            response = response,
        ));
    }

    writeln!(
        script,
        r#"#!/bin/bash
ARGS="$*"
case "$ARGS" in
{case_arms}
    *)
        echo "Unknown endpoint: $ARGS" >&2
        exit 1
        ;;
esac
"#,
        case_arms = case_arms,
    )
    .expect("Failed to write script");

    let path = script.path().to_path_buf();
    let mut perms = std::fs::metadata(&path)
        .expect("Failed to read metadata")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).expect("Failed to set permissions");

    script
}

/// Create an executable shell script in a temp file that echoes the given
/// content to stdout and exits with the given code. The script inspects its
/// arguments to route responses for different `gh` subcommands.
fn make_gh_script(
    issues_json: Option<&str>,
    code_json: Option<&str>,
    auth_exit_code: Option<i32>,
    auth_stderr: Option<&str>,
) -> NamedTempFile {
    let mut script = NamedTempFile::new().expect("Failed to create temp script");

    let issues_response = issues_json.unwrap_or(r#"{"total_count":0,"items":[]}"#);
    let code_response = code_json.unwrap_or(r#"{"total_count":0,"items":[]}"#);
    let auth_exit = auth_exit_code.unwrap_or(0);
    let auth_err = auth_stderr.unwrap_or("");

    // Write a script that checks arguments to determine which response to give
    writeln!(
        script,
        r#"#!/bin/bash
# Fake gh CLI for testing
ARGS="$@"

if echo "$ARGS" | grep -q "auth status"; then
    if [ {auth_exit} -ne 0 ]; then
        echo "{auth_err}" >&2
        exit {auth_exit}
    fi
    echo "Logged in to github.com"
    exit 0
fi

if echo "$ARGS" | grep -q "search/issues"; then
    cat << 'ISSUES_EOF'
{issues_response}
ISSUES_EOF
    exit 0
fi

if echo "$ARGS" | grep -q "search/code"; then
    cat << 'CODE_EOF'
{code_response}
CODE_EOF
    exit 0
fi

echo "Unknown command: $ARGS" >&2
exit 1
"#,
        auth_exit = auth_exit,
        auth_err = auth_err,
        issues_response = issues_response,
        code_response = code_response,
    )
    .expect("Failed to write script");

    // Make the script executable
    let path = script.path().to_path_buf();
    let mut perms = std::fs::metadata(&path)
        .expect("Failed to read metadata")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).expect("Failed to set permissions");

    script
}

fn make_config(gh_path: &str) -> GitHubConfig {
    GitHubConfig {
        orgs: vec!["tookitaki".to_string()],
        repos: vec![],
        max_results: 20,
        gh_path: gh_path.to_string(),
    }
}

fn make_query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.to_string(),
        max_results: 20,
        filters: SearchFilters::default(),
    }
}

// ===========================================================================
// Test 1: search_returns_issues_and_prs
// ===========================================================================

#[tokio::test]
async fn search_returns_issues_and_prs() {
    let issues_json = r#"{
        "total_count": 2,
        "items": [
            {
                "number": 123,
                "title": "Fix broadcast OOM",
                "body": "The broadcast queue grows unbounded causing OOM in production clusters",
                "html_url": "https://github.com/tookitaki/product-amls/pull/123",
                "state": "open",
                "updated_at": "2026-04-01T10:00:00Z",
                "score": 15.5,
                "pull_request": {"url": "https://api.github.com/repos/tookitaki/product-amls/pulls/123"},
                "repository_url": "https://api.github.com/repos/tookitaki/product-amls"
            },
            {
                "number": 456,
                "title": "Add retry logic",
                "body": "Retries for flaky API calls",
                "html_url": "https://github.com/tookitaki/product-amls/issues/456",
                "state": "closed",
                "updated_at": "2026-03-28T08:00:00Z",
                "score": 10.2,
                "repository_url": "https://api.github.com/repos/tookitaki/product-amls"
            }
        ]
    }"#;

    let script = make_gh_script(Some(issues_json), None, None, None);
    let config = make_config(script.path().to_str().unwrap());
    let source = GitHubSource::new(config);

    let results = source.search(&make_query("broadcast OOM")).await.unwrap();

    // Should have 2 issues/PRs (code search returns empty)
    assert_eq!(results.len(), 2);

    // First result (PR)
    assert_eq!(results[0].source, "github");
    assert!(results[0].title.contains("product-amls#123"));
    assert!(results[0].title.contains("Fix broadcast OOM"));
    assert!(results[0].title.contains("[PR]"));
    assert_eq!(
        results[0].url.as_deref(),
        Some("https://github.com/tookitaki/product-amls/pull/123")
    );
    assert!(results[0]
        .snippet
        .contains("broadcast queue grows unbounded"));
    assert_eq!(results[0].metadata.get("kind").unwrap(), "PR");
    assert_eq!(results[0].metadata.get("state").unwrap(), "open");
    assert!(results[0].timestamp.is_some());

    // Second result (Issue, no pull_request field)
    assert!(results[1].title.contains("#456"));
    assert!(results[1].title.contains("[Issue]"));
    assert_eq!(results[1].metadata.get("kind").unwrap(), "Issue");
    assert_eq!(results[1].metadata.get("state").unwrap(), "closed");

    // Relevance: first should be higher (score 15.5 vs 10.2)
    assert!(results[0].relevance > results[1].relevance);
    // Both should be in [0.0, 1.0]
    for r in &results {
        assert!(r.relevance >= 0.0 && r.relevance <= 1.0);
    }
}

// ===========================================================================
// Test 2: search_returns_code_results
// ===========================================================================

#[tokio::test]
async fn search_returns_code_results() {
    let code_json = r#"{
        "total_count": 1,
        "items": [
            {
                "name": "broadcast.rs",
                "path": "src/engine/broadcast.rs",
                "html_url": "https://github.com/tookitaki/product-amls/blob/main/src/engine/broadcast.rs",
                "repository": {"full_name": "tookitaki/product-amls"},
                "score": 8.0
            }
        ]
    }"#;

    let script = make_gh_script(None, Some(code_json), None, None);
    let config = make_config(script.path().to_str().unwrap());
    let source = GitHubSource::new(config);

    let results = source.search(&make_query("broadcast")).await.unwrap();

    // Should have 1 code result (issues search returns empty)
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].source, "github");
    assert_eq!(
        results[0].title,
        "tookitaki/product-amls: src/engine/broadcast.rs"
    );
    assert_eq!(results[0].snippet, "src/engine/broadcast.rs");
    assert_eq!(results[0].metadata.get("kind").unwrap(), "code");
    assert_eq!(
        results[0].metadata.get("repo").unwrap(),
        "tookitaki/product-amls"
    );
    assert_eq!(results[0].metadata.get("file").unwrap(), "broadcast.rs");
    assert!(results[0].url.is_some());
}

// ===========================================================================
// Test 3: search_returns_empty_for_no_matches
// ===========================================================================

#[tokio::test]
async fn search_returns_empty_for_no_matches() {
    let script = make_gh_script(None, None, None, None);
    let config = make_config(script.path().to_str().unwrap());
    let source = GitHubSource::new(config);

    let results = source
        .search(&make_query("xyznonexistent12345"))
        .await
        .unwrap();

    assert_eq!(results.len(), 0);
}

// ===========================================================================
// Test 4: search_combines_issues_and_code
// ===========================================================================

#[tokio::test]
async fn search_combines_issues_and_code() {
    let issues_json = r#"{
        "total_count": 1,
        "items": [
            {
                "number": 789,
                "title": "OOM fix",
                "body": "Fix the OOM",
                "html_url": "https://github.com/tookitaki/product-amls/issues/789",
                "state": "open",
                "updated_at": "2026-04-01T10:00:00Z",
                "score": 12.0,
                "repository_url": "https://api.github.com/repos/tookitaki/product-amls"
            }
        ]
    }"#;

    let code_json = r#"{
        "total_count": 1,
        "items": [
            {
                "name": "oom.rs",
                "path": "src/oom.rs",
                "html_url": "https://github.com/tookitaki/product-amls/blob/main/src/oom.rs",
                "repository": {"full_name": "tookitaki/product-amls"},
                "score": 5.0
            }
        ]
    }"#;

    let script = make_gh_script(Some(issues_json), Some(code_json), None, None);
    let config = make_config(script.path().to_str().unwrap());
    let source = GitHubSource::new(config);

    let results = source.search(&make_query("OOM")).await.unwrap();

    assert_eq!(results.len(), 2);

    // Should have one issue and one code result
    let kinds: Vec<&str> = results
        .iter()
        .map(|r| r.metadata.get("kind").unwrap().as_str())
        .collect();
    assert!(kinds.contains(&"Issue"));
    assert!(kinds.contains(&"code"));
}

// ===========================================================================
// Test 5: health_check_authenticated
// ===========================================================================

#[tokio::test]
async fn health_check_authenticated() {
    let script = make_gh_script(None, None, Some(0), None);
    let config = make_config(script.path().to_str().unwrap());
    let source = GitHubSource::new(config);

    let health = source.health_check().await;

    assert_eq!(health.source, "github");
    assert!(matches!(health.status, HealthStatus::Healthy));
    assert!(health.latency_ms.is_some());
    assert!(health.message.as_ref().unwrap().contains("OK"));
}

// ===========================================================================
// Test 6: health_check_not_authenticated
// ===========================================================================

#[tokio::test]
async fn health_check_not_authenticated() {
    let script = make_gh_script(
        None,
        None,
        Some(1),
        Some("You are not logged into any GitHub hosts. Run gh auth login to authenticate."),
    );
    let config = make_config(script.path().to_str().unwrap());
    let source = GitHubSource::new(config);

    let health = source.health_check().await;

    assert_eq!(health.source, "github");
    assert!(matches!(health.status, HealthStatus::Unavailable));
    assert!(health.latency_ms.is_some());
}

// ===========================================================================
// Test 7: name_and_description
// ===========================================================================

#[tokio::test]
async fn name_and_description() {
    let config = GitHubConfig::default();
    let source = GitHubSource::new(config);

    assert_eq!(source.name(), "github");
    assert!(source.description().contains("GitHub"));
}

// ===========================================================================
// Test 8: scope_qualifier_with_repos
// ===========================================================================

#[tokio::test]
async fn scope_qualifier_with_repos() {
    // When repos are configured, they should take precedence over orgs
    let issues_json = r#"{"total_count":0,"items":[]}"#;

    let script = make_gh_script(Some(issues_json), None, None, None);
    let config = GitHubConfig {
        orgs: vec!["tookitaki".to_string()],
        repos: vec![
            "tookitaki/product-amls".to_string(),
            "tookitaki/product-dss".to_string(),
        ],
        max_results: 10,
        gh_path: script.path().to_str().unwrap().to_string(),
    };
    let source = GitHubSource::new(config);

    // The search should succeed — we are mainly verifying no panic/error
    let results = source.search(&make_query("test")).await.unwrap();
    assert_eq!(results.len(), 0);
}

// ===========================================================================
// Test 9: body_truncated_to_200_chars
// ===========================================================================

#[tokio::test]
async fn body_truncated_to_200_chars() {
    let long_body = "A".repeat(500);
    let issues_json = format!(
        r#"{{
        "total_count": 1,
        "items": [
            {{
                "number": 1,
                "title": "Long body",
                "body": "{}",
                "html_url": "https://github.com/org/repo/issues/1",
                "state": "open",
                "updated_at": "2026-04-01T10:00:00Z",
                "score": 1.0,
                "repository_url": "https://api.github.com/repos/org/repo"
            }}
        ]
    }}"#,
        long_body
    );

    let script = make_gh_script(Some(&issues_json), None, None, None);
    let config = make_config(script.path().to_str().unwrap());
    let source = GitHubSource::new(config);

    let results = source.search(&make_query("test")).await.unwrap();

    assert_eq!(results.len(), 1);
    // Should be truncated to 200 + "..."
    assert_eq!(results[0].snippet.len(), 203);
    assert!(results[0].snippet.ends_with("..."));
}

// ===========================================================================
// Test 10: gh_binary_not_found
// ===========================================================================

#[tokio::test]
async fn gh_binary_not_found() {
    let config = GitHubConfig {
        orgs: vec!["tookitaki".to_string()],
        repos: vec![],
        max_results: 20,
        gh_path: "/nonexistent/path/to/gh".to_string(),
    };
    let source = GitHubSource::new(config);

    // health_check should return Unavailable, not panic
    let health = source.health_check().await;
    assert!(matches!(health.status, HealthStatus::Unavailable));
}

// ===========================================================================
// Test 11: rate_limit_error_from_stderr
// ===========================================================================

#[tokio::test]
async fn rate_limit_error_from_stderr() {
    // Create a script that always fails with rate limit error
    let mut script = NamedTempFile::new().expect("Failed to create temp script");
    writeln!(
        script,
        r#"#!/bin/bash
echo "API rate limit exceeded" >&2
exit 1
"#
    )
    .expect("Failed to write script");

    let path = script.path().to_path_buf();
    let mut perms = std::fs::metadata(&path)
        .expect("Failed to read metadata")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).expect("Failed to set permissions");

    let config = GitHubConfig {
        orgs: vec!["tookitaki".to_string()],
        repos: vec![],
        max_results: 20,
        gh_path: script.path().to_str().unwrap().to_string(),
    };
    let source = GitHubSource::new(config);

    // health_check should capture rate limit
    let health = source.health_check().await;
    assert!(matches!(health.status, HealthStatus::Unavailable));

    // The error message should mention rate limit
    let msg = health.message.unwrap_or_default().to_lowercase();
    assert!(
        msg.contains("rate limit"),
        "Expected rate limit in message, got: {}",
        msg
    );
}

// ===========================================================================
// Test 12: search_with_no_orgs_or_repos
// ===========================================================================

#[tokio::test]
async fn search_with_no_orgs_or_repos() {
    let issues_json = r#"{
        "total_count": 1,
        "items": [
            {
                "number": 1,
                "title": "Global result",
                "body": "Found globally",
                "html_url": "https://github.com/someone/repo/issues/1",
                "state": "open",
                "updated_at": "2026-04-01T10:00:00Z",
                "score": 1.0,
                "repository_url": "https://api.github.com/repos/someone/repo"
            }
        ]
    }"#;

    let script = make_gh_script(Some(issues_json), None, None, None);
    let config = GitHubConfig {
        orgs: vec![],
        repos: vec![],
        max_results: 20,
        gh_path: script.path().to_str().unwrap().to_string(),
    };
    let source = GitHubSource::new(config);

    let results = source.search(&make_query("global test")).await.unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].title.contains("Global result"));
}

// ===========================================================================
// Test 13: get_detail_pr_returns_full_markdown
// ===========================================================================

#[tokio::test]
async fn get_detail_pr_returns_full_markdown() {
    let pr_json = r#"{
        "title": "Fix broadcast OOM in Spark driver",
        "state": "closed",
        "merged_at": "2026-03-30T14:00:00Z",
        "user": {"login": "ganesh-tt"},
        "head": {"ref": "fix/broadcast-oom"},
        "base": {"ref": "develop"},
        "created_at": "2026-03-28T10:00:00Z",
        "updated_at": "2026-03-30T14:00:00Z",
        "additions": 150,
        "deletions": 30,
        "changed_files": 5,
        "body": "This PR fixes the broadcast OOM by adding a bounded queue with backpressure."
    }"#;

    let reviews_json = r#"[
        {
            "user": {"login": "reviewer1"},
            "state": "APPROVED",
            "body": "LGTM, good fix!",
            "submitted_at": "2026-03-29T12:00:00Z"
        },
        {
            "user": {"login": "reviewer2"},
            "state": "CHANGES_REQUESTED",
            "body": "Needs a unit test",
            "submitted_at": "2026-03-29T10:00:00Z"
        }
    ]"#;

    let comments_json = r#"[
        {
            "user": {"login": "reviewer1"},
            "body": "Consider using a ring buffer here",
            "path": "src/engine/broadcast.rs",
            "line": 42,
            "created_at": "2026-03-29T11:00:00Z"
        }
    ]"#;

    let script = make_detail_gh_script(&[
        ("pulls/123/reviews", reviews_json),
        ("pulls/123/comments", comments_json),
        ("pulls/123", pr_json),
    ]);
    let config = make_config(script.path().to_str().unwrap());
    let source = GitHubSource::new(config);

    let md = source
        .get_detail_pr("tookitaki", "product-amls", 123)
        .await
        .unwrap();

    // Title
    assert!(md.contains("tookitaki/product-amls#123: Fix broadcast OOM in Spark driver"));

    // Status — merged_at is present so status should be "Merged"
    assert!(md.contains("| Status | Merged |"));

    // Author
    assert!(md.contains("| Author | ganesh-tt |"));

    // Branch
    assert!(md.contains("| Branch | fix/broadcast-oom → develop |"));

    // Changes
    assert!(md.contains("+150 -30 across 5 files"));

    // Description
    assert!(md.contains("## Description"));
    assert!(md.contains("bounded queue with backpressure"));

    // Reviews
    assert!(md.contains("## Reviews (2)"));
    assert!(md.contains("@reviewer1 — APPROVED"));
    assert!(md.contains("LGTM, good fix!"));
    assert!(md.contains("@reviewer2 — CHANGES_REQUESTED"));
    assert!(md.contains("Needs a unit test"));

    // Review comments
    assert!(md.contains("## Review Comments (1)"));
    assert!(md.contains("@reviewer1 on src/engine/broadcast.rs:42"));
    assert!(md.contains("Consider using a ring buffer here"));
}

// ===========================================================================
// Test 14: get_detail_issue_returns_full_markdown
// ===========================================================================

#[tokio::test]
async fn get_detail_issue_returns_full_markdown() {
    let issue_json = r#"{
        "title": "Spark driver OOM on large datasets",
        "state": "open",
        "user": {"login": "ganesh-tt"},
        "created_at": "2026-03-25T09:00:00Z",
        "updated_at": "2026-03-28T10:00:00Z",
        "body": "When processing datasets > 10GB, the Spark driver runs out of memory.",
        "labels": [
            {"name": "bug"},
            {"name": "priority:high"}
        ]
    }"#;

    let comments_json = r#"[
        {
            "user": {"login": "teammate1"},
            "body": "I can reproduce this on the performance cluster.",
            "created_at": "2026-03-26T10:00:00Z"
        },
        {
            "user": {"login": "ganesh-tt"},
            "body": "Root cause: unbounded broadcast queue. Will submit a fix PR.",
            "created_at": "2026-03-27T15:00:00Z"
        }
    ]"#;

    let script = make_detail_gh_script(&[
        ("issues/456/comments", comments_json),
        ("issues/456", issue_json),
    ]);
    let config = make_config(script.path().to_str().unwrap());
    let source = GitHubSource::new(config);

    let md = source
        .get_detail_issue("tookitaki", "product-amls", 456)
        .await
        .unwrap();

    // Title
    assert!(md.contains("tookitaki/product-amls#456: Spark driver OOM on large datasets"));

    // Status
    assert!(md.contains("| Status | Open |"));

    // Author
    assert!(md.contains("| Author | ganesh-tt |"));

    // Labels
    assert!(md.contains("| Labels | bug, priority:high |"));

    // Description
    assert!(md.contains("## Description"));
    assert!(md.contains("datasets > 10GB"));

    // Comments
    assert!(md.contains("## Comments (2)"));
    assert!(md.contains("@teammate1"));
    assert!(md.contains("reproduce this on the performance cluster"));
    assert!(md.contains("@ganesh-tt"));
    assert!(md.contains("unbounded broadcast queue"));
}

// ===========================================================================
// Test 15: get_detail_pr_open_status
// ===========================================================================

#[tokio::test]
async fn get_detail_pr_open_status() {
    let pr_json = r#"{
        "title": "WIP: New feature",
        "state": "open",
        "merged_at": null,
        "user": {"login": "dev1"},
        "head": {"ref": "feature/new"},
        "base": {"ref": "develop"},
        "created_at": "2026-04-01T10:00:00Z",
        "updated_at": "2026-04-01T10:00:00Z",
        "additions": 10,
        "deletions": 2,
        "changed_files": 1,
        "body": "Work in progress"
    }"#;

    let script = make_detail_gh_script(&[
        ("pulls/99/reviews", "[]"),
        ("pulls/99/comments", "[]"),
        ("pulls/99", pr_json),
    ]);
    let config = make_config(script.path().to_str().unwrap());
    let source = GitHubSource::new(config);

    let md = source.get_detail_pr("org", "repo", 99).await.unwrap();

    // Open PR should show "Open", not "Merged"
    assert!(md.contains("| Status | Open |"));
    // Should NOT contain Merged row
    assert!(!md.contains("| Merged |"));
    // Empty reviews and comments
    assert!(md.contains("## Reviews (0)"));
    assert!(md.contains("## Review Comments (0)"));
}

// ===========================================================================
// Test 16: get_detail_issue_no_labels
// ===========================================================================

#[tokio::test]
async fn get_detail_issue_no_labels() {
    let issue_json = r#"{
        "title": "Simple issue",
        "state": "closed",
        "user": {"login": "dev1"},
        "created_at": "2026-04-01T10:00:00Z",
        "updated_at": "2026-04-01T10:00:00Z",
        "body": "A simple issue without labels.",
        "labels": []
    }"#;

    let script = make_detail_gh_script(&[("issues/1/comments", "[]"), ("issues/1", issue_json)]);
    let config = make_config(script.path().to_str().unwrap());
    let source = GitHubSource::new(config);

    let md = source.get_detail_issue("org", "repo", 1).await.unwrap();

    assert!(md.contains("| Status | Closed |"));
    // No labels row when labels array is empty
    assert!(!md.contains("| Labels |"));
    assert!(md.contains("## Comments (0)"));
}

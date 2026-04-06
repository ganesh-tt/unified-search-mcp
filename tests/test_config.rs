use std::io::Write;
use tempfile::NamedTempFile;
use unified_search_mcp::config;
use unified_search_mcp::models::SearchError;

// ============================================================================
// 1. valid_full_config_parses
// ============================================================================
#[test]
fn valid_full_config_parses() {
    // Set env vars that the fixture references
    unsafe {
        std::env::set_var("TEST_SLACK_TOKEN", "xoxp-test-token-123");
        std::env::set_var("TEST_ATLASSIAN_TOKEN", "atlassian-secret-456");
    }

    let path = "fixtures/config/valid_full.yaml";
    let cfg = config::load(path).expect("should parse valid_full.yaml");

    // Server
    assert_eq!(cfg.server.name, "test-server");
    assert_eq!(cfg.server.max_results, 20);
    assert_eq!(cfg.server.timeout_seconds, 10);
    assert_eq!(cfg.server.log_level, "info");

    // Slack
    let slack = cfg.sources.slack.expect("slack should be present");
    assert!(slack.enabled);
    assert_eq!(slack.config.user_token, "xoxp-test-token-123");
    assert!((slack.weight - 1.0).abs() < f32::EPSILON);

    // Confluence
    let conf = cfg.sources.confluence.expect("confluence should be present");
    assert!(conf.enabled);
    assert_eq!(conf.config.base_url, "https://test.atlassian.net");
    assert_eq!(conf.config.email, "test@example.com");
    assert_eq!(conf.config.api_token, "atlassian-secret-456");
    assert_eq!(conf.config.spaces, vec!["DEV"]);

    // Jira
    let jira = cfg.sources.jira.expect("jira should be present");
    assert!(jira.enabled);
    assert_eq!(jira.config.base_url, "https://test.atlassian.net");
    assert_eq!(jira.config.api_token, "atlassian-secret-456");
    assert_eq!(jira.config.projects, vec!["FIN"]);

    // Local text
    let lt = cfg.sources.local_text.expect("local_text should be present");
    assert!(lt.enabled);
    // Path should be tilde-expanded (not start with ~)
    assert!(!lt.config.paths.is_empty());
    assert_eq!(lt.config.include_patterns, vec!["**/*.rs"]);
    assert_eq!(lt.config.exclude_patterns, vec!["**/target/**"]);
}

// ============================================================================
// 2. minimal_config_parses
// ============================================================================
#[test]
fn minimal_config_parses() {
    let path = "fixtures/config/valid_minimal.yaml";
    let cfg = config::load(path).expect("should parse valid_minimal.yaml");

    // Only local_text should be present
    assert!(cfg.sources.local_text.is_some());
    let lt = cfg.sources.local_text.unwrap();
    assert!(lt.enabled);
    assert_eq!(lt.config.paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(), vec!["/tmp/test"]);

    // Other sources should be None
    assert!(cfg.sources.slack.is_none());
    assert!(cfg.sources.confluence.is_none());
    assert!(cfg.sources.jira.is_none());

    // Server defaults should be applied
    assert_eq!(cfg.server.max_results, 20);
    assert_eq!(cfg.server.timeout_seconds, 10);
    assert_eq!(cfg.server.log_level, "info");
    assert_eq!(cfg.server.name, "unified-search");
}

// ============================================================================
// 3. env_var_interpolation
// ============================================================================
#[test]
fn env_var_interpolation() {
    unsafe {
        std::env::set_var("TEST_CONFIG_TOKEN_ABC", "resolved-value-789");
    }

    let yaml = r#"
sources:
  slack:
    enabled: true
    user_token: "${TEST_CONFIG_TOKEN_ABC}"
"#;
    let mut tmp = NamedTempFile::new().unwrap();
    write!(tmp, "{}", yaml).unwrap();

    let cfg = config::load(tmp.path().to_str().unwrap()).expect("should parse");
    let slack = cfg.sources.slack.expect("slack should be present");
    assert_eq!(slack.config.user_token, "resolved-value-789");
}

// ============================================================================
// 4. missing_env_var_errors
// ============================================================================
#[test]
fn missing_env_var_errors() {
    // Make sure this var definitely does not exist
    std::env::remove_var("NONEXISTENT_VAR_12345");

    let path = "fixtures/config/missing_env_var.yaml";
    let result = config::load(path);

    assert!(result.is_err(), "should fail on missing env var");
    let err = result.unwrap_err();
    match &err {
        SearchError::Config(msg) => {
            assert!(
                msg.contains("NONEXISTENT_VAR_12345"),
                "error should name the missing var, got: {}",
                msg
            );
        }
        other => panic!("expected SearchError::Config, got: {:?}", other),
    }
}

// ============================================================================
// 5. invalid_yaml_syntax
// ============================================================================
#[test]
fn invalid_yaml_syntax() {
    let path = "fixtures/config/invalid_syntax.yaml";
    let result = config::load(path);

    assert!(result.is_err(), "should fail on invalid YAML");
    match &result.unwrap_err() {
        SearchError::Config(msg) => {
            // Should contain some context about the parse failure
            assert!(
                !msg.is_empty(),
                "error message should provide context"
            );
        }
        other => panic!("expected SearchError::Config, got: {:?}", other),
    }
}

// ============================================================================
// 6. missing_config_file
// ============================================================================
#[test]
fn missing_config_file() {
    let result = config::load("/tmp/does_not_exist_unified_search_test.yaml");

    assert!(result.is_err(), "should fail on missing file");
    match &result.unwrap_err() {
        SearchError::Config(msg) => {
            assert!(
                msg.contains("config.example.yaml"),
                "error should mention config.example.yaml, got: {}",
                msg
            );
        }
        other => panic!("expected SearchError::Config, got: {:?}", other),
    }
}

// ============================================================================
// 7. disabled_sources_skipped
// ============================================================================
#[test]
fn disabled_sources_skipped() {
    let yaml = r#"
sources:
  slack:
    enabled: false
    user_token: "xoxp-unused"
"#;
    let mut tmp = NamedTempFile::new().unwrap();
    write!(tmp, "{}", yaml).unwrap();

    let cfg = config::load(tmp.path().to_str().unwrap()).expect("should parse");

    // When enabled=false, the source config should either be None or have enabled=false
    match cfg.sources.slack {
        None => {} // acceptable: disabled sources are omitted
        Some(ref slack) => {
            assert!(!slack.enabled, "slack should be disabled");
        }
    }
}

// ============================================================================
// 8. tilde_expansion
// ============================================================================
#[test]
fn tilde_expansion() {
    let yaml = r#"
sources:
  local_text:
    enabled: true
    paths: ["~/projects/repo"]
"#;
    let mut tmp = NamedTempFile::new().unwrap();
    write!(tmp, "{}", yaml).unwrap();

    let cfg = config::load(tmp.path().to_str().unwrap()).expect("should parse");
    let lt = cfg.sources.local_text.expect("local_text should be present");

    let first_path = lt.config.paths[0].display().to_string();
    // Should NOT start with ~ — it should be expanded to real home dir
    assert!(
        !first_path.starts_with('~'),
        "path should be tilde-expanded, got: {}",
        first_path
    );
    assert!(
        first_path.contains("projects/repo"),
        "path should contain the rest of the path, got: {}",
        first_path
    );
}

// ============================================================================
// 9. defaults_applied
// ============================================================================
#[test]
fn defaults_applied() {
    // Config with no server section at all
    let yaml = r#"
sources:
  local_text:
    enabled: true
    paths: ["/tmp/x"]
"#;
    let mut tmp = NamedTempFile::new().unwrap();
    write!(tmp, "{}", yaml).unwrap();

    let cfg = config::load(tmp.path().to_str().unwrap()).expect("should parse");

    assert_eq!(cfg.server.max_results, 20);
    assert_eq!(cfg.server.timeout_seconds, 10);
    assert_eq!(cfg.server.log_level, "info");
    assert_eq!(cfg.server.name, "unified-search");
}

// ============================================================================
// 10. debug_output_redacts_tokens
// ============================================================================
#[test]
fn debug_output_redacts_tokens() {
    use unified_search_mcp::sources::slack::SlackConfig;
    use unified_search_mcp::sources::jira::JiraConfig;
    use unified_search_mcp::sources::confluence::ConfluenceConfig;

    let slack = SlackConfig {
        user_token: "xoxp-secret-token-12345".to_string(),
        max_results: 20,
        base_url: "https://slack.com".to_string(),
    };
    let debug_output = format!("{:?}", slack);
    assert!(
        !debug_output.contains("xoxp-secret"),
        "Debug should not contain token, got: {}",
        debug_output
    );
    assert!(
        debug_output.contains("REDACTED"),
        "Debug should show REDACTED, got: {}",
        debug_output
    );

    let jira = JiraConfig {
        base_url: "https://test.atlassian.net".to_string(),
        email: "user@test.com".to_string(),
        api_token: "secret-api-token-xyz".to_string(),
        projects: vec![],
        max_results: 25,
    };
    let debug_output = format!("{:?}", jira);
    assert!(
        !debug_output.contains("secret-api-token"),
        "Debug should not contain api_token, got: {}",
        debug_output
    );
    assert!(
        debug_output.contains("REDACTED"),
        "Debug should show REDACTED, got: {}",
        debug_output
    );

    let confluence = ConfluenceConfig {
        base_url: "https://test.atlassian.net".to_string(),
        email: "user@test.com".to_string(),
        api_token: "secret-confluence-token".to_string(),
        spaces: vec![],
        max_results: 10,
    };
    let debug_output = format!("{:?}", confluence);
    assert!(
        !debug_output.contains("secret-confluence"),
        "Debug should not contain api_token, got: {}",
        debug_output
    );
    assert!(
        debug_output.contains("REDACTED"),
        "Debug should show REDACTED, got: {}",
        debug_output
    );
}

// ============================================================================
// 11. rejects_http_base_url_for_jira
// ============================================================================
#[test]
fn rejects_http_base_url_for_jira() {
    let config_content = r#"
server:
  name: test
sources:
  jira:
    enabled: true
    base_url: "http://insecure.example.com"
    email: "test@test.com"
    api_token: "token"
"#;
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    std::fs::write(&path, config_content).unwrap();

    let result = unified_search_mcp::config::load(path.to_str().unwrap());
    assert!(result.is_err(), "Should reject http:// base_url");
    let err = result.unwrap_err().to_string();
    assert!(
        err.to_lowercase().contains("https"),
        "Error should mention HTTPS requirement, got: {}",
        err
    );
}

// ============================================================================
// 12. allows_https_base_url
// ============================================================================
#[test]
fn allows_https_base_url() {
    let config_content = r#"
server:
  name: test
sources:
  jira:
    enabled: true
    base_url: "https://secure.atlassian.net"
    email: "test@test.com"
    api_token: "token"
"#;
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    std::fs::write(&path, config_content).unwrap();

    let result = unified_search_mcp::config::load(path.to_str().unwrap());
    assert!(result.is_ok(), "Should accept https:// base_url, got: {:?}", result.err());
}

// ============================================================================
// 13. concurrent_config_loads_dont_interfere
// ============================================================================
#[test]
fn concurrent_config_loads_dont_interfere() {
    use std::io::Write;
    use std::sync::Arc;
    use std::thread;

    // Config A: slack enabled with a missing env var (CONCURRENT_TEST_VAR_A not set)
    let yaml_a = r#"
sources:
  slack:
    enabled: true
    user_token: "${CONCURRENT_TEST_VAR_A}"
"#;

    // Config B: local_text only, no missing env vars at all
    let yaml_b = r#"
sources:
  local_text:
    enabled: true
    paths: ["/tmp/concurrent-test-b"]
"#;

    // Make sure the missing var is definitely absent
    std::env::remove_var("CONCURRENT_TEST_VAR_A");

    let mut tmp_a = NamedTempFile::new().unwrap();
    write!(tmp_a, "{}", yaml_a).unwrap();
    let mut tmp_b = NamedTempFile::new().unwrap();
    write!(tmp_b, "{}", yaml_b).unwrap();

    let path_a = Arc::new(tmp_a.path().to_str().unwrap().to_string());
    let path_b = Arc::new(tmp_b.path().to_str().unwrap().to_string());

    // Run many concurrent pairs of loads to surface any race condition
    let handles: Vec<_> = (0..16).map(|i| {
        let pa = Arc::clone(&path_a);
        let pb = Arc::clone(&path_b);
        thread::spawn(move || {
            if i % 2 == 0 {
                // Config A should fail (missing env var for enabled slack)
                let result_a = config::load(&pa);
                assert!(
                    result_a.is_err(),
                    "thread {}: config A (missing slack token) should fail, got Ok",
                    i
                );
            } else {
                // Config B should succeed and must NOT see config A's missing vars
                let result_b = config::load(&pb);
                assert!(
                    result_b.is_ok(),
                    "thread {}: config B (local_text only) should succeed, got: {:?}",
                    i,
                    result_b.err()
                );
            }
        })
    }).collect();

    for h in handles {
        h.join().expect("thread panicked");
    }
}

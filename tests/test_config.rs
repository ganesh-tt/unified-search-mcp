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

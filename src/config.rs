use std::path::PathBuf;

use regex::Regex;
use serde::Deserialize;

use crate::models::SearchError;
use crate::sources::confluence::ConfluenceConfig;
use crate::sources::github::GitHubConfig;
use crate::sources::jira::JiraConfig;
use crate::sources::local_text::LocalTextConfig;
use crate::sources::slack::SlackConfig;

// ===========================================================================
// Public config types
// ===========================================================================

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub sources: SourcesConfig,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub name: String,
    pub max_results: usize,
    pub timeout_seconds: u64,
    pub log_level: String,
    pub metrics_path: String,
    pub cache_ttl_seconds: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            name: "unified-search".to_string(),
            max_results: 20,
            timeout_seconds: 10,
            log_level: "info".to_string(),
            metrics_path: "~/.unified-search/metrics.jsonl".to_string(),
            cache_ttl_seconds: 300,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SourcesConfig {
    pub slack: Option<SlackSourceConfig>,
    pub confluence: Option<ConfluenceSourceConfig>,
    pub jira: Option<JiraSourceConfig>,
    pub local_text: Option<LocalTextSourceConfig>,
    pub github: Option<GitHubSourceConfig>,
}

#[derive(Debug, Clone)]
pub struct SlackSourceConfig {
    pub enabled: bool,
    pub weight: f32,
    pub config: SlackConfig,
}

#[derive(Debug, Clone)]
pub struct ConfluenceSourceConfig {
    pub enabled: bool,
    pub weight: f32,
    pub config: ConfluenceConfig,
}

#[derive(Debug, Clone)]
pub struct JiraSourceConfig {
    pub enabled: bool,
    pub weight: f32,
    pub config: JiraConfig,
}

#[derive(Debug, Clone)]
pub struct LocalTextSourceConfig {
    pub enabled: bool,
    pub weight: f32,
    pub config: LocalTextConfig,
}

#[derive(Debug, Clone)]
pub struct GitHubSourceConfig {
    pub enabled: bool,
    pub weight: f32,
    pub config: GitHubConfig,
}

// ===========================================================================
// Raw deserialization types (serde_yml)
// ===========================================================================

#[derive(Debug, Deserialize, Default)]
struct RawConfig {
    #[serde(default)]
    server: Option<RawServerConfig>,
    #[serde(default)]
    sources: Option<RawSourcesConfig>,
}

#[derive(Debug, Deserialize)]
struct RawServerConfig {
    name: Option<String>,
    max_results: Option<usize>,
    timeout_seconds: Option<u64>,
    log_level: Option<String>,
    metrics_path: Option<String>,
    cache_ttl_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
struct RawSourcesConfig {
    slack: Option<RawSlackConfig>,
    confluence: Option<RawConfluenceConfig>,
    jira: Option<RawJiraConfig>,
    local_text: Option<RawLocalTextConfig>,
    github: Option<RawGitHubConfig>,
}

#[derive(Debug, Deserialize)]
struct RawSlackConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    user_token: String,
    #[serde(default = "default_weight")]
    weight: f32,
    #[serde(default = "default_source_max_results")]
    max_results: usize,
    #[serde(default = "default_slack_base_url")]
    base_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawConfluenceConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    api_token: String,
    #[serde(default)]
    spaces: Vec<String>,
    #[serde(default = "default_weight")]
    weight: f32,
    #[serde(default = "default_source_max_results")]
    max_results: usize,
}

#[derive(Debug, Deserialize)]
struct RawJiraConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    api_token: String,
    #[serde(default)]
    projects: Vec<String>,
    #[serde(default = "default_weight")]
    weight: f32,
    #[serde(default = "default_source_max_results")]
    max_results: usize,
}

#[derive(Debug, Deserialize)]
struct RawLocalTextConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    include_patterns: Vec<String>,
    #[serde(default)]
    exclude_patterns: Vec<String>,
    #[serde(default = "default_max_file_size")]
    max_file_size_bytes: u64,
    #[serde(default = "default_weight")]
    weight: f32,
    #[serde(default = "default_source_max_results")]
    #[allow(dead_code)]
    max_results: usize,
}

#[derive(Debug, Deserialize)]
struct RawGitHubConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    orgs: Vec<String>,
    #[serde(default)]
    repos: Vec<String>,
    #[serde(default = "default_weight")]
    weight: f32,
    #[serde(default = "default_source_max_results")]
    max_results: usize,
}

fn default_weight() -> f32 {
    1.0
}
fn default_source_max_results() -> usize {
    20
}
fn default_max_file_size() -> u64 {
    10 * 1024 * 1024
}
fn default_slack_base_url() -> Option<String> {
    None
}

// ===========================================================================
// load()
// ===========================================================================

/// Load and parse a YAML config file, interpolating `${ENV_VAR}` references
/// and expanding tilde (`~`) in paths.
pub fn load(path: &str) -> Result<AppConfig, SearchError> {
    // Read file
    let content = std::fs::read_to_string(path).map_err(|e| {
        SearchError::Config(format!(
            "Failed to read config file '{}': {}. See config.example.yaml for a template.",
            path, e
        ))
    })?;

    // Interpolate env vars: ${VAR_NAME} -> value; collect missing var names locally
    let (interpolated, missing_vars) = interpolate_env_vars(&content)?;

    // Parse YAML
    let raw: RawConfig = serde_yml::from_str(&interpolated)
        .map_err(|e| SearchError::Config(format!("Invalid YAML in '{}': {}", path, e)))?;

    // Build AppConfig with defaults
    let server = match raw.server {
        Some(s) => ServerConfig {
            name: s.name.unwrap_or_else(|| "unified-search".to_string()),
            max_results: s.max_results.unwrap_or(20),
            timeout_seconds: s.timeout_seconds.unwrap_or(10),
            log_level: s.log_level.unwrap_or_else(|| "info".to_string()),
            metrics_path: s
                .metrics_path
                .unwrap_or_else(|| "~/.unified-search/metrics.jsonl".to_string()),
            cache_ttl_seconds: s.cache_ttl_seconds.unwrap_or(300),
        },
        None => ServerConfig::default(),
    };

    let sources = match raw.sources {
        Some(raw_sources) => build_sources(raw_sources, &missing_vars)?,
        None => SourcesConfig::default(),
    };

    Ok(AppConfig { server, sources })
}

/// Replace `${VAR_NAME}` patterns with the corresponding environment variable
/// value. Returns the interpolated string and a list of variable names that
/// were referenced but not set in the environment. Missing vars are replaced
/// with an empty string; the caller validates required fields later (only for
/// enabled sources), which avoids erroring on disabled sources whose env vars
/// aren't set.
fn interpolate_env_vars(input: &str) -> Result<(String, Vec<String>), SearchError> {
    let re = Regex::new(r"\$\{([^}]+)\}").expect("env var regex should compile");
    let mut result = input.to_string();
    let mut missing: Vec<String> = Vec::new();

    let captures: Vec<(String, String)> = re
        .captures_iter(input)
        .map(|cap| {
            let full_match = cap.get(0).unwrap().as_str().to_string();
            let var_name = cap.get(1).unwrap().as_str().to_string();
            (full_match, var_name)
        })
        .collect();

    for (full_match, var_name) in captures {
        match std::env::var(&var_name) {
            Ok(value) => {
                result = result.replace(&full_match, &value);
            }
            Err(_) => {
                missing.push(var_name);
                // Replace with empty — validation happens per-source when enabled
                result = result.replace(&full_match, "");
            }
        }
    }

    Ok((result, missing))
}

/// Expand tilde in a path string using shellexpand.
fn expand_tilde(path: &str) -> String {
    shellexpand::tilde(path).to_string()
}

/// Build the SourcesConfig from the raw deserialized data.
fn build_sources(
    raw: RawSourcesConfig,
    missing_vars: &[String],
) -> Result<SourcesConfig, SearchError> {
    let slack = raw.slack.map(|s| SlackSourceConfig {
        enabled: s.enabled,
        weight: s.weight,
        config: SlackConfig {
            user_token: s.user_token,
            max_results: s.max_results,
            base_url: s
                .base_url
                .unwrap_or_else(|| "https://slack.com".to_string()),
        },
    });

    let confluence = raw.confluence.map(|c| ConfluenceSourceConfig {
        enabled: c.enabled,
        weight: c.weight,
        config: ConfluenceConfig {
            base_url: c.base_url,
            email: c.email,
            api_token: c.api_token,
            spaces: c.spaces,
            max_results: c.max_results,
        },
    });

    let jira = raw.jira.map(|j| JiraSourceConfig {
        enabled: j.enabled,
        weight: j.weight,
        config: JiraConfig {
            base_url: j.base_url,
            email: j.email,
            api_token: j.api_token,
            projects: j.projects,
            max_results: j.max_results,
        },
    });

    let local_text = raw.local_text.map(|lt| {
        let paths: Vec<PathBuf> = lt
            .paths
            .iter()
            .map(|p| PathBuf::from(expand_tilde(p)))
            .collect();

        LocalTextSourceConfig {
            enabled: lt.enabled,
            weight: lt.weight,
            config: LocalTextConfig {
                paths,
                include_patterns: lt.include_patterns,
                exclude_patterns: lt.exclude_patterns,
                max_file_size_bytes: lt.max_file_size_bytes,
            },
        }
    });

    let github = raw.github.map(|g| GitHubSourceConfig {
        enabled: g.enabled,
        weight: g.weight,
        config: GitHubConfig {
            orgs: g.orgs,
            repos: g.repos,
            max_results: g.max_results,
            gh_path: "gh".to_string(),
        },
    });

    let config = SourcesConfig {
        slack,
        confluence,
        jira,
        local_text,
        github,
    };

    // Validate: enabled sources must have required fields (non-empty after env var interpolation)
    validate_enabled_sources(&config, missing_vars)?;

    Ok(config)
}

fn validate_url_security(url: &str, source_name: &str) -> Result<(), SearchError> {
    if url.is_empty() {
        return Ok(()); // Empty URLs are handled elsewhere
    }
    // Allow localhost/127.0.0.1 for testing (wiremock)
    if url.contains("127.0.0.1") || url.contains("localhost") {
        return Ok(());
    }
    if !url.starts_with("https://") {
        return Err(SearchError::Config(format!(
            "{} base_url must use HTTPS (got '{}'). Use https:// for security.",
            source_name, url
        )));
    }
    Ok(())
}

fn validate_enabled_sources(
    sources: &SourcesConfig,
    missing: &[String],
) -> Result<(), SearchError> {
    // Validate HTTPS for enabled sources with base_url fields
    if let Some(jira) = &sources.jira {
        if jira.enabled {
            validate_url_security(&jira.config.base_url, "jira")?;
        }
    }
    if let Some(confluence) = &sources.confluence {
        if confluence.enabled {
            validate_url_security(&confluence.config.base_url, "confluence")?;
        }
    }
    if let Some(slack) = &sources.slack {
        if slack.enabled {
            validate_url_security(&slack.config.base_url, "slack")?;
        }
    }

    if missing.is_empty() {
        return Ok(());
    }

    // Check if any missing env var belongs to an enabled source
    // For each missing var, check if the source that uses it is enabled
    for var in missing.iter() {
        let is_slack_var = sources
            .slack
            .as_ref()
            .is_some_and(|s| s.enabled && s.config.user_token.is_empty());
        let is_confluence_var = sources.confluence.as_ref().is_some_and(|c| {
            c.enabled
                && (c.config.base_url.is_empty()
                    || c.config.api_token.is_empty()
                    || c.config.email.is_empty())
        });
        let is_jira_var = sources.jira.as_ref().is_some_and(|j| {
            j.enabled
                && (j.config.base_url.is_empty()
                    || j.config.api_token.is_empty()
                    || j.config.email.is_empty())
        });

        if is_slack_var || is_confluence_var || is_jira_var {
            return Err(SearchError::Config(format!(
                "Environment variable '{}' is not set (referenced in config)",
                var
            )));
        }
    }
    Ok(())
}

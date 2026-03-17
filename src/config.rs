use std::path::PathBuf;

use regex::Regex;
use serde::Deserialize;

use crate::models::SearchError;
use crate::sources::confluence::ConfluenceConfig;
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
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            name: "unified-search".to_string(),
            max_results: 20,
            timeout_seconds: 10,
            log_level: "info".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SourcesConfig {
    pub slack: Option<SlackSourceConfig>,
    pub confluence: Option<ConfluenceSourceConfig>,
    pub jira: Option<JiraSourceConfig>,
    pub local_text: Option<LocalTextSourceConfig>,
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
}

#[derive(Debug, Deserialize, Default)]
struct RawSourcesConfig {
    slack: Option<RawSlackConfig>,
    confluence: Option<RawConfluenceConfig>,
    jira: Option<RawJiraConfig>,
    local_text: Option<RawLocalTextConfig>,
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

    // Interpolate env vars: ${VAR_NAME} -> value
    let interpolated = interpolate_env_vars(&content)?;

    // Parse YAML
    let raw: RawConfig = serde_yml::from_str(&interpolated).map_err(|e| {
        SearchError::Config(format!("Invalid YAML in '{}': {}", path, e))
    })?;

    // Build AppConfig with defaults
    let server = match raw.server {
        Some(s) => ServerConfig {
            name: s.name.unwrap_or_else(|| "unified-search".to_string()),
            max_results: s.max_results.unwrap_or(20),
            timeout_seconds: s.timeout_seconds.unwrap_or(10),
            log_level: s.log_level.unwrap_or_else(|| "info".to_string()),
        },
        None => ServerConfig::default(),
    };

    let sources = match raw.sources {
        Some(raw_sources) => build_sources(raw_sources)?,
        None => SourcesConfig::default(),
    };

    Ok(AppConfig { server, sources })
}

/// Replace `${VAR_NAME}` patterns with the corresponding environment variable
/// value. Returns an error naming the variable if it is not set.
fn interpolate_env_vars(input: &str) -> Result<String, SearchError> {
    let re = Regex::new(r"\$\{([^}]+)\}").expect("env var regex should compile");
    let mut result = input.to_string();

    // Collect all matches first to avoid borrow issues
    let captures: Vec<(String, String)> = re
        .captures_iter(input)
        .map(|cap| {
            let full_match = cap.get(0).unwrap().as_str().to_string();
            let var_name = cap.get(1).unwrap().as_str().to_string();
            (full_match, var_name)
        })
        .collect();

    for (full_match, var_name) in captures {
        let value = std::env::var(&var_name).map_err(|_| {
            SearchError::Config(format!(
                "Environment variable '{}' is not set (referenced in config)",
                var_name
            ))
        })?;
        result = result.replace(&full_match, &value);
    }

    Ok(result)
}

/// Expand tilde in a path string using shellexpand.
fn expand_tilde(path: &str) -> String {
    shellexpand::tilde(path).to_string()
}

/// Build the SourcesConfig from the raw deserialized data.
fn build_sources(raw: RawSourcesConfig) -> Result<SourcesConfig, SearchError> {
    let slack = raw.slack.map(|s| {
        SlackSourceConfig {
            enabled: s.enabled,
            weight: s.weight,
            config: SlackConfig {
                user_token: s.user_token,
                max_results: s.max_results,
                base_url: s.base_url.unwrap_or_else(|| "https://slack.com".to_string()),
            },
        }
    });

    let confluence = raw.confluence.map(|c| {
        ConfluenceSourceConfig {
            enabled: c.enabled,
            weight: c.weight,
            config: ConfluenceConfig {
                base_url: c.base_url,
                email: c.email,
                api_token: c.api_token,
                spaces: c.spaces,
                max_results: c.max_results,
            },
        }
    });

    let jira = raw.jira.map(|j| {
        JiraSourceConfig {
            enabled: j.enabled,
            weight: j.weight,
            config: JiraConfig {
                base_url: j.base_url,
                email: j.email,
                api_token: j.api_token,
                projects: j.projects,
                max_results: j.max_results,
            },
        }
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

    Ok(SourcesConfig {
        slack,
        confluence,
        jira,
        local_text,
    })
}

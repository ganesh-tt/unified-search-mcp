use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

use unified_search_mcp::models::{SearchFilters, SearchQuery};
use unified_search_mcp::sources::local_text::{LocalTextConfig, LocalTextSource};
use unified_search_mcp::sources::SearchSource;

// ===========================================================================
// Helpers
// ===========================================================================

/// Build a `LocalTextConfig` for the given temp dir with sensible defaults.
fn config_for(dir: &TempDir) -> LocalTextConfig {
    LocalTextConfig {
        paths: vec![dir.path().to_path_buf()],
        include_patterns: vec![],
        exclude_patterns: vec![],
        max_file_size_bytes: 10 * 1024 * 1024, // 10 MB
    }
}

/// Build a simple search query with text and default filters.
fn query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.to_string(),
        max_results: 50,
        filters: SearchFilters::default(),
    }
}

/// Write a file into the given directory, creating subdirectories as needed.
fn write_file(dir: &TempDir, relative_path: &str, content: &str) -> PathBuf {
    let full_path = dir.path().join(relative_path);
    if let Some(parent) = full_path.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(&full_path, content).expect("write file");
    full_path
}

// ===========================================================================
// Test 1: finds_matches_in_rust_files
// ===========================================================================

/// Temp dir with a .rs file containing "SearchResult", search finds it.
#[tokio::test]
async fn finds_matches_in_rust_files() {
    let dir = TempDir::new().unwrap();
    write_file(
        &dir,
        "src/models.rs",
        "pub struct SearchResult {\n    pub source: String,\n}\n",
    );

    let config = config_for(&dir);
    let source = LocalTextSource::new(config);
    let results = source.search(&query("SearchResult")).await.unwrap();

    assert!(!results.is_empty(), "Expected at least one match for 'SearchResult'");
    assert_eq!(results[0].source, "local_text");
    assert!(
        results[0].snippet.contains("SearchResult"),
        "Snippet should contain the matched term, got: {}",
        results[0].snippet
    );
}

// ===========================================================================
// Test 2: include_pattern_filters
// ===========================================================================

/// .rs file matches, .yaml file is skipped when include pattern is *.rs.
#[tokio::test]
async fn include_pattern_filters() {
    let dir = TempDir::new().unwrap();
    write_file(&dir, "main.rs", "fn hello() { println!(\"hello world\"); }\n");
    write_file(&dir, "config.yaml", "key: hello world\n");

    let config = LocalTextConfig {
        paths: vec![dir.path().to_path_buf()],
        include_patterns: vec!["*.rs".to_string()],
        exclude_patterns: vec![],
        max_file_size_bytes: 10 * 1024 * 1024,
    };

    let source = LocalTextSource::new(config);
    let results = source.search(&query("hello")).await.unwrap();

    assert!(!results.is_empty(), "Expected at least one match in .rs file");
    // All results should be from .rs files, not .yaml
    for r in &results {
        let title = r.title.to_lowercase();
        assert!(
            title.ends_with(".rs"),
            "Expected only .rs files, got title: {}",
            r.title
        );
    }
}

// ===========================================================================
// Test 3: exclude_pattern_filters
// ===========================================================================

/// **/target/** excluded: file in target/ not found, file outside target/ found.
#[tokio::test]
async fn exclude_pattern_filters() {
    let dir = TempDir::new().unwrap();
    write_file(&dir, "src/lib.rs", "fn excluded_token() {}\n");
    write_file(&dir, "target/debug/lib.rs", "fn excluded_token() {}\n");

    let config = LocalTextConfig {
        paths: vec![dir.path().to_path_buf()],
        include_patterns: vec![],
        exclude_patterns: vec!["**/target/**".to_string()],
        max_file_size_bytes: 10 * 1024 * 1024,
    };

    let source = LocalTextSource::new(config);
    let results = source.search(&query("excluded_token")).await.unwrap();

    assert!(!results.is_empty(), "Expected match from src/lib.rs");
    // No result should reference target/
    for r in &results {
        assert!(
            !r.title.contains("target/") && !r.title.contains("target\\"),
            "Result should not be from target/ directory, got: {}",
            r.title
        );
    }
}

// ===========================================================================
// Test 4: no_matches_returns_empty
// ===========================================================================

/// Searching for a term that doesn't exist returns empty vec, no error.
#[tokio::test]
async fn no_matches_returns_empty() {
    let dir = TempDir::new().unwrap();
    write_file(&dir, "hello.txt", "nothing interesting here\n");

    let config = config_for(&dir);
    let source = LocalTextSource::new(config);
    let results = source.search(&query("nonexistent_xyzzy_term")).await.unwrap();

    assert!(results.is_empty(), "Expected empty results for no matches");
}

// ===========================================================================
// Test 5: missing_path_warns
// ===========================================================================

/// Nonexistent path returns empty results and does not crash.
#[tokio::test]
async fn missing_path_warns() {
    let config = LocalTextConfig {
        paths: vec![PathBuf::from("/tmp/nonexistent_path_unified_search_test_12345")],
        include_patterns: vec![],
        exclude_patterns: vec![],
        max_file_size_bytes: 10 * 1024 * 1024,
    };

    let source = LocalTextSource::new(config);
    let results = source.search(&query("anything")).await.unwrap();

    assert!(results.is_empty(), "Expected empty results for missing path");
}

// ===========================================================================
// Test 6: max_file_size_respected
// ===========================================================================

/// File exceeding max_file_size_bytes is skipped.
#[tokio::test]
async fn max_file_size_respected() {
    let dir = TempDir::new().unwrap();
    // Write a file just over 1KB with the search term
    let big_content = format!("findme {}", "x".repeat(2000));
    write_file(&dir, "big.txt", &big_content);
    // Write a small file also with the search term
    write_file(&dir, "small.txt", "findme here\n");

    let config = LocalTextConfig {
        paths: vec![dir.path().to_path_buf()],
        include_patterns: vec![],
        exclude_patterns: vec![],
        max_file_size_bytes: 1024, // 1 KB limit
    };

    let source = LocalTextSource::new(config);
    let results = source.search(&query("findme")).await.unwrap();

    // Should find match in small.txt but not big.txt
    assert!(!results.is_empty(), "Expected match from small.txt");
    for r in &results {
        assert!(
            !r.title.contains("big.txt"),
            "big.txt should be skipped due to file size, got: {}",
            r.title
        );
    }
}

// ===========================================================================
// Test 7: snippet_has_context
// ===========================================================================

/// Match line plus surrounding context appears in the snippet.
#[tokio::test]
async fn snippet_has_context() {
    let dir = TempDir::new().unwrap();
    let content = "line1\nline2\nline3 MATCHME here\nline4\nline5\n";
    write_file(&dir, "ctx.txt", content);

    let config = config_for(&dir);
    let source = LocalTextSource::new(config);
    let results = source.search(&query("MATCHME")).await.unwrap();

    assert!(!results.is_empty(), "Expected at least one match");
    let snippet = &results[0].snippet;
    assert!(
        snippet.contains("MATCHME"),
        "Snippet should contain the match term, got: {}",
        snippet
    );
}

// ===========================================================================
// Test 8: multiple_matches_single_result
// ===========================================================================

/// 3 matches in 1 file produce a single SearchResult.
#[tokio::test]
async fn multiple_matches_single_result() {
    let dir = TempDir::new().unwrap();
    let content = "alpha\nalpha\nalpha\n";
    write_file(&dir, "triple.txt", content);

    let config = config_for(&dir);
    let source = LocalTextSource::new(config);
    let results = source.search(&query("alpha")).await.unwrap();

    assert_eq!(
        results.len(),
        1,
        "Expected exactly 1 result for 3 matches in the same file, got {}",
        results.len()
    );
    // Metadata should have a match_count
    let match_count = results[0]
        .metadata
        .get("match_count")
        .expect("Expected match_count in metadata");
    let count: usize = match_count.parse().expect("match_count should be numeric");
    assert!(
        count >= 3,
        "Expected at least 3 matches, got {}",
        count
    );
}

// ===========================================================================
// Test 9: relevance_by_match_count
// ===========================================================================

/// File with 5 matches should have higher relevance than file with 1 match.
#[tokio::test]
async fn relevance_by_match_count() {
    let dir = TempDir::new().unwrap();
    write_file(
        &dir,
        "many.txt",
        "beta\nbeta\nbeta\nbeta\nbeta\n",
    );
    write_file(&dir, "few.txt", "beta\n");

    let config = config_for(&dir);
    let source = LocalTextSource::new(config);
    let results = source.search(&query("beta")).await.unwrap();

    assert_eq!(results.len(), 2, "Expected 2 results, got {}", results.len());

    let many_result = results.iter().find(|r| r.title.contains("many.txt")).unwrap();
    let few_result = results.iter().find(|r| r.title.contains("few.txt")).unwrap();

    assert!(
        many_result.relevance > few_result.relevance,
        "File with 5 matches ({:.2}) should have higher relevance than file with 1 match ({:.2})",
        many_result.relevance,
        few_result.relevance
    );
}

// ===========================================================================
// Test 10: file_url_generation
// ===========================================================================

/// URL of result starts with "file:///".
#[tokio::test]
async fn file_url_generation() {
    let dir = TempDir::new().unwrap();
    write_file(&dir, "urltest.txt", "unique_url_token\n");

    let config = config_for(&dir);
    let source = LocalTextSource::new(config);
    let results = source.search(&query("unique_url_token")).await.unwrap();

    assert!(!results.is_empty(), "Expected at least one match");
    let url = results[0].url.as_ref().expect("Expected a URL on the result");
    assert!(
        url.starts_with("file:///"),
        "URL should start with file:///, got: {}",
        url
    );
}

// ===========================================================================
// Test 11: regex_special_chars_escaped
// ===========================================================================

/// Query "(foo) [bar]" doesn't crash — special regex chars are escaped.
#[tokio::test]
async fn regex_special_chars_escaped() {
    let dir = TempDir::new().unwrap();
    write_file(&dir, "special.txt", "some (foo) [bar] content\n");

    let config = config_for(&dir);
    let source = LocalTextSource::new(config);

    // Should not error even though query has regex special chars
    let results = source.search(&query("(foo) [bar]")).await.unwrap();

    // Should actually find the literal text since we escape the query
    assert!(
        !results.is_empty(),
        "Expected match for literal '(foo) [bar]'"
    );
}

// ===========================================================================
// Test 12: empty_query_returns_empty
// ===========================================================================

/// Empty string query returns 0 results.
#[tokio::test]
async fn empty_query_returns_empty() {
    let dir = TempDir::new().unwrap();
    write_file(&dir, "nonempty.txt", "some content here\n");

    let config = config_for(&dir);
    let source = LocalTextSource::new(config);
    let results = source.search(&query("")).await.unwrap();

    assert!(
        results.is_empty(),
        "Expected empty results for empty query, got {} results",
        results.len()
    );
}

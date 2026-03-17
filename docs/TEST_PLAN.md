# Test Plan — Unified Search MCP Server

## Testing Strategy

### Frameworks & Tools
- **Unit tests**: `#[cfg(test)]` modules + `tests/` directory
- **HTTP mocking**: `wiremock` crate — mock Slack/Confluence/JIRA APIs
- **Filesystem fixtures**: `tempfile` crate + `fixtures/` directory
- **Assertions**: `pretty_assertions` for readable diffs, `assert_matches` for enum matching

### Test Categories

| Category | Where | What |
|----------|-------|------|
| Unit | `#[cfg(test)]` in source files | Pure logic: ranking, dedup, config parsing, model conversions |
| Adapter | `tests/test_{source}.rs` | Each source adapter against wiremock (or temp files for local) |
| Integration | `tests/test_integration.rs` | Full orchestrator pipeline with all sources mocked |

### No Real API Calls
All external API tests use `wiremock` mock servers. No network calls in CI or local test runs.
Real API validation is manual (documented in README under "Manual Testing").

---

## Module Test Specifications

### models.rs — `tests/test_models.rs`

```
TEST: search_result_serialization_roundtrip
  Given: a SearchResult with all fields populated
  When: serialized to JSON then deserialized back
  Then: result equals original

TEST: search_result_ordering_by_relevance
  Given: results with relevance [0.5, 0.9, 0.1, 0.7]
  When: sorted
  Then: order is [0.9, 0.7, 0.5, 0.1]

TEST: search_result_ordering_tiebreak_by_timestamp
  Given: results with same relevance but different timestamps
  When: sorted
  Then: more recent first

TEST: search_result_ordering_none_timestamp_last
  Given: results where some have None timestamp
  When: sorted (same relevance)
  Then: None timestamps sort after Some timestamps

TEST: search_query_defaults
  Given: SearchQuery::default()
  Then: max_results = 20, filters.sources = None, filters.after = None, filters.before = None

TEST: search_filters_partial
  Given: SearchFilters with only `sources` set
  When: serialized
  Then: after and before are null in JSON

TEST: health_status_display
  Given: each HealthStatus variant
  Then: Healthy → "healthy", Degraded → "degraded", Unavailable → "unavailable"

TEST: unified_response_with_warnings
  Given: UnifiedSearchResponse with 2 results and 1 warning
  When: serialized
  Then: JSON contains results array (length 2) and warnings array (length 1)

TEST: search_result_edge_relevance_zero
  Given: SearchResult with relevance 0.0
  Then: serializes and deserializes correctly, sorts last

TEST: search_result_edge_relevance_one
  Given: SearchResult with relevance 1.0
  Then: serializes and deserializes correctly, sorts first

TEST: search_result_empty_metadata
  Given: SearchResult with empty HashMap metadata
  Then: serializes as empty object {}
```

### core.rs — `tests/test_core.rs`

```
// Mock sources (defined in test file):
//
// struct MockSource {
//     name: String,
//     results: Vec<SearchResult>,
//     delay: Option<Duration>,
//     should_error: bool,
//     error_message: String,
// }
//
// struct PanicSource — panics on search (should not crash orchestrator)

TEST: single_source_happy_path
  Given: orchestrator with one MockSource returning 3 results
  When: search("test query")
  Then: response contains 3 results, 0 warnings, total_sources_queried = 1

TEST: multiple_sources_merged
  Given: orchestrator with 3 MockSources returning [2, 3, 1] results
  When: search("query")
  Then: response contains 6 results sorted by relevance

TEST: source_timeout_returns_partial
  Given: orchestrator with [fast MockSource, SlowSource (30s delay)]
  Config: timeout = 2s
  When: search("query")
  Then: response has fast source's results + warning about slow source timeout
  And: total time < 3s

TEST: source_error_returns_partial
  Given: orchestrator with [MockSource(ok), ErrorSource("connection refused")]
  When: search("query")
  Then: response has MockSource results + warning "connection refused"

TEST: all_sources_fail
  Given: orchestrator with [ErrorSource, ErrorSource]
  When: search("query")
  Then: response has 0 results, 2 warnings

TEST: source_weights_affect_ranking
  Given: two sources, source_a (weight=2.0) returns result(relevance=0.5)
         source_b (weight=1.0) returns result(relevance=0.8)
  When: search
  Then: source_a result (weighted 1.0) ranks above source_b result (weighted 0.8)

TEST: dedup_by_url
  Given: two sources both return result with url="https://example.com/page"
         source_a's version has relevance 0.9, source_b's has 0.5
  When: search
  Then: only one result with that URL (the 0.9 one)

TEST: dedup_by_snippet_hash
  Given: two sources return results with different URLs but identical snippets
  When: search
  Then: only one result kept (higher relevance)

TEST: max_results_truncation
  Given: two sources each returning 15 results, max_results=20
  When: search
  Then: response has exactly 20 results (top 20 by score)

TEST: source_filter_in_query
  Given: orchestrator with [slack_source, confluence_source, jira_source]
  Query: sources = ["slack"]
  When: search
  Then: only slack_source is queried, others not called

TEST: health_check_all
  Given: orchestrator with [healthy_source, unhealthy_source]
  When: health_check_all
  Then: returns [Healthy, Unavailable] statuses

TEST: empty_sources_list
  Given: orchestrator with no sources
  When: search("anything")
  Then: response has 0 results, 0 warnings, total_sources_queried = 0

TEST: panic_source_doesnt_crash
  Given: orchestrator with [MockSource(ok), PanicSource]
  When: search("query")
  Then: response has MockSource results + warning about panic source
```

### local_text.rs — `tests/test_local_text.rs`

```
// Uses fixtures/local/sample_codebase/ and fixtures/local/sample_docs/
// Creates temp directories with known content for deterministic tests

TEST: finds_matches_in_rust_files
  Given: temp dir with main.rs containing "SearchResult"
  Config: paths=[temp_dir], include=["**/*.rs"]
  When: search("SearchResult")
  Then: 1 result, title contains "main.rs", snippet contains "SearchResult"

TEST: include_pattern_filters
  Given: temp dir with main.rs and config.yaml both containing "tokio"
  Config: include=["**/*.rs"]
  When: search("tokio")
  Then: only main.rs in results

TEST: exclude_pattern_filters
  Given: temp dir with src/lib.rs and target/debug/build.rs both containing "fn main"
  Config: exclude=["**/target/**"]
  When: search("fn main")
  Then: only src/lib.rs in results

TEST: no_matches_returns_empty
  Given: temp dir with files
  When: search("xyznonexistent")
  Then: 0 results, no error

TEST: missing_path_warns
  Given: config with paths=["/nonexistent/path"]
  When: search("anything")
  Then: 0 results, warning about missing path

TEST: max_file_size_respected
  Given: temp dir with small.rs (100 bytes) and huge.bin (2MB)
  Config: max_file_size = 1MB
  When: search("content")
  Then: huge.bin not searched

TEST: snippet_has_context
  Given: file with match on line 10
  When: search
  Then: snippet includes surrounding lines (not just the match line)

TEST: multiple_matches_single_result
  Given: file with "query" on lines 5, 15, 25
  When: search("query")
  Then: 1 result for that file (not 3)

TEST: relevance_by_match_count
  Given: file_a with 5 matches, file_b with 1 match
  When: search
  Then: file_a.relevance > file_b.relevance

TEST: file_url_generation
  Given: match in /Users/x/projects/repo/src/main.rs
  Then: url = "file:///Users/x/projects/repo/src/main.rs"

TEST: regex_special_chars_escaped
  Given: search query "(foo) [bar]"
  Then: no regex panic, treats as literal search

TEST: empty_query_returns_empty
  When: search("")
  Then: 0 results
```

### confluence.rs — `tests/test_confluence.rs`

```
// All tests use wiremock to mock Confluence REST API

TEST: successful_search_maps_results
  Given: wiremock returns fixtures/confluence/search_success.json
  When: search("broadcast threshold")
  Then: 3 results with correct titles, snippets (HTML stripped), URLs

TEST: html_stripped_from_excerpt
  Given: wiremock returns result with excerpt "<b>bold</b> text <em>italic</em>"
  When: search
  Then: snippet = "bold text italic"

TEST: space_filter_in_cql
  Given: config spaces = ["DEV", "OPS"]
  When: search("query")
  Then: CQL sent to API includes `space IN ("DEV","OPS")`

TEST: empty_results
  Given: wiremock returns fixtures/confluence/search_empty.json
  When: search("nonexistent")
  Then: 0 results, no error

TEST: auth_failure_401
  Given: wiremock returns 401
  When: search("query")
  Then: error message contains "401" and mentions checking email/token

TEST: forbidden_403
  Given: wiremock returns 403
  When: search("query")
  Then: error message about insufficient permissions

TEST: rate_limited_429
  Given: wiremock returns 429 with Retry-After: 30
  When: search("query")
  Then: error mentions rate limit and 30s wait

TEST: server_error_500
  Given: wiremock returns 500
  When: search("query")
  Then: error surfaced (not swallowed)

TEST: network_timeout
  Given: wiremock delays 30s
  Source config: timeout shorter
  When: search("query")
  Then: timeout error

TEST: malformed_json
  Given: wiremock returns invalid JSON
  When: search("query")
  Then: parse error (not panic)

TEST: health_check_success
  Given: wiremock returns 200 for /wiki/rest/api/space?limit=1
  When: health_check()
  Then: Healthy with latency

TEST: relevance_from_api_order
  Given: API returns results in order [A, B, C]
  Then: A.relevance > B.relevance > C.relevance (position-based)

TEST: query_with_quotes_escaped
  Given: query = 'broadcast "threshold"'
  When: search
  Then: CQL sent has escaped quotes: siteSearch ~ "broadcast \"threshold\""

TEST: query_with_cql_operators_literal
  Given: query = "AND OR NOT"
  When: search
  Then: treated as literal text, not CQL operators

TEST: time_filter_after
  Given: query with after = 2026-01-01
  When: search
  Then: CQL includes 'lastmodified >= "2026-01-01"'

TEST: time_filter_before
  Given: query with before = 2026-03-01
  When: search
  Then: CQL includes 'lastmodified <= "2026-03-01"'
```

### jira.rs — `tests/test_jira.rs`

```
// All tests use wiremock to mock JIRA REST API

TEST: successful_search_maps_results
  Given: wiremock returns fixtures/jira/search_success.json
  When: search("broadcast threshold")
  Then: 3 results, title=summary, URL=browse link, metadata has project+status

TEST: project_filter_in_jql
  Given: config projects = ["FIN", "PLAT"]
  When: search("query")
  Then: JQL includes `project IN ("FIN","PLAT")`

TEST: description_truncated
  Given: issue with 1000-char description
  When: mapped to SearchResult
  Then: snippet is first 300 chars + "..."

TEST: empty_results
  Given: wiremock returns 0 issues
  When: search
  Then: 0 results

TEST: auth_failure_401
  Given: wiremock returns 401
  Then: error with auth hint

TEST: forbidden_403
  Given: wiremock returns 403
  Then: permission error

TEST: rate_limited_429
  Given: wiremock returns 429
  Then: rate limit error

TEST: server_error_500
  Given: wiremock returns 500
  Then: server error surfaced

TEST: network_timeout
  Given: wiremock delays
  Then: timeout error

TEST: malformed_json
  Given: invalid JSON
  Then: parse error

TEST: health_check
  Given: wiremock returns 200 for /rest/api/3/myself
  Then: Healthy

TEST: metadata_includes_fields
  Given: issue with project=FIN, status=In Progress, assignee=John
  Then: metadata has all three

TEST: browse_url_construction
  Given: base_url="https://org.atlassian.net", key="FIN-123"
  Then: url = "https://org.atlassian.net/browse/FIN-123"

TEST: query_with_quotes_escaped
  Given: query = 'broadcast "threshold"'
  When: search
  Then: JQL sent has escaped quotes

TEST: query_with_jql_operators_literal
  Given: query = "AND OR NOT"
  When: search
  Then: treated as literal text

TEST: time_filter_after
  Given: query with after = 2026-01-01
  When: search
  Then: JQL includes 'updated >= "2026-01-01"'

TEST: time_filter_before
  Given: query with before = 2026-03-01
  When: search
  Then: JQL includes 'updated <= "2026-03-01"'
```

### slack.rs — `tests/test_slack.rs`

```
// All tests use wiremock to mock Slack Web API

TEST: successful_search_maps_results
  Given: wiremock returns fixtures/slack/search_messages_success.json
  When: search("broadcast threshold")
  Then: 3 results with text snippets, permalinks, channel names

TEST: channel_name_in_metadata
  Given: result in #engineering channel
  Then: metadata["channel"] = "engineering"

TEST: username_in_metadata
  Given: message from user "ganesh"
  Then: metadata["user"] = "ganesh"

TEST: empty_results
  Given: wiremock returns 0 matches
  Then: 0 results

TEST: ok_false_response
  Given: wiremock returns { "ok": false, "error": "invalid_auth" }
  Then: error message includes "invalid_auth"

TEST: wrong_token_type_hint
  Given: wiremock returns { "ok": false, "error": "not_allowed_token_type" }
  Then: error mentions "search requires user token (xoxp-), not bot token (xoxb-)"

TEST: rate_limited
  Given: wiremock returns 429 with Retry-After
  Then: rate limit error

TEST: network_timeout
  Given: wiremock delays
  Then: timeout error

TEST: malformed_json
  Given: invalid JSON
  Then: parse error

TEST: health_check_auth_test
  Given: wiremock returns { "ok": true } for auth.test
  Then: Healthy

TEST: relevance_from_score
  Given: Slack returns score field on matches
  Then: normalized to 0.0–1.0

TEST: timestamp_from_ts_field
  Given: message with ts="1710700800.123456"
  Then: timestamp parsed to correct DateTime<Utc>
```

### config.rs — `tests/test_config.rs`

```
TEST: valid_full_config_parses
  Given: fixtures/config/valid_full.yaml (all sources enabled)
  When: load config
  Then: all source configs populated correctly

TEST: minimal_config_parses
  Given: fixtures/config/valid_minimal.yaml (only local_text)
  When: load config
  Then: local_text enabled, others disabled/absent

TEST: env_var_interpolation
  Given: YAML with "${TEST_TOKEN}" and env TEST_TOKEN="secret123"
  When: load config
  Then: value = "secret123"

TEST: missing_env_var_errors
  Given: YAML with "${NONEXISTENT_VAR}"
  When: load config
  Then: error names "NONEXISTENT_VAR"

TEST: invalid_yaml_syntax
  Given: fixtures/config/invalid_syntax.yaml
  When: load config
  Then: error with line number

TEST: missing_config_file
  Given: path to nonexistent file
  When: load config
  Then: error mentions config.example.yaml

TEST: disabled_sources_skipped
  Given: config with slack.enabled=false
  When: build sources from config
  Then: no SlackSource in list

TEST: tilde_expansion
  Given: path "~/projects/repo"
  When: parsed
  Then: expanded to "/Users/{user}/projects/repo"

TEST: defaults_applied
  Given: config with server section omitted
  When: load
  Then: max_results=20, timeout_seconds=10, log_level="info"
```

### test_integration.rs

```
TEST: full_pipeline_all_sources_mocked
  Given: config with all sources → wiremock servers
  When: unified_search("query")
  Then: results from all sources, merged, ranked, deduplicated

TEST: mixed_success_failure
  Given: slack mock returns 401, others succeed
  When: unified_search("query")
  Then: confluence+jira+local results present, warning about slack

TEST: search_source_single
  Given: all sources configured
  When: search_source("confluence", "query")
  Then: only confluence results

TEST: list_sources_health
  Given: all sources configured, jira mock down
  When: list_sources()
  Then: [slack=Healthy, confluence=Healthy, jira=Unavailable, local_text=Healthy]

TEST: source_filter_respected
  Given: query with sources=["slack", "jira"]
  When: unified_search
  Then: only slack and jira queried

TEST: time_filters_passed_through
  Given: query with after=2026-01-01
  When: sources receive the query
  Then: each source's search method receives the filter

TEST: max_results_global
  Given: each source returns 15, global max=20
  When: unified_search
  Then: 20 results total

TEST: all_sources_disabled
  Given: config with everything disabled
  When: unified_search
  Then: 0 results, message about no sources enabled

TEST: unified_search_returns_markdown_table
  Given: 3 results from different sources
  When: unified_search formats output
  Then: output contains "| # | Source | Title |" header row
  And: 3 data rows with correct source/title/snippet/URL
  And: footer with warnings count and query time

TEST: response_truncation_saves_file
  Given: local_text returns 60 results
  When: unified_search
  Then: response contains top 20 results
  And: note about full results saved to ~/.unified-search/last-search-results.json
  And: file exists with all 60 results

TEST: search_source_returns_richer_detail
  Given: search_source("confluence", "query")
  When: results formatted
  Then: each result has full snippet (not truncated for table) and all metadata fields
```

### server.rs — `tests/test_server.rs`

```
// MCP tool registration and JSON-RPC dispatch tests

TEST: tools_list_returns_all_four
  Given: server initialized
  When: tools/list request
  Then: response contains unified_search, search_source, index_local, list_sources

TEST: unified_search_tool_dispatch
  Given: server with mocked orchestrator
  When: tools/call unified_search { query: "test" }
  Then: orchestrator.search called, results returned as formatted content

TEST: search_source_tool_dispatch
  Given: server with mocked orchestrator
  When: tools/call search_source { source: "slack", query: "test" }
  Then: orchestrator routes to slack source only

TEST: list_sources_tool_dispatch
  Given: server with mocked orchestrator
  When: tools/call list_sources {}
  Then: health_check_all called, statuses returned

TEST: unknown_tool_name_returns_error
  Given: server initialized
  When: tools/call nonexistent_tool {}
  Then: error response "tool not found"

TEST: malformed_json_rpc_request
  Given: server receives invalid JSON
  Then: proper JSON-RPC error response, server does not crash
```

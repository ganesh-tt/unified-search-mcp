use tempfile::TempDir;
use unified_search_mcp::metrics::{MetricsLogger, MetricsEntry};

#[tokio::test]
async fn logs_entry_to_jsonl() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("metrics.jsonl");

    let logger = MetricsLogger::new(path.clone());

    let entry = MetricsEntry::Search {
        tool: "unified_search".to_string(),
        query: "broadcast threshold".to_string(),
        sources_queried: vec!["slack".to_string(), "jira".to_string()],
        total_results: 10,
        deduped_results: 8,
        total_ms: 450,
    };

    logger.log(entry).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let content = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 1);

    let parsed: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(parsed["tool"], "unified_search");
    assert_eq!(parsed["query"], "broadcast threshold");
    assert_eq!(parsed["total_results"], 10);
    assert!(parsed["ts"].is_string());
}

#[tokio::test]
async fn logs_multiple_entries() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("metrics.jsonl");

    let logger = MetricsLogger::new(path.clone());

    for i in 0..5 {
        let entry = MetricsEntry::Search {
            tool: "unified_search".to_string(),
            query: format!("query {}", i),
            sources_queried: vec!["slack".to_string()],
            total_results: i,
            deduped_results: i,
            total_ms: 100 + i as u64,
        };
        logger.log(entry).await;
    }

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let content = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 5);
}

#[test]
fn detail_entry_serializes() {
    let entry = MetricsEntry::Detail {
        tool: "get_detail".to_string(),
        identifier: "FIN-1234".to_string(),
        detected_source: "jira".to_string(),
        explicit_source: None,
        latency_ms: 350,
        comments_returned: 15,
        error: None,
    };

    let json = serde_json::to_value(&entry).unwrap();
    assert_eq!(json["tool"], "get_detail");
    assert_eq!(json["identifier"], "FIN-1234");
    assert_eq!(json["latency_ms"], 350);
}

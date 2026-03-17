use tokio;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub title: String,
    pub relevance: f32,
}

#[tokio::main]
async fn main() {
    println!("async main running");
}

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::models::UnifiedSearchResponse;

pub struct ResponseCache {
    entries: HashMap<String, CacheEntry>,
    max_entries: usize,
    ttl: Duration,
}

struct CacheEntry {
    response: UnifiedSearchResponse,
    created_at: Instant,
    last_accessed: Instant,
}

impl ResponseCache {
    pub fn new(max_entries: usize, ttl: Duration) -> Self {
        Self { entries: HashMap::new(), max_entries, ttl }
    }

    fn make_key(query: &str, sources: &[&str]) -> String {
        let normalized = query.trim().to_lowercase();
        let mut sorted: Vec<&str> = sources.to_vec();
        sorted.sort();
        format!("{}|{}", normalized, sorted.join(","))
    }

    pub fn get(&mut self, query: &str, sources: &[&str]) -> Option<UnifiedSearchResponse> {
        let key = Self::make_key(query, sources);
        let expired = self.entries.get(&key).map_or(false, |e| {
            self.ttl.is_zero() || e.created_at.elapsed() > self.ttl
        });
        if expired {
            self.entries.remove(&key);
            return None;
        }
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.last_accessed = Instant::now();
            let mut response = entry.response.clone();
            response.cache_hit = true;
            Some(response)
        } else {
            None
        }
    }

    pub fn put(&mut self, query: &str, sources: &[&str], response: UnifiedSearchResponse) {
        if self.ttl.is_zero() {
            return;
        }
        let key = Self::make_key(query, sources);
        if self.entries.len() >= self.max_entries && !self.entries.contains_key(&key) {
            self.evict_oldest();
        }
        self.entries.insert(key, CacheEntry {
            response,
            created_at: Instant::now(),
            last_accessed: Instant::now(),
        });
    }

    fn evict_oldest(&mut self) {
        if let Some(key) = self.entries.iter()
            .min_by_key(|(_, e)| e.last_accessed)
            .map(|(k, _)| k.clone())
        {
            self.entries.remove(&key);
        }
    }
}

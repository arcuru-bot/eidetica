use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub author: String,
    pub content: String,
    pub timestamp: DateTime<Utc>,
}

impl ChatMessage {
    pub fn new(author: String, content: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            author,
            content,
            timestamp: Utc::now(),
        }
    }

    /// Returns true if this is a system/action message (from /me or similar)
    pub fn is_action(&self) -> bool {
        self.author == "*"
    }
}

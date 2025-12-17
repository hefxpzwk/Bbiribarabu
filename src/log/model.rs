use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LogItem {
    pub id: String,
    pub created_at: DateTime<Local>,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct BranchLogFile {
    pub branch: String,
    pub items: Vec<LogItem>,
}

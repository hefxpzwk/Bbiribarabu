use std::path::PathBuf;

use crate::git::branch::current_branch;
use crate::git::repo::repo_root;
use crate::log::store::LogStore;

#[derive(Debug)]
pub struct AppState {
    pub repo_root: PathBuf,
    pub current_branch: String,
    pub log_store: LogStore,
}

impl AppState {
    pub fn init() -> Result<Self, String> {
        let repo_root = repo_root()?;
        let branch = current_branch()?;
        let log_store = LogStore::new(&repo_root)?;

        Ok(Self {
            repo_root,
            current_branch: branch,
            log_store,
        })
    }
}

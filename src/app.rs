use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::git::branch::current_branch;
use crate::git::repo::repo_root;
use crate::log::store::LogStore;

#[derive(Debug)]
pub struct AppState {
    pub repo_root: PathBuf,
    pub current_branch: String,
    pub log_store: LogStore,

    last_branch_check: Instant,
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
            last_branch_check: Instant::now(),
        })
    }

    /// 일정 주기마다 브랜치 갱신
    pub fn refresh_branch_if_needed(&mut self) {
        // 500ms에 한 번만 체크
        if self.last_branch_check.elapsed() < Duration::from_millis(500) {
            return;
        }
        self.last_branch_check = Instant::now();

        if let Ok(branch) = current_branch() {
            if branch != self.current_branch {
                self.current_branch = branch;
            }
        }
    }
}

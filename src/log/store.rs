use std::fs;
use std::path::{Path, PathBuf};

use crate::log::model::{BranchLogFile, LogItem};
use chrono::Local;

#[derive(Debug)]
pub struct LogStore {
    base_dir: PathBuf, // repo_root/.bbiribarabu/logs
}

impl LogStore {
    pub fn new(repo_root: &Path) -> Result<Self, String> {
        let base_dir = repo_root.join(".bbiribarabu").join("logs");
        fs::create_dir_all(&base_dir).map_err(|e| format!("로그 디렉터리 생성 실패: {}", e))?;

        Ok(Self { base_dir })
    }

    fn branch_file_path(&self, branch: &str) -> PathBuf {
        // 브랜치명에 슬래시가 들어가면 파일 경로 깨질 수 있어서 치환
        let safe = branch.replace('/', "__");
        self.base_dir.join(format!("{}.json", safe))
    }

    pub fn load(&self, branch: &str) -> Result<BranchLogFile, String> {
        let path = self.branch_file_path(branch);

        if !path.exists() {
            return Ok(BranchLogFile {
                branch: branch.to_string(),
                items: vec![],
            });
        }

        let data = fs::read_to_string(&path)
            .map_err(|e| format!("로그 파일 읽기 실패: {} ({})", e, path.display()))?;

        serde_json::from_str(&data).map_err(|e| format!("로그 JSON 파싱 실패: {}", e))
    }

    pub fn append_text(&self, branch: &str, text: &str) -> Result<LogItem, String> {
        let mut file = self.load(branch)?;
        let item = LogItem {
            id: format!("{}", Local::now().timestamp_millis()),
            created_at: Local::now(),
            text: text.to_string(),
        };
        file.items.push(item.clone());

        let path = self.branch_file_path(branch);
        let json = serde_json::to_string_pretty(&file)
            .map_err(|e| format!("로그 JSON 직렬화 실패: {}", e))?;

        fs::write(&path, json)
            .map_err(|e| format!("로그 파일 쓰기 실패: {} ({})", e, path.display()))?;

        Ok(item)
    }

    pub fn list(&self, branch: &str) -> Result<Vec<LogItem>, String> {
        Ok(self.load(branch)?.items)
    }
}

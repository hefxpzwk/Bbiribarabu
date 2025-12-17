use crate::git::branch::current_branch;

#[derive(Debug)]
pub struct AppState {
    pub current_branch: String,
}

impl AppState {
    /// 앱 시작 시 단 한 번 호출되는 초기화
    pub fn init() -> Result<Self, String> {
        let branch = current_branch()?;

        Ok(Self {
            current_branch: branch,
        })
    }
}

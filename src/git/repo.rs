use std::path::PathBuf;
use std::process::Command;

pub fn repo_root() -> Result<PathBuf, String> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| format!("git 실행 실패: {}", e))?;

    if !output.status.success() {
        return Err("git repo가 아님 (rev-parse 실패)".to_string());
    }

    let s = String::from_utf8(output.stdout)
        .map_err(|_| "repo root 문자열 변환 실패".to_string())?
        .trim()
        .to_string();

    if s.is_empty() {
        Err("repo root 경로를 찾지 못함".to_string())
    } else {
        Ok(PathBuf::from(s))
    }
}

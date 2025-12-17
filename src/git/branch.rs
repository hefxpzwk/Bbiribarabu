use std::process::Command;

pub fn current_branch() -> Result<String, String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .output()
        .map_err(|e| format!("git 실행 실패: {}", e))?;

    if !output.status.success() {
        return Err("git 명령이 정상 종료되지 않음".to_string());
    }

    let branch = String::from_utf8(output.stdout)
        .map_err(|_| "브랜치 이름을 문자열로 변환 실패".to_string())?
        .trim()
        .to_string();

    if branch.is_empty() {
        Err("현재 브랜치를 찾을 수 없음".to_string())
    } else {
        Ok(branch)
    }
}

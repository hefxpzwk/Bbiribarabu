mod app;
mod git;
mod ui;
mod log;

use app::AppState;

fn main() {
    println!("bbiribarabu starting...");

    let app_state = match AppState::init() {
        Ok(state) => state,
        Err(err) => {
            eprintln!("초기화 실패: {}", err);
            return;
        }
    };

    println!("repo: {}", app_state.repo_root.display());
    println!("branch: {}", app_state.current_branch);

    // ✅ Stage 2 테스트: 로그 1개 추가
    let added = app_state
        .log_store
        .append_text(&app_state.current_branch, "첫 로그 테스트: 앱 실행됨")
        .unwrap_or_else(|e| {
            eprintln!("로그 추가 실패: {}", e);
            std::process::exit(1);
        });

    println!("로그 추가됨: {} / {}", added.id, added.created_at);

    // ✅ Stage 2 테스트: 로그 목록 출력
    let items = app_state
        .log_store
        .list(&app_state.current_branch)
        .unwrap_or_else(|e| {
            eprintln!("로그 조회 실패: {}", e);
            std::process::exit(1);
        });

    println!("현재 브랜치 로그 {}개", items.len());
}

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

    println!("현재 브랜치: {}", app_state.current_branch);

    // 🔒 Stage 1에서는 여기서 끝
}

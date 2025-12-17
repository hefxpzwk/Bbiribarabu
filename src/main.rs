mod app;
mod git;
mod ui;
mod log;
mod cli;

use app::AppState;
use clap::Parser;
use cli::{Cli, Commands};

fn main() {
    let cli = Cli::parse();

    let app_state = match AppState::init() {
        Ok(state) => state,
        Err(err) => {
            eprintln!("초기화 실패: {}", err);
            return;
        }
    };

    match cli.command {
        Commands::Add { text } => {
            let item = app_state
                .log_store
                .append_text(&app_state.current_branch, &text)
                .unwrap_or_else(|e| {
                    eprintln!("로그 추가 실패: {}", e);
                    std::process::exit(1);
                });

            println!(
                "✅ 로그 추가됨 [{}] {}",
                item.created_at.format("%Y-%m-%d %H:%M:%S"),
                item.text
            );
        }

        Commands::List => {
            let items = app_state
                .log_store
                .list(&app_state.current_branch)
                .unwrap_or_else(|e| {
                    eprintln!("로그 조회 실패: {}", e);
                    std::process::exit(1);
                });

            if items.is_empty() {
                println!("📭 현재 브랜치에 로그가 없습니다");
                return;
            }

            println!("📌 브랜치: {}", app_state.current_branch);
            println!("--------------------------------");

            for item in items {
                println!(
                    "[{}] {}",
                    item.created_at.format("%Y-%m-%d %H:%M:%S"),
                    item.text
                );
            }
        }
    }
}

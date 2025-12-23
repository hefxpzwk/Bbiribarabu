mod app;
mod cli;
mod git;
mod log;
mod ui;
mod voice;

use app::AppState;
use clap::Parser;
use cli::{Cli, Commands};
use voice::silence_whisper_logs;

fn main() {
    silence_whisper_logs();
    let cli = Cli::parse();

    let mut app_state = match AppState::init() {
        Ok(state) => state,
        Err(err) => {
            eprintln!("초기화 실패: {}", err);
            return;
        }
    };

    match cli.command {
        Some(Commands::Add { text }) => {
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

        Some(Commands::List) => {
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

            for item in items {
                println!(
                    "[{}] {}",
                    item.created_at.format("%Y-%m-%d %H:%M:%S"),
                    item.text
                );
            }
        }

        Some(Commands::Voice { seconds }) => {
            let model_path = std::env::var("WHISPER_MODEL")
                .unwrap_or_else(|_| "models/ggml-tiny.bin".to_string());

            let mut config = voice::VadConfig::default();
            config.max_record_ms = (seconds.max(1) as u32) * 1000;
            let text = voice::transcribe_from_mic_vad(&model_path, config)
                .unwrap_or_else(|e| {
                    eprintln!("보이스 인식 실패: {}", e);
                    std::process::exit(1);
                });

            let trimmed = text.trim();
            if trimmed.is_empty() {
                println!("인식된 텍스트가 없습니다");
                return;
            }

            let item = app_state
                .log_store
                .append_text(&app_state.current_branch, trimmed)
                .unwrap_or_else(|e| {
                    eprintln!("로그 추가 실패: {}", e);
                    std::process::exit(1);
                });

            println!(
                "✅ 보이스 로그 추가됨 [{}] {}",
                item.created_at.format("%Y-%m-%d %H:%M:%S"),
                item.text
            );
        }

        None => {
            if let Err(e) = ui::tui::run(&mut app_state) {
                eprintln!("TUI 실행 오류: {}", e);
            }
        }
    }
}

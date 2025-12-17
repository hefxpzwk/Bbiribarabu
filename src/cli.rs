use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "bbiribarabu")]
#[command(about = "브랜치 컨텍스트 로그 도구", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>, // 👈 Option으로 변경
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// 현재 브랜치에 로그 추가
    Add {
        /// 기록할 텍스트
        text: String,
    },

    /// 현재 브랜치 로그 목록 조회
    List,
}

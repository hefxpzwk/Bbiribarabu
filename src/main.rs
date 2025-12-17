mod app;
mod git;
mod ui;
mod log;

use git::branch::current_branch;

fn main() {
    println!("bbiribarabu starting...");

    match current_branch() {
        Ok(branch) => {
            println!("현재 브랜치: {}", branch);
        }
        Err(err) => {
            eprintln!("브랜치 조회 실패: {}", err);
        }
    }
}

#[derive(PartialEq)]
enum InputMode {
    Normal,
    Editing,
}

struct UiState {
    mode: InputMode,
    input: String,
}

impl UiState {
    fn new() -> Self {
        Self {
            mode: InputMode::Normal,
            input: String::new(),
        }
    }
}


use std::io::{self, Stdout};
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    event::{self, Event, KeyCode},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Terminal,
};

use crate::app::AppState;

pub fn run(app: &mut AppState) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut ui_state = UiState::new();

    let res = run_loop(&mut terminal, app, &mut ui_state);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    res
}


fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut AppState,
    ui: &mut UiState,
) -> io::Result<()> {
    loop {
        // 🔄 브랜치 변경 감지
        app.refresh_branch_if_needed();

        if ui.mode == InputMode::Editing {
            // 브랜치 바뀌면 입력 취소
            ui.mode = InputMode::Normal;
            ui.input.clear();
        }

        terminal.draw(|f| {
            let size = f.size();

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                Constraint::Length(3), // header
                Constraint::Min(0),    // body
                Constraint::Length(3), // input
                ])
                .split(size);


            // 헤더
            let header = Paragraph::new(Line::from(vec![
                Span::styled(" repo: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(app.repo_root.display().to_string()),
                Span::raw(" | "),
                Span::styled("branch: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(&app.current_branch),
            ]))
            .block(Block::default().borders(Borders::ALL).title(" bbiribarabu "));
            f.render_widget(header, chunks[0]);

            // 본문 좌/우 분할
            let body = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(60),
                    Constraint::Percentage(40),
                ])
                .split(chunks[1]);

            // 좌측: Git 터미널 영역 (placeholder)
            let left = Paragraph::new("Git Terminal Area\n(곧 실제 터미널 연결)")
                .block(Block::default().borders(Borders::ALL).title(" Terminal "));
            f.render_widget(left, body[0]);

            // 우측: 로그 리스트
            let items = app
                .log_store
                .list(&app.current_branch)
                .unwrap_or_default()
                .into_iter()
                .rev()
                .map(|it| {
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            format!("[{}] ", it.created_at.format("%m-%d %H:%M")),
                            Style::default().add_modifier(Modifier::DIM),
                        ),
                        Span::raw(it.text),
                    ]))
                })
                .collect::<Vec<_>>();

            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title(" Logs "));
            f.render_widget(list, body[1]);

            let input_block = Block::default()
            .borders(Borders::ALL)
            .title(match ui.mode {
                InputMode::Normal => " Press i to add log ",
                InputMode::Editing => " Enter log (Enter=save, Esc=cancel) ",
            });

            let input = Paragraph::new(ui.input.as_str())
                .block(input_block);

            f.render_widget(input, chunks[2]);

            if matches!(ui.mode, InputMode::Editing) {
                f.set_cursor(
                    chunks[2].x + ui.input.len() as u16 + 1,
                    chunks[2].y + 1,
                );
            }

        })?;

        // 키 입력
        if event::poll(std::time::Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                match ui.mode {
                    InputMode::Normal => match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Char('i') => {
                            ui.mode = InputMode::Editing;
                            ui.input.clear();
                        }
                        _ => {}
                    },

                    InputMode::Editing => match key.code {
                        KeyCode::Esc => {
                            ui.mode = InputMode::Normal;
                            ui.input.clear();
                        }
                        KeyCode::Enter => {
                            if !ui.input.trim().is_empty() {
                                let _ = app
                                    .log_store
                                    .append_text(&app.current_branch, &ui.input);
                            }
                            ui.input.clear();
                            ui.mode = InputMode::Normal;
                        }
                        KeyCode::Backspace => {
                            ui.input.pop();
                        }
                        KeyCode::Char(c) => {
                            ui.input.push(c);
                        }
                        _ => {}
                    },
                }
            }
        }

    }
    Ok(())
}

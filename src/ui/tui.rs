use std::{
    io::{self, Stdout},
    path::PathBuf,
};

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::{app::AppState, ui::terminal::ShellRunner};

#[derive(PartialEq)]
enum InputMode {
    Normal,
    EditingLog,
    EditingShell,
}

struct UiState {
    mode: InputMode,
    log_input: String,
    shell_input: String,
    shell: ShellRunner,
}

impl UiState {
    fn new(repo_root: PathBuf) -> Self {
        Self {
            mode: InputMode::Normal,
            log_input: String::new(),
            shell_input: String::new(),
            shell: ShellRunner::new(repo_root),
        }
    }
}

pub fn run(app: &mut AppState) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut ui_state = UiState::new(app.repo_root.clone());

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
        let prev_branch = app.current_branch.clone();
        app.refresh_branch_if_needed();
        let branch_changed = prev_branch != app.current_branch;

        // 브랜치가 바뀌면 입력 상태 취소
        if branch_changed && matches!(ui.mode, InputMode::EditingLog | InputMode::EditingShell) {
            ui.mode = InputMode::Normal;
            ui.log_input.clear();
            ui.shell_input.clear();
        }

        ui.shell.poll_output();

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
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" bbiribarabu "),
            );
            f.render_widget(header, chunks[0]);

            // 본문 좌/우 분할
            let body = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
                .split(chunks[1]);

            // 좌측: 터미널 출력
            let terminal_area = body[0];
            let max_visible = terminal_area.height.saturating_sub(2).max(1) as usize;
            let lines = ui.shell.recent_lines(max_visible.min(50));
            let padding = max_visible.saturating_sub(lines.len());
            let mut display_lines: Vec<Line> = Vec::with_capacity(max_visible);
            display_lines.extend(std::iter::repeat(Line::from("")).take(padding));
            display_lines.extend(lines.into_iter().map(Line::from));

            let left = Paragraph::new(display_lines)
                .block(Block::default().borders(Borders::ALL).title(" Terminal "));
            f.render_widget(left, terminal_area);

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

            let list =
                List::new(items).block(Block::default().borders(Borders::ALL).title(" Logs "));
            f.render_widget(list, body[1]);

            let input_block = Block::default().borders(Borders::ALL).title(match ui.mode {
                InputMode::Normal => " Press i=log, :=shell, q=quit ",
                InputMode::EditingLog => " Enter log (Enter=save, Esc=cancel) ",
                InputMode::EditingShell => " Shell command (Enter=run, Esc=cancel) ",
            });

            let input_text = match ui.mode {
                InputMode::EditingShell => ui.shell_input.as_str(),
                _ => ui.log_input.as_str(),
            };

            let input = Paragraph::new(input_text).block(input_block);

            f.render_widget(input, chunks[2]);

            if matches!(ui.mode, InputMode::EditingLog | InputMode::EditingShell) {
                let cursor_pos = match ui.mode {
                    InputMode::EditingShell => ui.shell_input.len(),
                    _ => ui.log_input.len(),
                };
                f.set_cursor(chunks[2].x + cursor_pos as u16 + 1, chunks[2].y + 1);
            }
        })?;

        // 키 입력
        if event::poll(std::time::Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                match ui.mode {
                    InputMode::Normal => match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Char('i') => {
                            ui.mode = InputMode::EditingLog;
                            ui.log_input.clear();
                        }
                        KeyCode::Char(':') => {
                            ui.mode = InputMode::EditingShell;
                            ui.shell_input.clear();
                        }
                        _ => {}
                    },

                    InputMode::EditingLog => match key.code {
                        KeyCode::Esc => {
                            ui.mode = InputMode::Normal;
                            ui.log_input.clear();
                        }
                        KeyCode::Enter => {
                            if !ui.log_input.trim().is_empty() {
                                let _ = app
                                    .log_store
                                    .append_text(&app.current_branch, &ui.log_input);
                            }
                            ui.log_input.clear();
                            ui.mode = InputMode::Normal;
                        }
                        KeyCode::Backspace => {
                            ui.log_input.pop();
                        }
                        KeyCode::Char(c) => {
                            ui.log_input.push(c);
                        }
                        _ => {}
                    },

                    InputMode::EditingShell => match key.code {
                        KeyCode::Esc => {
                            ui.mode = InputMode::Normal;
                            ui.shell_input.clear();
                        }
                        KeyCode::Enter => {
                            if !ui.shell_input.trim().is_empty() {
                                ui.shell.run_command(ui.shell_input.clone());
                            }
                            ui.shell_input.clear();
                            ui.mode = InputMode::Normal;
                        }
                        KeyCode::Backspace => {
                            ui.shell_input.pop();
                        }
                        KeyCode::Char(c) => {
                            ui.shell_input.push(c);
                        }
                        _ => {}
                    },
                }
            }
        }
    }
    Ok(())
}

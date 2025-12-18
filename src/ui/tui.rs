use std::{
    io::{self, Stdout},
    path::PathBuf,
};

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};

use crate::{
    app::AppState,
    ui::terminal::{OutputKind, TerminalRunner},
};

#[derive(PartialEq, Eq, Copy, Clone)]
enum Focus {
    Terminal,
    LogInput,
}

#[derive(PartialEq, Eq, Copy, Clone)]
enum InputMode {
    Normal,
    EditingLog,
}

struct UiState {
    focus: Focus,
    mode: InputMode,
    log_input: String,
    history: Vec<String>,
    history_index: usize,
    terminal: TerminalRunner,
}

impl UiState {
    fn new(repo_root: PathBuf) -> Self {
        Self {
            focus: Focus::Terminal,
            mode: InputMode::Normal,
            log_input: String::new(),
            history: Vec::new(),
            history_index: 0,
            terminal: TerminalRunner::new(repo_root),
        }
    }

    fn reset_history_pos(&mut self) {
        self.history_index = self.history.len();
    }
}

pub fn run(app: &mut AppState) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut ui_state = UiState::new(app.repo_root.clone());

    let res = run_loop(&mut terminal, app, &mut ui_state);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
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

        // 브랜치가 바뀌면 로그 입력은 취소
        if branch_changed && ui.mode == InputMode::EditingLog {
            ui.mode = InputMode::Normal;
            ui.log_input.clear();
        }

        ui.terminal.poll_output();

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
            let title = match ui.focus {
                Focus::Terminal => " Terminal (focus) ",
                Focus::LogInput => " Terminal ",
            };
            let terminal_block = Block::default().borders(Borders::ALL).title(title);
            let inner = terminal_block.inner(terminal_area);
            let [output_area, prompt_area] = *Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .split(inner)
            else {
                unreachable!()
            };

            let output_height = output_area.height as usize;

            let lines = ui.terminal.visible_lines(output_height);
            let display_lines: Vec<Line> = lines
                .into_iter()
                .map(|l| {
                    let style = match l.kind {
                        OutputKind::Command { .. } => Style::default()
                            .add_modifier(Modifier::BOLD)
                            .fg(Color::Gray),
                        OutputKind::Stderr {
                            exit_success,
                            is_errorish,
                        } => {
                            if !exit_success || is_errorish {
                                Style::default().fg(Color::Red)
                            } else {
                                Style::default()
                                    .fg(Color::DarkGray)
                                    .add_modifier(Modifier::DIM)
                            }
                        }
                        OutputKind::Info => Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::DIM | Modifier::ITALIC),
                        OutputKind::Stdout => Style::default(),
                    };
                    Line::from(Span::styled(l.text, style))
                })
                .collect();

            let prompt_line = {
                let repo_name = app
                    .repo_root
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("repo");
                Line::from(vec![
                    Span::styled(
                        format!("{}({})", repo_name, app.current_branch),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(" $ ", Style::default().add_modifier(Modifier::DIM)),
                    Span::raw(&ui.terminal.input),
                ])
            };

            let output_widget = Paragraph::new(display_lines).wrap(Wrap { trim: false });
            f.render_widget(terminal_block, terminal_area);
            f.render_widget(output_widget, output_area);
            f.render_widget(Paragraph::new(prompt_line), prompt_area);

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

            let input_block =
                Block::default()
                    .borders(Borders::ALL)
                    .title(match (ui.focus, ui.mode) {
                        (Focus::LogInput, InputMode::EditingLog) => {
                            " Enter log (Enter=save, Esc=cancel) "
                        }
                        (Focus::LogInput, InputMode::Normal) => {
                            " Log input (press i to add, Tab=switch, q=quit) "
                        }
                        _ => " Log input (Tab to focus) ",
                    });

            let input_text = ui.log_input.as_str();

            let input = Paragraph::new(input_text).block(input_block);

            f.render_widget(input, chunks[2]);

            if matches!(ui.mode, InputMode::EditingLog) && ui.focus == Focus::LogInput {
                let cursor_pos = ui.log_input.len();
                f.set_cursor(chunks[2].x + cursor_pos as u16 + 1, chunks[2].y + 1);
            }

            if ui.focus == Focus::Terminal {
                let cursor_x = terminal_area.x + 2 + ui.terminal.input.len() as u16;
                let cursor_y = terminal_area.y + terminal_area.height.saturating_sub(2);
                f.set_cursor(cursor_x, cursor_y);
            }
        })?;

        // 키 입력
        if event::poll(std::time::Duration::from_millis(200))? {
            match event::read()? {
                Event::Key(key) => {
                    match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Tab => {
                            ui.focus = match ui.focus {
                                Focus::Terminal => Focus::LogInput,
                                Focus::LogInput => Focus::Terminal,
                            };
                            if ui.focus == Focus::Terminal && ui.mode == InputMode::EditingLog {
                                ui.mode = InputMode::Normal;
                                ui.log_input.clear();
                            }
                        }
                        _ => {}
                    }

                    match ui.focus {
                        Focus::Terminal => match key.code {
                            KeyCode::Enter => {
                                let cmd = ui.terminal.input.trim().to_string();
                                if !cmd.is_empty() {
                                    ui.terminal.run_command(&cmd);
                                    ui.history.push(cmd);
                                    ui.reset_history_pos();
                                }
                                ui.terminal.input.clear();
                            }
                            KeyCode::Backspace => {
                                ui.terminal.input.pop();
                                ui.reset_history_pos();
                            }
                            KeyCode::Char(c) => {
                                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT
                                {
                                    ui.terminal.input.push(c);
                                    ui.reset_history_pos();
                                }
                            }
                            KeyCode::Up => {
                                if !ui.history.is_empty() {
                                    if ui.history_index == ui.history.len() {
                                        ui.history_index = ui.history.len().saturating_sub(1);
                                    } else if ui.history_index > 0 {
                                        ui.history_index -= 1;
                                    }
                                    ui.terminal.input = ui.history[ui.history_index].clone();
                                }
                            }
                            KeyCode::Down => {
                                if ui.history_index + 1 < ui.history.len() {
                                    ui.history_index += 1;
                                    ui.terminal.input = ui.history[ui.history_index].clone();
                                } else {
                                    ui.history_index = ui.history.len();
                                    ui.terminal.input.clear();
                                }
                            }
                            KeyCode::PageUp => ui.terminal.scroll_up(10),
                            KeyCode::PageDown => ui.terminal.scroll_down(10),
                            _ => {}
                        },
                        Focus::LogInput => match ui.mode {
                            InputMode::Normal => match key.code {
                                KeyCode::Char('i') => {
                                    ui.mode = InputMode::EditingLog;
                                    ui.log_input.clear();
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
                        },
                    }
                }
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => ui.terminal.scroll_up(3),
                    MouseEventKind::ScrollDown => ui.terminal.scroll_down(3),
                    _ => {}
                },
                _ => {}
            }
        }
    }
    Ok(())
}

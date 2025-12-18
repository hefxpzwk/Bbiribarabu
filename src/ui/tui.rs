use std::{
    io::{self, Stdout},
    path::PathBuf,
    time::Duration,
};

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};

use crate::{
    app::AppState,
    ui::pty_terminal::{PtyTerminal, encode_key_event},
};

#[derive(Copy, Clone, PartialEq, Eq)]
enum Focus {
    Terminal,
    LogInput,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum InputMode {
    Normal,
    EditingLog,
}

struct UiState {
    focus: Focus,
    mode: InputMode,
    log_input: String,
    pty: PtyTerminal,
    pending_resize: Option<(u16, u16)>,
}

impl UiState {
    fn new(repo_root: PathBuf, rows: u16, cols: u16) -> Result<Self, String> {
        Ok(Self {
            focus: Focus::Terminal,
            mode: InputMode::Normal,
            log_input: String::new(),
            pty: PtyTerminal::spawn(repo_root, rows, cols)?,
            pending_resize: None,
        })
    }
}

pub fn run(app: &mut AppState) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let size = terminal.size()?;
    let term_rows = size.height.saturating_sub(4);
    let term_cols = (size.width as f32 * 0.6) as u16;
    let mut ui_state =
        UiState::new(app.repo_root.clone(), term_rows, term_cols).map_err(to_io_error)?;

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
        let prev_branch = app.current_branch.clone();
        app.refresh_branch_if_needed();
        if prev_branch != app.current_branch && ui.mode == InputMode::EditingLog {
            ui.mode = InputMode::Normal;
            ui.log_input.clear();
        }

        ui.pty.poll_output();

        terminal.draw(|f| {
            let size = f.size();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(0),
                    Constraint::Length(3),
                ])
                .split(size);

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

            let body = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
                .split(chunks[1]);

            // Terminal panel
            let term_area = body[0];
            let title = match ui.focus {
                Focus::Terminal => " Terminal (focus) ",
                Focus::LogInput => " Terminal ",
            };
            let block = Block::default().borders(Borders::ALL).title(title);
            let inner = block.inner(term_area);

            let display_lines = ui.pty.lines(inner.height);
            let paragraph = Paragraph::new(
                display_lines
                    .clone()
                    .into_iter()
                    .map(Line::from)
                    .collect::<Vec<_>>(),
            )
            .wrap(Wrap { trim: false });
            f.render_widget(block, term_area);
            f.render_widget(paragraph, inner);

            if ui.focus == Focus::Terminal {
                if let Some((cx, cy)) = ui.pty.cursor() {
                    if cy < inner.height && cx < inner.width {
                        f.set_cursor(inner.x + cx, inner.y + cy);
                    }
                }
            }

            // Logs
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
            let log_block =
                List::new(items).block(Block::default().borders(Borders::ALL).title(" Logs "));
            f.render_widget(log_block, body[1]);

            // Input bar
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
        })?;

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    match key.code {
                        KeyCode::Char('q') if ui.focus == Focus::Terminal => break,
                        KeyCode::Tab => {
                            ui.focus = match ui.focus {
                                Focus::Terminal => Focus::LogInput,
                                Focus::LogInput => Focus::Terminal,
                            };
                        }
                        _ => {}
                    }

                    match ui.focus {
                        Focus::Terminal => {
                            if let Some(bytes) = encode_key_event(key) {
                                ui.pty.send_bytes(&bytes);
                            }
                        }
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
                    MouseEventKind::ScrollUp => ui.pty.scroll_up(3),
                    MouseEventKind::ScrollDown => ui.pty.scroll_down(3),
                    _ => {}
                },
                Event::Resize(rows, cols) => {
                    ui.pending_resize = Some((rows, cols));
                }
                _ => {}
            }
        }

        if let Some((rows, cols)) = ui.pending_resize.take() {
            let term_rows = rows.saturating_sub(4);
            let term_cols = (cols as f32 * 0.6) as u16;
            ui.pty.resize(term_rows, term_cols);
        }
    }
    Ok(())
}

fn to_io_error(e: String) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e)
}

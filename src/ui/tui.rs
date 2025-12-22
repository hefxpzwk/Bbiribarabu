use std::{
    io::{self, Stdout},
    path::PathBuf,
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, MouseEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};

use crate::{
    app::AppState,
    ui::pty_terminal::{PtyTerminal, encode_key_event},
    voice,
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
    debug_overlay: bool,
    status_message: Option<(String, Instant)>,
    voice_recording: Option<voice::VoiceRecording>,
}

impl UiState {
    fn new(repo_root: PathBuf, rows: u16, cols: u16) -> Result<Self, String> {
        Ok(Self {
            focus: Focus::Terminal,
            mode: InputMode::Normal,
            log_input: String::new(),
            pty: PtyTerminal::spawn(repo_root, rows, cols)?,
            debug_overlay: false,
            status_message: None,
            voice_recording: None,
        })
    }

    fn set_status(&mut self, message: impl Into<String>) {
        self.status_message = Some((message.into(), Instant::now()));
    }
}

pub fn run(app: &mut AppState) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let size = terminal.size()?;
    let layout = compute_layout(size);
    let mut ui_state = UiState::new(
        app.repo_root.clone(),
        layout.term_inner.height,
        layout.term_inner.width,
    )
    .map_err(to_io_error)?;

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
        if let Some((_, at)) = ui.status_message.as_ref() {
            if at.elapsed() > Duration::from_secs(2) {
                ui.status_message = None;
            }
        }

        let layout = compute_layout(terminal.size()?);
        ui.pty
            .ensure_size(layout.term_inner.height, layout.term_inner.width);
        ui.pty.poll_output();

        terminal.draw(|f| {
            let layout = compute_layout(f.size());
            let mut final_cursor_abs: Option<(u16, u16)> = None;

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
            f.render_widget(header, layout.header);

            // Terminal panel
            let term_area = layout.terminal;
            let title = match ui.focus {
                Focus::Terminal => " Terminal (focus) ",
                Focus::LogInput => " Terminal ",
            };
            let block = Block::default().borders(Borders::ALL).title(title);
            let inner = layout.term_inner;

            let display_lines = ui.pty.lines();
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
                if let Some(cursor) = ui.pty.cursor_state() {
                    if inner.width > 0 && inner.height > 0 && cursor.draw {
                        let col = cursor.col;
                        let row = cursor.row;
                        let clamped_col = col.min(inner.width.saturating_sub(1));
                        let clamped_row = row.min(inner.height.saturating_sub(1));
                        let abs_x = inner.x + clamped_col;
                        let abs_y = inner.y + clamped_row;
                        final_cursor_abs = Some((abs_x, abs_y));
                        f.set_cursor(abs_x, abs_y);
                    }
                }
            }

            if ui.debug_overlay {
                let debug = Paragraph::new(debug_lines(&ui, &layout, inner, final_cursor_abs))
                    .block(Block::default().borders(Borders::ALL).title(" debug "));
                let overlay_area = Rect {
                    x: inner.x.saturating_add(1),
                    y: inner.y.saturating_add(1),
                    width: inner.width.min(32),
                    height: inner.height.min(6),
                };
                f.render_widget(debug, overlay_area);
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
            f.render_widget(log_block, layout.logs);

            // Input bar
            let input_block =
                Block::default()
                    .borders(Borders::ALL)
                    .title(match (ui.focus, ui.mode) {
                        (Focus::LogInput, InputMode::EditingLog) => {
                            " Enter log (Enter=save, Esc=cancel) "
                        }
                        (Focus::LogInput, InputMode::Normal) => {
                            " Log input (press i to add, v=voice, Tab=switch, q=quit) "
                        }
                        _ => " Log input (Tab to focus) ",
                    });

            let input_text = if matches!(ui.mode, InputMode::EditingLog) {
                ui.log_input.as_str()
            } else if let Some((ref msg, _)) = ui.status_message {
                msg.as_str()
            } else {
                ""
            };
            let input = Paragraph::new(input_text).block(input_block);
            f.render_widget(input, layout.input);

            if matches!(ui.mode, InputMode::EditingLog) && ui.focus == Focus::LogInput {
                let cursor_pos = ui.log_input.len();
                f.set_cursor(layout.input.x + cursor_pos as u16 + 1, layout.input.y + 1);
            }
        })?;

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.code == KeyCode::Esc
                        && !(ui.focus == Focus::LogInput && ui.mode == InputMode::EditingLog)
                    {
                        ui.focus = match ui.focus {
                            Focus::Terminal => Focus::LogInput,
                            Focus::LogInput => Focus::Terminal,
                        };
                        continue;
                    }

                    match key.code {
                        KeyCode::Char('q') if ui.focus == Focus::Terminal => break,
                        KeyCode::F(2) => {
                            ui.debug_overlay = !ui.debug_overlay;
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
                                KeyCode::Char('v') => {
                                    if ui.voice_recording.is_none() {
                                        match voice::start_recording() {
                                            Ok(rec) => {
                                                ui.voice_recording = Some(rec);
                                                ui.set_status("녹음중... 다시 v로 종료");
                                            }
                                            Err(e) => {
                                                ui.set_status(format!("녹음 시작 실패: {}", e));
                                            }
                                        }
                                    } else if let Some(rec) = ui.voice_recording.take() {
                                        let model_path = std::env::var("WHISPER_MODEL")
                                            .unwrap_or_else(|_| "models/ggml-tiny.bin".to_string());
                                        let (audio, rate, channels) = rec.stop();
                                        match voice::transcribe_audio(
                                            &model_path,
                                            audio,
                                            rate,
                                            channels,
                                        ) {
                                            Ok(t) => {
                                                let trimmed = t.trim();
                                                if trimmed.is_empty() {
                                                    ui.set_status("보이스 인식 결과 없음");
                                                } else if let Err(e) = app
                                                    .log_store
                                                    .append_text(&app.current_branch, trimmed)
                                                {
                                                    ui.set_status(format!(
                                                        "보이스 로그 실패: {}",
                                                        e
                                                    ));
                                                } else {
                                                    ui.set_status("보이스 로그 추가됨");
                                                }
                                            }
                                            Err(e) => {
                                                ui.set_status(format!("보이스 인식 실패: {}", e));
                                            }
                                        }
                                    }
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
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }
    Ok(())
}

struct LayoutInfo {
    header: Rect,
    terminal: Rect,
    logs: Rect,
    input: Rect,
    term_inner: Rect,
}

fn compute_layout(area: Rect) -> LayoutInfo {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(area);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(chunks[1]);

    let term_area = body[0];
    let term_inner = Rect {
        x: term_area.x.saturating_add(1),
        y: term_area.y.saturating_add(1),
        width: term_area.width.saturating_sub(2),
        height: term_area.height.saturating_sub(2),
    };

    LayoutInfo {
        header: chunks[0],
        terminal: term_area,
        logs: body[1],
        input: chunks[2],
        term_inner,
    }
}

fn debug_lines(
    ui: &UiState,
    layout: &LayoutInfo,
    viewport: Rect,
    final_cursor_abs: Option<(u16, u16)>,
) -> Vec<Line<'static>> {
    let (rows, cols) = ui.pty.size();
    let cursor = ui.pty.cursor_state();
    let cursor_line = cursor
        .as_ref()
        .map(|c| format!("cursor(raw): row={}, col={}", c.row, c.col))
        .unwrap_or_else(|| "cursor(raw): (hidden)".to_string());
    let draw_cursor = cursor.map(|c| c.draw).unwrap_or(false);
    let follow = ui.pty.scroll_offset() == 0;
    let final_cursor_line = final_cursor_abs
        .map(|(x, y)| format!("cursor(abs): {},{}", x, y))
        .unwrap_or_else(|| "cursor(abs): (not drawn)".to_string());
    vec![
        Line::from(format!(
            "terminal rect: ({}, {}) {}x{}",
            layout.terminal.x, layout.terminal.y, layout.terminal.width, layout.terminal.height
        )),
        Line::from(format!(
            "inner rect: ({}, {}) {}x{}",
            layout.term_inner.x,
            layout.term_inner.y,
            layout.term_inner.width,
            layout.term_inner.height
        )),
        Line::from(cursor_line),
        Line::from(final_cursor_line),
        Line::from(format!("pty size: {}x{}", rows, cols)),
        Line::from(format!("viewport: {}x{}", viewport.height, viewport.width)),
        Line::from(format!("scroll_offset: {}", ui.pty.scroll_offset())),
        Line::from(format!("follow: {}", if follow { "yes" } else { "no" })),
        Line::from(format!(
            "alt_screen: {}",
            if ui.pty.alternate_screen() {
                "yes"
            } else {
                "no"
            }
        )),
        Line::from(format!(
            "draw_cursor: {}",
            if draw_cursor { "true" } else { "false" }
        )),
    ]
}

fn to_io_error(e: String) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e)
}

use std::{
    io::{self, Stdout},
    path::PathBuf,
    sync::mpsc::{self, TryRecvError},
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, MouseEventKind},
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
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
    ConfirmDelete,
}

struct UiState {
    focus: Focus,
    mode: InputMode,
    log_input: String,
    input_cursor: usize,
    pty: PtyTerminal,
    debug_overlay: bool,
    status_message: Option<(String, Instant)>,
    voice_task: Option<mpsc::Receiver<Result<String, String>>>,
    log_scroll_y: usize,
    log_scroll_x: usize,
    input_scroll_x: usize,
    selected_log_index: usize,
}

impl UiState {
    fn new(repo_root: PathBuf, rows: u16, cols: u16) -> Result<Self, String> {
        Ok(Self {
            focus: Focus::Terminal,
            mode: InputMode::Normal,
            log_input: String::new(),
            input_cursor: 0,
            pty: PtyTerminal::spawn(repo_root, rows, cols)?,
            debug_overlay: false,
            status_message: None,
            voice_task: None,
            log_scroll_y: 0,
            log_scroll_x: 0,
            input_scroll_x: 0,
            selected_log_index: 0,
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
            ui.input_cursor = 0;
            ui.input_scroll_x = 0;
        }
        if prev_branch != app.current_branch && ui.mode == InputMode::ConfirmDelete {
            ui.mode = InputMode::Normal;
        }
        if let Some((_, at)) = ui.status_message.as_ref() {
            if ui.voice_task.is_none() && at.elapsed() > Duration::from_secs(2) {
                ui.status_message = None;
            }
        }

        let layout = compute_layout(terminal.size()?);
        let input_inner_width = layout.input.width.saturating_sub(2) as usize;
        ui.pty
            .ensure_size(layout.term_inner.height, layout.term_inner.width);
        ui.pty.poll_output();

        let log_items_raw = app
            .log_store
            .list(&app.current_branch)
            .unwrap_or_default()
            .into_iter()
            .rev()
            .collect::<Vec<_>>();
        let log_items = log_items_raw
            .iter()
            .map(|it| format!("[{}] {}", it.created_at.format("%m-%d %H:%M"), it.text))
            .collect::<Vec<_>>();
        let log_inner_height = layout.logs.height.saturating_sub(2) as usize;
        if log_items.is_empty() {
            ui.selected_log_index = 0;
            ui.log_scroll_y = 0;
        } else {
            if ui.selected_log_index >= log_items.len() {
                ui.selected_log_index = log_items.len().saturating_sub(1);
            }
            if log_inner_height > 0 {
                let max_start = log_items.len().saturating_sub(log_inner_height);
                if ui.log_scroll_y > max_start {
                    ui.log_scroll_y = max_start;
                }
                if ui.selected_log_index < ui.log_scroll_y {
                    ui.log_scroll_y = ui.selected_log_index;
                } else if ui.selected_log_index >= ui.log_scroll_y + log_inner_height {
                    ui.log_scroll_y = ui.selected_log_index + 1 - log_inner_height;
                }
            } else {
                ui.log_scroll_y = 0;
            }
        }

        if let Some(rx) = ui.voice_task.as_ref() {
            match rx.try_recv() {
                Ok(result) => {
                    ui.voice_task = None;
                    match result {
                        Ok(t) => {
                            let trimmed = t.trim();
                            if trimmed.is_empty() {
                                ui.set_status("보이스 인식 결과 없음");
                            } else if let Err(e) =
                                app.log_store.append_text(&app.current_branch, trimmed)
                            {
                                ui.set_status(format!("보이스 로그 실패: {}", e));
                            } else {
                                ui.set_status("보이스 로그 추가됨");
                            }
                        }
                        Err(e) => {
                            ui.set_status(format!("보이스 인식 실패: {}", e));
                        }
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    ui.voice_task = None;
                    ui.set_status("보이스 인식 실패");
                }
            }
        }

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
            let log_inner_width = layout.logs.width.saturating_sub(2) as usize;
            let start = ui.log_scroll_y.min(log_items.len());
            let end = (start + log_inner_height).min(log_items.len());
            let items = log_items[start..end]
                .iter()
                .enumerate()
                .map(|(idx, line)| {
                    let sliced = slice_from_col(line, ui.log_scroll_x, log_inner_width);
                    let mut item = ListItem::new(Line::from(Span::raw(sliced)));
                    if start + idx == ui.selected_log_index {
                        item = item.style(Style::default().add_modifier(Modifier::REVERSED));
                    }
                    item
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
                            " Log input (press i to add, v=voice, Esc=switch, q=quit) "
                        }
                        (Focus::LogInput, InputMode::ConfirmDelete) => {
                            " Confirm delete (y/n) "
                        }
                        _ => " Log input (Esc to focus) ",
                    });

            let (input_text, cursor_col) = if matches!(ui.mode, InputMode::EditingLog) {
                if input_inner_width == 0 {
                    (String::new(), None)
                } else {
                    let width = ui.log_input.as_str().width();
                    let cursor_width = width_upto_char(&ui.log_input, ui.input_cursor);
                    let max_visible = input_inner_width.saturating_sub(1);
                    let max_start = width.saturating_sub(max_visible);
                    if ui.input_scroll_x > max_start {
                        ui.input_scroll_x = max_start;
                    }
                    let sliced =
                        slice_from_col(&ui.log_input, ui.input_scroll_x, input_inner_width);
                    let cursor = cursor_width.saturating_sub(ui.input_scroll_x).min(max_visible);
                    (sliced, Some(cursor as u16))
                }
            } else if matches!(ui.mode, InputMode::ConfirmDelete) {
                (
                    "정말 이 로그를 삭제할까요? [y] 삭제 / [n] 취소".to_string(),
                    None,
                )
            } else if ui.voice_task.is_some() {
                ("녹음중... 멈추면 자동 종료".to_string(), None)
            } else if let Some((ref msg, _)) = ui.status_message {
                (msg.clone(), None)
            } else {
                (String::new(), None)
            };
            let input = Paragraph::new(input_text).block(input_block);
            f.render_widget(input, layout.input);

            if matches!(ui.mode, InputMode::EditingLog) && ui.focus == Focus::LogInput {
                if let Some(col) = cursor_col {
                    f.set_cursor(layout.input.x + col + 1, layout.input.y + 1);
                }
            }
        })?;

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    if ui.mode == InputMode::ConfirmDelete {
                        match key.code {
                            KeyCode::Char('y') => {
                                if let Some(item) = log_items_raw.get(ui.selected_log_index) {
                                    if let Ok(true) = app.log_store.delete_by_id(
                                        &app.current_branch,
                                        &item.id,
                                    ) {
                                        ui.set_status("log deleted");
                                        let next_len = log_items_raw.len().saturating_sub(1);
                                        if next_len == 0 {
                                            ui.selected_log_index = 0;
                                        } else if ui.selected_log_index >= next_len {
                                            ui.selected_log_index = next_len - 1;
                                        }
                                    }
                                }
                                ui.mode = InputMode::Normal;
                            }
                            KeyCode::Char('n') | KeyCode::Esc => {
                                ui.mode = InputMode::Normal;
                            }
                            _ => {}
                        }
                        continue;
                    }
                    if key.code == KeyCode::Esc
                        && !(ui.focus == Focus::LogInput
                            && matches!(ui.mode, InputMode::EditingLog | InputMode::ConfirmDelete))
                    {
                        ui.focus = match ui.focus {
                            Focus::Terminal => Focus::LogInput,
                            Focus::LogInput => Focus::Terminal,
                        };
                        continue;
                    }

                    match key.code {
                        KeyCode::Char('q') if ui.focus == Focus::LogInput => break,
                        KeyCode::F(2) => {
                            ui.debug_overlay = !ui.debug_overlay;
                        }
                        _ => {}
                    }

                    match ui.focus {
                        Focus::Terminal => {
                            match key.code {
                                KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                    ui.pty.scroll_up(1);
                                }
                                KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                    ui.pty.scroll_down(1);
                                }
                                KeyCode::PageUp => ui.pty.scroll_up(5),
                                KeyCode::PageDown => ui.pty.scroll_down(5),
                                _ => {
                                    if let Some(bytes) = encode_key_event(key) {
                                        ui.pty.send_bytes(&bytes);
                                    }
                                }
                            }
                        }
                        Focus::LogInput => match ui.mode {
                            InputMode::Normal => match key.code {
                                KeyCode::Char('i') => {
                                    ui.mode = InputMode::EditingLog;
                                    ui.log_input.clear();
                                    ui.input_cursor = 0;
                                    ui.input_scroll_x = 0;
                                }
                                KeyCode::Char('d') => {
                                    if !log_items_raw.is_empty() {
                                        ui.mode = InputMode::ConfirmDelete;
                                    }
                                }
                                KeyCode::Char('v') => {
                                    if ui.voice_task.is_none() {
                                        let model_path = std::env::var("WHISPER_MODEL")
                                            .unwrap_or_else(|_| "models/ggml-tiny.bin".to_string());
                                        let (tx, rx) =
                                            mpsc::channel::<Result<String, String>>();
                                        ui.voice_task = Some(rx);
                                        ui.set_status("녹음중... 멈추면 자동 종료");

                                        std::thread::spawn(move || {
                                            let config = voice::VadConfig::default();
                                            let result =
                                                voice::transcribe_from_mic_vad(&model_path, config);
                                            let _ = tx.send(result);
                                        });
                                    }
                                }
                                KeyCode::Up => {
                                    ui.selected_log_index =
                                        ui.selected_log_index.saturating_sub(1);
                                }
                                KeyCode::Down => {
                                    if ui.selected_log_index + 1 < log_items_raw.len() {
                                        ui.selected_log_index += 1;
                                    }
                                }
                                KeyCode::PageUp => {
                                    let step = log_inner_height.max(1);
                                    ui.selected_log_index =
                                        ui.selected_log_index.saturating_sub(step);
                                }
                                KeyCode::PageDown => {
                                    let step = log_inner_height.max(1);
                                    let next = ui.selected_log_index.saturating_add(step);
                                    if log_items_raw.is_empty() {
                                        ui.selected_log_index = 0;
                                    } else {
                                        ui.selected_log_index =
                                            next.min(log_items_raw.len().saturating_sub(1));
                                    }
                                }
                                KeyCode::Left => {
                                    ui.log_scroll_x = ui.log_scroll_x.saturating_sub(4);
                                }
                                KeyCode::Right => {
                                    ui.log_scroll_x = ui.log_scroll_x.saturating_add(4);
                                }
                                KeyCode::Home => {
                                    ui.log_scroll_x = 0;
                                }
                                _ => {}
                            },
                            InputMode::ConfirmDelete => {}
                            InputMode::EditingLog => match key.code {
                                KeyCode::Esc => {
                                    ui.mode = InputMode::Normal;
                                    ui.log_input.clear();
                                    ui.input_cursor = 0;
                                    ui.input_scroll_x = 0;
                                }
                                KeyCode::Enter => {
                                    if !ui.log_input.trim().is_empty() {
                                        let _ = app
                                            .log_store
                                            .append_text(&app.current_branch, &ui.log_input);
                                    }
                                    ui.log_input.clear();
                                    ui.mode = InputMode::Normal;
                                    ui.input_cursor = 0;
                                    ui.input_scroll_x = 0;
                                }
                                KeyCode::Backspace => {
                                    if ui.input_cursor > 0 {
                                        let idx = byte_index_from_char(
                                            &ui.log_input,
                                            ui.input_cursor - 1,
                                        );
                                        let next_idx =
                                            byte_index_from_char(&ui.log_input, ui.input_cursor);
                                        ui.log_input.replace_range(idx..next_idx, "");
                                        ui.input_cursor -= 1;
                                        ui.input_scroll_x = adjust_input_scroll(
                                            &ui.log_input,
                                            ui.input_cursor,
                                            input_inner_width,
                                            ui.input_scroll_x,
                                        );
                                    }
                                }
                                KeyCode::Delete => {
                                    let len = ui.log_input.chars().count();
                                    if ui.input_cursor < len {
                                        let idx =
                                            byte_index_from_char(&ui.log_input, ui.input_cursor);
                                        let next_idx = byte_index_from_char(
                                            &ui.log_input,
                                            ui.input_cursor + 1,
                                        );
                                        ui.log_input.replace_range(idx..next_idx, "");
                                        ui.input_scroll_x = adjust_input_scroll(
                                            &ui.log_input,
                                            ui.input_cursor,
                                            input_inner_width,
                                            ui.input_scroll_x,
                                        );
                                    }
                                }
                                KeyCode::Left => {
                                    if ui.input_cursor > 0 {
                                        ui.input_cursor -= 1;
                                    }
                                    ui.input_scroll_x = adjust_input_scroll(
                                        &ui.log_input,
                                        ui.input_cursor,
                                        input_inner_width,
                                        ui.input_scroll_x,
                                    );
                                }
                                KeyCode::Right => {
                                    let len = ui.log_input.chars().count();
                                    if ui.input_cursor < len {
                                        ui.input_cursor += 1;
                                    }
                                    ui.input_scroll_x = adjust_input_scroll(
                                        &ui.log_input,
                                        ui.input_cursor,
                                        input_inner_width,
                                        ui.input_scroll_x,
                                    );
                                }
                                KeyCode::Home => {
                                    ui.input_cursor = 0;
                                    ui.input_scroll_x = 0;
                                }
                                KeyCode::End => {
                                    ui.input_cursor = ui.log_input.chars().count();
                                    ui.input_scroll_x = adjust_input_scroll(
                                        &ui.log_input,
                                        ui.input_cursor,
                                        input_inner_width,
                                        ui.input_scroll_x,
                                    );
                                }
                                KeyCode::Char(c) => {
                                    let idx =
                                        byte_index_from_char(&ui.log_input, ui.input_cursor);
                                    ui.log_input.insert(idx, c);
                                    ui.input_cursor += 1;
                                    ui.input_scroll_x = adjust_input_scroll(
                                        &ui.log_input,
                                        ui.input_cursor,
                                        input_inner_width,
                                        ui.input_scroll_x,
                                    );
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

fn slice_from_col(text: &str, start_col: usize, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }

    let mut out = String::new();
    let mut col = 0;

    for ch in text.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if col + w <= start_col {
            col += w;
            continue;
        }
        if col >= start_col + max_cols {
            break;
        }
        out.push(ch);
        col += w;
    }

    out
}

fn width_upto_char(text: &str, char_idx: usize) -> usize {
    let mut width = 0;
    for (i, ch) in text.chars().enumerate() {
        if i >= char_idx {
            break;
        }
        width += UnicodeWidthChar::width(ch).unwrap_or(0);
    }
    width
}

fn byte_index_from_char(text: &str, char_idx: usize) -> usize {
    if char_idx == 0 {
        return 0;
    }
    for (i, (byte_idx, _)) in text.char_indices().enumerate() {
        if i == char_idx {
            return byte_idx;
        }
    }
    text.len()
}

fn adjust_input_scroll(
    text: &str,
    cursor: usize,
    inner_width: usize,
    current_scroll: usize,
) -> usize {
    if inner_width == 0 {
        return 0;
    }
    let max_visible = inner_width.saturating_sub(1);
    let total_width = text.width();
    let cursor_width = width_upto_char(text, cursor);
    let mut scroll = current_scroll;

    if cursor_width < scroll {
        scroll = cursor_width;
    } else if cursor_width > scroll + max_visible {
        scroll = cursor_width.saturating_sub(max_visible);
    }

    let max_start = total_width.saturating_sub(max_visible);
    scroll.min(max_start)
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

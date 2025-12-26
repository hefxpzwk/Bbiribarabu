use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use portable_pty::{CommandBuilder, MasterPty, PtyPair, PtySize, native_pty_system};
use vt100::{Parser, Screen};

/// Owns the PTY handles and moves raw bytes between the shell and the UI.
pub struct PtyShell {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    rx: Receiver<Vec<u8>>,
    _child: Box<dyn portable_pty::Child + Send>,
}

impl PtyShell {
    pub fn spawn(repo_root: PathBuf, rows: u16, cols: u16) -> Result<Self, String> {
        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair: PtyPair = native_pty_system()
            .openpty(size)
            .map_err(|e| format!("openpty failed: {e}"))?;

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "bash".to_string());
        let mut cmd = CommandBuilder::new(shell);
        cmd.cwd(repo_root);

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("spawn shell failed: {e}"))?;

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("clone reader failed: {e}"))?;
        let (tx, rx) = mpsc::channel();
        spawn_reader_thread(reader, tx);

        let master = pair.master;
        let writer = master
            .take_writer()
            .map_err(|e| format!("take_writer failed: {e}"))?;

        Ok(Self {
            master,
            writer,
            rx,
            _child: child,
        })
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
    }

    pub fn write(&mut self, bytes: &[u8]) {
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    pub fn try_read(&mut self) -> Option<Vec<u8>> {
        self.rx.try_recv().ok()
    }
}

/// High-level terminal abstraction that keeps PTY I/O separate from UI rendering.
pub struct PtyTerminal {
    shell: PtyShell,
    parser: Parser,
}

#[derive(Debug, Clone, Copy)]
pub struct CursorState {
    pub draw: bool,
    pub row: u16,
    pub col: u16,
}

impl PtyTerminal {
    pub fn spawn(repo_root: PathBuf, rows: u16, cols: u16) -> Result<Self, String> {
        Ok(Self {
            shell: PtyShell::spawn(repo_root, rows, cols)?,
            parser: Parser::new(rows, cols, 10_000),
        })
    }

    /// Keep shell/VT size in sync with the current panel inner size.
    pub fn ensure_size(&mut self, rows: u16, cols: u16) {
        let (current_rows, current_cols) = self.parser.screen().size();
        if current_rows != rows || current_cols != cols {
            let offset = self.scroll_offset();
            self.shell.resize(rows, cols);
            self.parser.set_size(rows, cols);
            self.parser.set_scrollback(offset);
        }
    }

    pub fn send_bytes(&mut self, bytes: &[u8]) {
        // Only follow the live view when already at the bottom; keep sticky scroll otherwise.
        if self.scroll_offset() == 0 {
            self.parser.set_scrollback(0);
        }
        self.shell.write(bytes);
    }

    pub fn poll_output(&mut self) {
        while let Some(bytes) = self.shell.try_read() {
            // Preserve raw stream; vt100 handles control sequences internally.
            self.parser.process(&bytes);
        }
    }

    pub fn screen(&self) -> &Screen {
        self.parser.screen()
    }

    pub fn cursor_state(&self) -> Option<CursorState> {
        let screen = self.parser.screen();
        if self.scroll_offset() > 0 || screen.hide_cursor() {
            return None;
        }
        let (row, col) = screen.cursor_position();
        Some(CursorState {
            draw: true,
            row,
            col,
        })
    }

    pub fn scroll_up(&mut self, lines: usize) {
        let offset = self.scroll_offset().saturating_add(lines);
        self.parser.set_scrollback(offset);
    }

    pub fn scroll_down(&mut self, lines: usize) {
        let offset = self.scroll_offset().saturating_sub(lines);
        self.parser.set_scrollback(offset);
    }

    pub fn scroll_offset(&self) -> usize {
        self.parser.screen().scrollback()
    }

    pub fn size(&self) -> (u16, u16) {
        self.parser.screen().size()
    }

    pub fn alternate_screen(&self) -> bool {
        self.parser.screen().alternate_screen()
    }
}

fn spawn_reader_thread(mut reader: Box<dyn Read + Send>, tx: Sender<Vec<u8>>) {
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let _ = tx.send(buf[..n].to_vec());
                }
                Err(_) => thread::sleep(Duration::from_millis(5)),
            }
        }
    });
}

/// Converts crossterm key events into the raw byte sequences expected by a PTY-backed shell.
pub fn encode_key_event(key: KeyEvent) -> Option<Vec<u8>> {
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }

    match key.code {
        KeyCode::Char(c) => {
            let mut bytes = if key.modifiers.contains(KeyModifiers::CONTROL) {
                let upper = c.to_ascii_uppercase();
                let ctrl = match upper {
                    '@' => 0x00,
                    'A'..='Z' => upper as u8 - b'A' + 1,
                    '[' => 0x1b,
                    '\\' => 0x1c,
                    ']' => 0x1d,
                    '^' | '6' => 0x1e,
                    '_' => 0x1f,
                    ' ' => 0x00,
                    _ => return None,
                };
                vec![ctrl]
            } else {
                c.to_string().into_bytes()
            };

            if key.modifiers.contains(KeyModifiers::ALT) {
                bytes.insert(0, 0x1b);
            }
            Some(bytes)
        }
        KeyCode::Enter => Some(vec![b'\r']),
        KeyCode::Backspace => Some(vec![0x7f]),
        KeyCode::Tab => Some(vec![b'\t']),
        KeyCode::BackTab => Some(b"\x1b[Z".to_vec()),
        KeyCode::Esc => Some(vec![0x1b]),
        KeyCode::Left => Some(b"\x1b[D".to_vec()),
        KeyCode::Right => Some(b"\x1b[C".to_vec()),
        KeyCode::Up => Some(b"\x1b[A".to_vec()),
        KeyCode::Down => Some(b"\x1b[B".to_vec()),
        KeyCode::Home => Some(b"\x1b[H".to_vec()),
        KeyCode::End => Some(b"\x1b[F".to_vec()),
        KeyCode::PageUp => Some(b"\x1b[5~".to_vec()),
        KeyCode::PageDown => Some(b"\x1b[6~".to_vec()),
        KeyCode::Delete => Some(b"\x1b[3~".to_vec()),
        KeyCode::Insert => Some(b"\x1b[2~".to_vec()),
        KeyCode::F(n) if (1..=12).contains(&n) => {
            // Match common xterm sequences for F1-F12.
            let seq = match n {
                1 => "\x1bOP",
                2 => "\x1bOQ",
                3 => "\x1bOR",
                4 => "\x1bOS",
                5 => "\x1b[15~",
                6 => "\x1b[17~",
                7 => "\x1b[18~",
                8 => "\x1b[19~",
                9 => "\x1b[20~",
                10 => "\x1b[21~",
                11 => "\x1b[23~",
                12 => "\x1b[24~",
                _ => return None,
            };
            Some(seq.as_bytes().to_vec())
        }
        _ => None,
    }
}

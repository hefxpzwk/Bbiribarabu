use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

#[derive(Clone, Copy)]
pub enum OutputKind {
    Command,
    Stdout,
    Stderr,
    Info,
}

#[derive(Clone)]
pub struct OutputLine {
    pub kind: OutputKind,
    pub text: String,
}

/// Long-lived bash runner with buffered output and scroll support.
pub struct TerminalRunner {
    pub input: String,
    pub scroll: usize,
    buffer: VecDeque<OutputLine>,
    max_lines: usize,
    tx_cmd: Sender<String>,
    rx_out: Receiver<OutputLine>,
}

impl TerminalRunner {
    pub fn new(repo_root: PathBuf) -> Self {
        let (tx_cmd, rx_cmd) = mpsc::channel::<String>();
        let (tx_out, rx_out) = mpsc::channel::<OutputLine>();

        thread::spawn(move || Self::command_loop(repo_root, rx_cmd, tx_out));

        let mut runner = Self {
            input: String::new(),
            scroll: 0,
            buffer: VecDeque::new(),
            max_lines: 500,
            tx_cmd,
            rx_out,
        };

        runner.push_output(
            OutputKind::Info,
            "Shell started (bash --noprofile --norc)".into(),
        );
        runner
    }

    fn command_loop(repo_root: PathBuf, rx_cmd: Receiver<String>, tx_out: Sender<OutputLine>) {
        let mut child = match Command::new("bash")
            .arg("--noprofile")
            .arg("--norc")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(&repo_root)
            .spawn()
        {
            Ok(child) => child,
            Err(err) => {
                let _ = tx_out.send(OutputLine {
                    kind: OutputKind::Stderr,
                    text: format!("[error] failed to start bash: {}", err),
                });
                return;
            }
        };

        // stdout reader
        if let Some(stdout) = child.stdout.take() {
            let tx = tx_out.clone();
            thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().flatten() {
                    let _ = tx.send(OutputLine {
                        kind: OutputKind::Stdout,
                        text: line,
                    });
                }
            });
        }

        // stderr reader
        if let Some(stderr) = child.stderr.take() {
            let tx = tx_out.clone();
            thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().flatten() {
                    let _ = tx.send(OutputLine {
                        kind: OutputKind::Stderr,
                        text: line,
                    });
                }
            });
        }

        while let Ok(cmd) = rx_cmd.recv() {
            let trimmed = cmd.trim();
            if trimmed.is_empty() {
                continue;
            }

            let _ = tx_out.send(OutputLine {
                kind: OutputKind::Command,
                text: format!("$ {}", trimmed),
            });

            match child.stdin.as_mut() {
                Some(stdin) => {
                    if writeln!(stdin, "{}", trimmed).is_err() {
                        let _ = tx_out.send(OutputLine {
                            kind: OutputKind::Stderr,
                            text: "[error] failed to write to shell stdin".into(),
                        });
                        break;
                    }
                    let _ = stdin.flush();
                }
                None => {
                    let _ = tx_out.send(OutputLine {
                        kind: OutputKind::Stderr,
                        text: "[error] shell stdin closed".into(),
                    });
                    break;
                }
            }
        }
    }

    fn push_output(&mut self, kind: OutputKind, text: String) {
        self.buffer.push_back(OutputLine { kind, text });
        if self.buffer.len() > self.max_lines {
            self.buffer.pop_front();
        }
        let max_scroll = self.buffer.len().saturating_sub(1);
        if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }
    }

    pub fn run_command(&mut self, cmd: &str) {
        let trimmed = cmd.trim();
        if trimmed.is_empty() {
            return;
        }
        self.scroll = 0;
        let _ = self.tx_cmd.send(trimmed.to_string());
    }

    /// Drain pending output without blocking and keep only the latest lines.
    pub fn poll_output(&mut self) {
        while let Ok(line) = self.rx_out.try_recv() {
            self.push_output(line.kind, line.text);
        }
    }

    pub fn visible_lines(&self, height: usize) -> Vec<OutputLine> {
        if height == 0 {
            return Vec::new();
        }
        let total = self.buffer.len();
        let end = total.saturating_sub(self.scroll);
        let start = end.saturating_sub(height);
        self.buffer
            .iter()
            .skip(start)
            .take(end.saturating_sub(start))
            .cloned()
            .collect()
    }

    pub fn buffer_len(&self) -> usize {
        self.buffer.len()
    }

    pub fn scroll_up(&mut self, amount: usize) {
        let max_scroll = self.buffer.len().saturating_sub(1);
        self.scroll = (self.scroll + amount).min(max_scroll);
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll = self.scroll.saturating_sub(amount);
    }
}

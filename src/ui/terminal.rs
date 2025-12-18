use std::{
    collections::VecDeque,
    path::PathBuf,
    process::Command,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

#[derive(Clone, Copy, PartialEq)]
pub enum OutputKind {
    Command {
        exit_code: i32,
    },
    Stdout,
    Stderr {
        exit_success: bool,
        is_errorish: bool,
    },
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
            max_lines: 1000,
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
        while let Ok(cmd) = rx_cmd.recv() {
            let trimmed = cmd.trim();
            if trimmed.is_empty() {
                continue;
            }

            let output = Command::new("bash")
                .arg("--noprofile")
                .arg("--norc")
                .arg("-lc")
                .arg(trimmed)
                .current_dir(&repo_root)
                .output();

            let (status, stdout, stderr) = match output {
                Ok(out) => (
                    out.status,
                    String::from_utf8_lossy(&out.stdout).into_owned(),
                    String::from_utf8_lossy(&out.stderr).into_owned(),
                ),
                Err(err) => {
                    let _ = tx_out.send(OutputLine {
                        kind: OutputKind::Command { exit_code: -1 },
                        text: format!("$ {}   (failed to start)", trimmed),
                    });
                    let _ = tx_out.send(OutputLine {
                        kind: OutputKind::Stderr {
                            exit_success: false,
                            is_errorish: true,
                        },
                        text: format!("error: {}", err),
                    });
                    continue;
                }
            };

            let exit_code = status.code().unwrap_or(-1);
            let success = status.success();

            let _ = tx_out.send(OutputLine {
                kind: OutputKind::Command { exit_code },
                text: format!("$ {}   (exit={})", trimmed, exit_code),
            });

            for line in stdout.lines() {
                let _ = tx_out.send(OutputLine {
                    kind: OutputKind::Stdout,
                    text: line.to_string(),
                });
            }

            for line in stderr.lines() {
                let lowered = line.trim_start().to_lowercase();
                let is_error_prefix =
                    lowered.starts_with("error:") || lowered.starts_with("fatal:");
                let kind = OutputKind::Stderr {
                    exit_success: success,
                    is_errorish: !success || is_error_prefix,
                };
                let _ = tx_out.send(OutputLine {
                    kind,
                    text: line.to_string(),
                });
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
        if self.scroll == 0 {
            self.scroll = 0; // keep at bottom
        }
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

    pub fn scroll_up(&mut self, amount: usize) {
        let max_scroll = self.buffer.len().saturating_sub(1);
        self.scroll = (self.scroll + amount).min(max_scroll);
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll = self.scroll.saturating_sub(amount);
    }
}

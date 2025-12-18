use std::{
    collections::VecDeque,
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

/// Runs bash commands per request and keeps recent output with scroll support.
pub struct TerminalRunner {
    pub input: String,
    pub scroll: usize,
    buffer: VecDeque<String>,
    max_lines: usize,
    tx_cmd: Sender<String>,
    rx_out: Receiver<String>,
}

impl TerminalRunner {
    pub fn new(repo_root: PathBuf) -> Self {
        let (tx_cmd, rx_cmd) = mpsc::channel::<String>();
        let (tx_out, rx_out) = mpsc::channel::<String>();

        thread::spawn(move || Self::command_loop(repo_root, rx_cmd, tx_out));

        Self {
            input: String::new(),
            scroll: 0,
            buffer: VecDeque::new(),
            max_lines: 500,
            tx_cmd,
            rx_out,
        }
    }

    fn command_loop(repo_root: PathBuf, rx_cmd: Receiver<String>, tx_out: Sender<String>) {
        for cmd in rx_cmd {
            let trimmed = cmd.trim();
            if trimmed.is_empty() {
                continue;
            }

            let _ = tx_out.send(format!("$ {}", trimmed));

            let mut child = match Command::new("bash")
                .arg("-lc")
                .arg(trimmed)
                .current_dir(&repo_root)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
            {
                Ok(child) => child,
                Err(err) => {
                    let _ = tx_out.send(format!("[error] {}", err));
                    continue;
                }
            };

            if let Some(stdout) = child.stdout.take() {
                let tx = tx_out.clone();
                thread::spawn(move || {
                    let reader = BufReader::new(stdout);
                    for line in reader.lines().flatten() {
                        let _ = tx.send(line);
                    }
                });
            }

            if let Some(stderr) = child.stderr.take() {
                let tx = tx_out.clone();
                thread::spawn(move || {
                    let reader = BufReader::new(stderr);
                    for line in reader.lines().flatten() {
                        let _ = tx.send(line);
                    }
                });
            }

            let _ = child.wait();
        }
    }

    pub fn run_command(&mut self, cmd: &str) {
        self.scroll = 0;
        let _ = self.tx_cmd.send(cmd.to_string());
    }

    /// Drain pending output without blocking and keep only the latest lines.
    pub fn poll_output(&mut self) {
        let mut changed = false;
        while let Ok(line) = self.rx_out.try_recv() {
            self.buffer.push_back(line);
            if self.buffer.len() > self.max_lines {
                self.buffer.pop_front();
            }
            changed = true;
        }

        if changed {
            let max_scroll = self.buffer.len().saturating_sub(1);
            if self.scroll > max_scroll {
                self.scroll = max_scroll;
            }
        }
    }

    pub fn visible_lines(&self, height: usize) -> Vec<String> {
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

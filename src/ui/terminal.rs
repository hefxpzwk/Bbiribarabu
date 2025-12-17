use std::{
    collections::VecDeque,
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

/// Executes shell commands in the repo root and streams stdout/stderr lines.
pub struct ShellRunner {
    tx_cmd: Sender<String>,
    rx_out: Receiver<String>,
    buffer: VecDeque<String>,
    max_lines: usize,
}

impl ShellRunner {
    pub fn new(repo_root: PathBuf) -> Self {
        let (tx_cmd, rx_cmd) = mpsc::channel::<String>();
        let (tx_out, rx_out) = mpsc::channel::<String>();

        thread::spawn(move || Self::command_loop(repo_root, rx_cmd, tx_out));

        Self {
            tx_cmd,
            rx_out,
            buffer: VecDeque::new(),
            max_lines: 300,
        }
    }

    fn command_loop(repo_root: PathBuf, rx_cmd: Receiver<String>, tx_out: Sender<String>) {
        for cmd in rx_cmd {
            if cmd.trim().is_empty() {
                continue;
            }

            let _ = tx_out.send(format!("$ {}", cmd));

            let mut child = match Command::new("bash")
                .arg("-lc")
                .arg(&cmd)
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

    pub fn run_command(&self, cmd: String) {
        let _ = self.tx_cmd.send(cmd);
    }

    /// Drain pending output without blocking and keep only the latest lines.
    pub fn poll_output(&mut self) {
        while let Ok(line) = self.rx_out.try_recv() {
            self.buffer.push_back(line);
            if self.buffer.len() > self.max_lines {
                self.buffer.pop_front();
            }
        }
    }

    pub fn recent_lines(&self, count: usize) -> Vec<String> {
        let len = self.buffer.len();
        let start = len.saturating_sub(count);
        self.buffer.iter().skip(start).cloned().collect()
    }
}

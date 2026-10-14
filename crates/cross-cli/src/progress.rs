//! Single-line download/extract progress on stderr.

use std::io::{IsTerminal, Write};

use cross_engine::Progress;

/// Single-line progress on stderr; silent when stderr is not a terminal.
pub struct ProgressRenderer {
    tty: bool,
    last: Option<(String, u64)>,
    line_open: bool,
}

impl ProgressRenderer {
    pub fn new() -> Self {
        Self {
            tty: std::io::stderr().is_terminal(),
            last: None,
            line_open: false,
        }
    }

    pub fn update(&mut self, progress: Progress<'_>) {
        if !self.tty {
            return;
        }
        let (label, text, key) = match progress {
            Progress::Downloading { label, done, total } => {
                let mb = |n: u64| n / (1024 * 1024);
                let text = match total {
                    Some(total) if total > 0 => format!(
                        "downloading {:>3}% ({}/{} MB)",
                        done * 100 / total,
                        mb(done),
                        mb(total)
                    ),
                    _ => format!("downloading {} MB", mb(done)),
                };
                (label, text, mb(done))
            }
            Progress::CacheHit { label } => (label, "using cached download".to_owned(), 0),
            Progress::Verifying { label } => (label, "verifying".to_owned(), 0),
            Progress::Extracting { label } => (label, "extracting".to_owned(), 0),
        };
        if self.last.as_ref() == Some(&(text.clone(), key)) {
            return;
        }
        self.last = Some((text.clone(), key));
        eprint!("\r\x1b[2K{label}: {text}");
        std::io::stderr().flush().ok();
        self.line_open = true;
    }

    pub fn finish(&mut self) {
        if self.line_open {
            eprintln!();
            self.line_open = false;
        }
    }
}

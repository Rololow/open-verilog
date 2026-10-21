//! Diagnostics: a message at a position, rendered with its source line and a caret.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub message: String,
    pub file: String,
    pub line: usize,
    pub col: usize,
}

impl Error {
    pub fn new(message: impl Into<String>, file: &str, line: usize, col: usize) -> Self {
        Error { message: message.into(), file: file.to_string(), line, col }
    }

    /// The diagnostic with the offending source line and a caret under the column.
    pub fn render(&self, source: &str) -> String {
        let text = source.lines().nth(self.line.saturating_sub(1)).unwrap_or("");
        let pad: String = text.chars().take(self.col.saturating_sub(1)).map(|c| if c == '\t' { '\t' } else { ' ' }).collect();
        format!("{}:{}:{}: error: {}\n {:>4} | {}\n      | {}^", self.file, self.line, self.col, self.message, self.line, text, pad)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}: error: {}", self.file, self.line, self.col, self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

pub const DEFAULT_CAPACITY: usize = 4096;
pub const BROADCAST_CAPACITY: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    Stdout,
    Stderr,
    App,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
        }
    }

    /// Single-character form used in the on-disk log file.
    pub fn as_char(self) -> char {
        match self {
            LogLevel::Debug => 'D',
            LogLevel::Info => 'I',
            LogLevel::Warn => 'W',
            LogLevel::Error => 'E',
        }
    }
}

/// Heuristic level detection for core output.
///
/// The three cores do not share a log format, and stderr is used for both
/// fatal errors and ordinary warnings, so level inference is best-effort. It is
/// only used for filtering in the UI; it never affects control flow.
pub fn infer_level(stream: LogStream, text: &str) -> LogLevel {
    let lowered = text.to_ascii_lowercase();
    if lowered.contains("fatal") || lowered.contains("panic") || lowered.contains("level=error") {
        return LogLevel::Error;
    }
    if lowered.contains("error") || lowered.contains("failed") {
        return LogLevel::Error;
    }
    if lowered.contains("warn") {
        return LogLevel::Warn;
    }
    if lowered.contains("debug") || lowered.contains("trace") {
        return LogLevel::Debug;
    }
    match stream {
        LogStream::Stderr => LogLevel::Warn,
        _ => LogLevel::Info,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLine {
    pub seq: u64,
    pub ts_ms: u64,
    pub core: String,
    pub stream: LogStream,
    pub level: LogLevel,
    pub text: String,
}

impl LogLine {
    /// `HH:MM:SS.mmm` in local time is not available without a tz crate; the
    /// UI renders timestamps itself, so the raw epoch millis are kept here.
    pub fn timestamp_ms(&self) -> u64 {
        self.ts_ms
    }
}

struct Inner {
    lines: VecDeque<LogLine>,
    dropped: u64,
}

/// Bounded, append-only log ring buffer with live fan-out.
///
/// The plan (ch.34) requires a ring buffer so that a chatty core cannot grow
/// memory without bound. When the buffer is full the oldest line is dropped and
/// a counter is incremented, which the UI can surface as "N lines dropped".
pub struct LogBuffer {
    capacity: usize,
    inner: Mutex<Inner>,
    seq: AtomicU64,
    tx: broadcast::Sender<LogLine>,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(16);
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            capacity,
            inner: Mutex::new(Inner {
                lines: VecDeque::with_capacity(capacity),
                dropped: 0,
            }),
            seq: AtomicU64::new(0),
            tx,
        }
    }

    pub fn push(&self, core: &str, stream: LogStream, text: impl Into<String>) {
        let text = text.into();
        let level = infer_level(stream, &text);
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let line = LogLine {
            seq,
            ts_ms: now_ms(),
            core: core.to_string(),
            stream,
            level,
            text,
        };

        {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if guard.lines.len() == self.capacity {
                guard.lines.pop_front();
                guard.dropped += 1;
            }
            guard.lines.push_back(line.clone());
        }

        // A send error only means no subscriber is attached right now.
        let _ = self.tx.send(line);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LogLine> {
        self.tx.subscribe()
    }

    /// Last `n` lines, oldest first.
    pub fn tail(&self, n: usize) -> Vec<LogLine> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let skip = guard.lines.len().saturating_sub(n);
        guard.lines.iter().skip(skip).cloned().collect()
    }

    pub fn tail_for(&self, core: &str, n: usize) -> Vec<LogLine> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut out: Vec<LogLine> = guard
            .lines
            .iter()
            .filter(|l| l.core == core)
            .cloned()
            .collect();
        let skip = out.len().saturating_sub(n);
        out.drain(..skip);
        out
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn dropped(&self) -> u64 {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).dropped
    }

    /// Total number of lines ever pushed.
    pub fn total(&self) -> u64 {
        self.seq.load(Ordering::Relaxed)
    }
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_drops_oldest_and_counts() {
        let buffer = LogBuffer::new(16);
        for i in 0..100 {
            buffer.push("mihomo", LogStream::Stdout, format!("line {i}"));
        }
        assert_eq!(buffer.len(), 16);
        assert_eq!(buffer.dropped(), 84);
        let tail = buffer.tail(2);
        assert_eq!(tail[1].text, "line 99");
    }

    #[test]
    fn level_inference_prefers_fatal_markers() {
        assert_eq!(infer_level(LogStream::Stderr, "FATAL bind failed"), LogLevel::Error);
        assert_eq!(infer_level(LogStream::Stdout, "level=warning msg=deprecated"), LogLevel::Warn);
        assert_eq!(infer_level(LogStream::Stdout, "inbound started"), LogLevel::Info);
    }

    #[test]
    fn tail_for_filters_by_core() {
        let buffer = LogBuffer::new(64);
        buffer.push("mihomo", LogStream::Stdout, "a");
        buffer.push("xray", LogStream::Stdout, "b");
        buffer.push("mihomo", LogStream::Stdout, "c");
        let tail = buffer.tail_for("mihomo", 10);
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].text, "a");
        assert_eq!(tail[1].text, "c");
    }
}

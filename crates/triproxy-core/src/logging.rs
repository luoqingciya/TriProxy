// TriProxy — a modern multi-core proxy client.
// Copyright (C) 2026 luoqingci
// SPDX-License-Identifier: GPL-3.0-or-later
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, either version 3 of the License, or (at your option) any later
// version. It is distributed in the hope that it will be useful, but WITHOUT ANY
// WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
// PARTICULAR PURPOSE. See LICENSE for the full text.
//
// Additional terms under GNU GPL v3 section 7 apply: see ADDITIONAL-TERMS.md.

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

/// Remove ANSI escape sequences from a line of core output.
///
/// All three cores force colour even when their output is a pipe, so every line
/// arrives wrapped in SGR codes (`ESC[36mINFO ESC[0m[0000] ...`). Left alone they
/// render as literal `[36m` garbage in the log panel and — worse — they sit in
/// the middle of the level word, which is what made level detection miss it.
pub fn strip_ansi(text: &str) -> String {
    if !text.contains('\u{1b}') {
        return text.trim_end_matches('\r').to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            if ch != '\r' {
                out.push(ch);
            }
            continue;
        }
        match chars.peek().copied() {
            // CSI: ESC '[' <params> <final byte in @..=~>
            Some('[') => {
                chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            // OSC: ESC ']' ... terminated by BEL or ESC '\'
            Some(']') => {
                chars.next();
                for c in chars.by_ref() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' {
                        chars.next();
                        break;
                    }
                }
            }
            // Any other two-byte escape is dropped as a pair.
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    out
}

/// Heuristic level detection for core output.
///
/// The three cores do not share a log format and all of them write ordinary
/// output to stderr, so the stream carries no severity information: an INFO
/// line from sing-box and a fatal bind error both arrive on stderr. Detection
/// therefore reads the level word the core declares, then falls back to
/// keywords in the message. It is only used for filtering in the UI; it never
/// affects control flow.
pub fn infer_level(_stream: LogStream, text: &str) -> LogLevel {
    if let Some(level) = declared_level(text) {
        return level;
    }
    let lowered = text.to_ascii_lowercase();
    if lowered.contains("fatal") || lowered.contains("panic") {
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
    LogLevel::Info
}

/// The level word declared by the core itself, if any.
///
/// One token scan covers all three formats because each puts a bare level word
/// on the line: sing-box/logrus `INFO[0000]`, mihomo `time="..." level=info`,
/// xray `2026/11/02 13:43:34 [Info]`. The first match wins, which matters for
/// mihomo: the declared level precedes the message, so a message that happens to
/// contain the word "error" cannot override `level=info`.
fn declared_level(text: &str) -> Option<LogLevel> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .find_map(level_word)
}

fn level_word(token: &str) -> Option<LogLevel> {
    const TABLE: [(&str, LogLevel); 14] = [
        ("info", LogLevel::Info),
        ("notice", LogLevel::Info),
        ("warn", LogLevel::Warn),
        ("warning", LogLevel::Warn),
        ("error", LogLevel::Error),
        ("err", LogLevel::Error),
        ("fatal", LogLevel::Error),
        ("panic", LogLevel::Error),
        ("critical", LogLevel::Error),
        ("severe", LogLevel::Error),
        ("debug", LogLevel::Debug),
        ("trace", LogLevel::Debug),
        ("verbose", LogLevel::Debug),
        ("dbg", LogLevel::Debug),
    ];
    TABLE
        .iter()
        .find(|(word, _)| token.eq_ignore_ascii_case(word))
        .map(|(_, level)| *level)
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
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .lines
            .len()
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
        assert_eq!(
            infer_level(LogStream::Stderr, "FATAL bind failed"),
            LogLevel::Error
        );
        assert_eq!(
            infer_level(LogStream::Stdout, "level=warning msg=deprecated"),
            LogLevel::Warn
        );
        assert_eq!(
            infer_level(LogStream::Stdout, "inbound started"),
            LogLevel::Info
        );
    }

    /// sing-box writes *everything* to stderr, including INFO, so the stream
    /// must not turn a normal connection line into a warning.
    #[test]
    fn singbox_info_on_stderr_is_not_a_warning() {
        let line = "+0800 2026-11-02 13:43:34 INFO[0000] inbound connection from 127.0.0.1:60270";
        assert_eq!(infer_level(LogStream::Stderr, line), LogLevel::Info);
    }

    #[test]
    fn declared_level_wins_over_message_content() {
        // mihomo puts the level first; the message may contain anything.
        let line = r#"time="2026-11-02T13:43:34Z" level=info msg="[TCP] connection error: EOF""#;
        assert_eq!(infer_level(LogStream::Stderr, line), LogLevel::Info);
    }

    #[test]
    fn xray_bracket_level_is_read() {
        let line = "2026/11/02 13:43:34 [Warning] failed to read geosite.dat";
        assert_eq!(infer_level(LogStream::Stderr, line), LogLevel::Warn);
    }

    #[test]
    fn stderr_without_markers_is_info() {
        assert_eq!(
            infer_level(
                LogStream::Stderr,
                "inbound mixed[mixed-in] started at 127.0.0.1:7890"
            ),
            LogLevel::Info
        );
    }

    #[test]
    fn ansi_sequences_are_stripped() {
        let raw = "\u{1b}[36mINFO\u{1b}[0m[0000] inbound connection from 127.0.0.1:60270\r";
        assert_eq!(
            strip_ansi(raw),
            "INFO[0000] inbound connection from 127.0.0.1:60270"
        );
        // A stripped line must still classify correctly.
        assert_eq!(
            infer_level(LogStream::Stderr, &strip_ansi(raw)),
            LogLevel::Info
        );
    }

    #[test]
    fn strip_ansi_leaves_plain_text_alone() {
        assert_eq!(strip_ansi("plain line"), "plain line");
        assert_eq!(
            strip_ansi("256-color \u{1b}[38;5;189mtext\u{1b}[0m"),
            "256-color text"
        );
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

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

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, OnceCell};

use crate::core::CoreKind;
use crate::error::{Error, Result};
use crate::health::{port_is_free, probe_tcp, CoreHealth};
use crate::logging::{now_ms, LogBuffer, LogStream};
use crate::state::CoreState;

/// How long a core is given to bind its inbound before it is considered failed.
pub const DEFAULT_READY_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a core is given to exit after a termination signal.
pub const DEFAULT_STOP_TIMEOUT: Duration = Duration::from_secs(5);
/// Rotate a per-core log file once it grows past this size.
pub const MAX_LOG_FILE_BYTES: u64 = 8 * 1024 * 1024;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Stop a long-lived core from inheriting the caller's stdio handles.
///
/// On Windows the child is created with handle inheritance enabled, and the
/// stdio handles TriProxy itself inherited (a shell pipe, an IDE terminal, a GUI
/// log pane) are inheritable. The core would then hold those handles open for its
/// entire lifetime, so any pipeline such as `triproxy start | tee log` never sees
/// EOF — the command appears to hang even though TriProxy already exited.
///
/// The pipes TriProxy creates for the core's own stdout/stderr are unaffected:
/// they are created as inheritable specifically for that child.
#[cfg(windows)]
fn detach_inherited_std_handles() {
    use std::ffi::c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(which: u32) -> *mut c_void;
        fn SetHandleInformation(handle: *mut c_void, mask: u32, flags: u32) -> i32;
    }

    const STD_INPUT_HANDLE: u32 = -10i32 as u32;
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    const HANDLE_FLAG_INHERIT: u32 = 0x0000_0001;
    const INVALID_HANDLE_VALUE: isize = -1;

    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        unsafe {
            let handle = GetStdHandle(which);
            if handle.is_null() || handle as isize == INVALID_HANDLE_VALUE {
                continue;
            }
            SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
        }
    }
}

#[cfg(not(windows))]
fn detach_inherited_std_handles() {}

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    /// Combined output, trimmed, for error messages.
    pub fn combined(&self) -> String {
        let mut text = String::new();
        let stderr = self.stderr.trim();
        let stdout = self.stdout.trim();
        if !stderr.is_empty() {
            text.push_str(stderr);
        }
        if !stdout.is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(stdout);
        }
        text
    }
}

/// Run a short-lived helper command (config check, `--version`) and capture output.
pub async fn run_capture(binary: &Path, args: &[OsString]) -> Result<CommandOutput> {
    let mut cmd = Command::new(binary);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.as_std_mut().creation_flags(CREATE_NO_WINDOW);
    }

    detach_inherited_std_handles();

    let output = cmd.output().await.map_err(|e| Error::io(binary, e))?;
    Ok(CommandOutput {
        success: output.status.success(),
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

struct Inner {
    state: CoreState,
    child: Option<tokio::process::Child>,
    pid: Option<u32>,
    config: Option<PathBuf>,
    started_at: Option<Instant>,
    listen_port: Option<u16>,
    last_error: Option<String>,
    /// Last TCP probe result and when it was taken.
    probe: Option<(Instant, Option<u64>)>,
}

/// How long a TCP probe result is reused before probing again.
///
/// The UI polls status every two seconds, and each probe is a real TCP
/// connection to the local inbound — which the core dutifully logs at INFO, one
/// line per poll. Process liveness is still checked on every call, so crash
/// detection stays immediate; only the "is the port accepting" answer is cached,
/// and it rarely changes.
const PROBE_TTL: Duration = Duration::from_secs(10);

/// Owns the OS process for exactly one core.
///
/// Design note: the supervisor is taken by `&self` everywhere and guards its
/// mutable state with an internal async mutex. This lets `CoreManager` hold
/// `Arc<dyn ProxyCore>` without nesting an outer lock around it — the failure
/// mode the plan warns about in ch.9.
pub struct Supervisor {
    kind: CoreKind,
    binary: PathBuf,
    logs: Arc<LogBuffer>,
    log_file: Option<PathBuf>,
    inner: Mutex<Inner>,
    version: OnceCell<String>,
}

impl Supervisor {
    pub fn new(
        kind: CoreKind,
        binary: PathBuf,
        logs: Arc<LogBuffer>,
        log_file: Option<PathBuf>,
    ) -> Self {
        Self {
            kind,
            binary,
            logs,
            log_file,
            inner: Mutex::new(Inner {
                state: CoreState::Stopped,
                child: None,
                pid: None,
                config: None,
                started_at: None,
                listen_port: None,
                last_error: None,
                probe: None,
            }),
            version: OnceCell::new(),
        }
    }

    pub fn kind(&self) -> CoreKind {
        self.kind
    }

    pub fn binary_path(&self) -> &Path {
        &self.binary
    }

    pub fn logs(&self) -> &Arc<LogBuffer> {
        &self.logs
    }

    pub fn cached_version(&self) -> Option<&String> {
        self.version.get()
    }

    pub fn set_cached_version(&self, version: String) {
        let _ = self.version.set(version);
    }

    pub async fn state(&self) -> CoreState {
        self.inner.lock().await.state
    }

    pub async fn pid(&self) -> Option<u32> {
        self.inner.lock().await.pid
    }

    pub async fn config_path(&self) -> Option<PathBuf> {
        self.inner.lock().await.config.clone()
    }

    /// Take over a core that is already running from a previous process.
    ///
    /// A CLI invocation is short-lived while the core it spawned is not, so the
    /// next invocation has no child handle. Adoption rebuilds the bookkeeping
    /// from persisted state and switches liveness checks to a PID probe.
    /// Returns `false` when the recorded PID is already gone.
    pub async fn adopt(&self, pid: u32, port: u16, config: Option<PathBuf>) -> bool {
        if !pid_alive(pid) {
            return false;
        }
        let mut guard = self.inner.lock().await;
        guard.state = CoreState::Running;
        guard.pid = Some(pid);
        guard.child = None;
        guard.listen_port = Some(port);
        guard.probe = None;
        guard.config = config;
        guard.started_at = None;
        guard.last_error = None;
        self.logs.push(
            self.kind.as_str(),
            LogStream::App,
            format!("adopted running process (pid {pid})"),
        );
        true
    }

    async fn recent_error_text(&self) -> String {
        let lines = self.logs.tail_for(self.kind.as_str(), 12);
        lines
            .iter()
            .map(|l| l.text.clone())
            .collect::<Vec<_>>()
            .join(" | ")
    }

    /// Spawn the core process and wait until it is actually serving.
    ///
    /// Readiness is defined as "the local inbound port accepts a TCP
    /// connection". A process that exits before that is reported as a startup
    /// failure with the tail of its output attached.
    pub async fn spawn(
        &self,
        config: &Path,
        args: Vec<OsString>,
        listen_port: u16,
        ready_timeout: Duration,
    ) -> Result<()> {
        let mut guard = self.inner.lock().await;

        if !guard.state.can_start() {
            return Err(Error::InvalidTransition {
                core: self.kind.as_str().to_string(),
                state: guard.state.to_string(),
                expected: "stopped or failed".to_string(),
            });
        }

        // Guard against binding conflicts before paying for process startup.
        if !wait_port_free(listen_port, Duration::from_secs(3)).await {
            guard.state = CoreState::Failed;
            guard.last_error = Some(format!("port {listen_port} already in use"));
            return Err(Error::PortInUse {
                core: self.kind.as_str().to_string(),
                port: listen_port,
            });
        }

        guard.state = CoreState::Starting;
        guard.last_error = None;
        guard.listen_port = Some(listen_port);

        self.logs.push(
            self.kind.as_str(),
            LogStream::App,
            format!("starting: {} {}", self.binary.display(), render_args(&args)),
        );

        let mut cmd = Command::new(&self.binary);
        cmd.args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(false);
        if let Some(dir) = config.parent() {
            cmd.current_dir(dir);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.as_std_mut().creation_flags(CREATE_NO_WINDOW);
        }

        detach_inherited_std_handles();

        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                guard.state = CoreState::Failed;
                let reason = format!("spawn failed: {e}");
                guard.last_error = Some(reason.clone());
                return Err(Error::StartupFailed {
                    core: self.kind.as_str().to_string(),
                    reason,
                });
            }
        };

        let pid = child.id();
        if let Some(stdout) = child.stdout.take() {
            spawn_line_reader(
                stdout,
                self.logs.clone(),
                self.kind,
                LogStream::Stdout,
                self.log_file.clone(),
            );
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_line_reader(
                stderr,
                self.logs.clone(),
                self.kind,
                LogStream::Stderr,
                self.log_file.clone(),
            );
        }

        guard.child = Some(child);
        guard.pid = pid;
        guard.config = Some(config.to_path_buf());
        guard.started_at = Some(Instant::now());

        // Readiness loop.
        let deadline = Instant::now() + ready_timeout;
        loop {
            let exited = match guard.child.as_mut() {
                Some(child) => child.try_wait().ok().flatten(),
                None => None,
            };
            if let Some(status) = exited {
                let tail = self.recent_error_text().await;
                guard.child = None;
                guard.pid = None;
                guard.state = CoreState::Failed;
                let reason = format!("exited during startup ({status}): {tail}");
                guard.last_error = Some(reason.clone());
                return Err(Error::StartupFailed {
                    core: self.kind.as_str().to_string(),
                    reason,
                });
            }

            if probe_tcp(listen_port).await.is_some() {
                guard.state = CoreState::Running;
                self.logs.push(
                    self.kind.as_str(),
                    LogStream::App,
                    format!("running: listening on 127.0.0.1:{listen_port}"),
                );
                return Ok(());
            }

            if Instant::now() >= deadline {
                let tail = self.recent_error_text().await;
                self.kill_locked(&mut guard).await;
                guard.state = CoreState::Failed;
                let reason = format!(
                    "did not bind 127.0.0.1:{listen_port} within {}s: {tail}",
                    ready_timeout.as_secs()
                );
                guard.last_error = Some(reason.clone());
                return Err(Error::StartupFailed {
                    core: self.kind.as_str().to_string(),
                    reason,
                });
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub async fn stop(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        if guard.state == CoreState::Stopped && guard.child.is_none() {
            return Ok(());
        }

        guard.state = CoreState::Stopping;
        self.kill_locked(&mut guard).await;
        guard.state = CoreState::Stopped;
        guard.started_at = None;
        guard.listen_port = None;
        guard.probe = None;
        self.logs
            .push(self.kind.as_str(), LogStream::App, "stopped");
        Ok(())
    }

    async fn kill_locked(&self, guard: &mut Inner) {
        let Some(mut child) = guard.child.take() else {
            // Adopted process: no handle, so termination goes through the PID.
            if let Some(pid) = guard.pid {
                let _ = force_kill(pid);
                wait_pid_gone(pid, DEFAULT_STOP_TIMEOUT).await;
                guard.pid = None;
                guard.config = None;
            }
            return;
        };
        let pid = guard.pid;

        // Ask nicely first: all three cores exit cleanly on a termination signal
        // and flush their logs. Windows has no equivalent signal, so the handle
        // kill is the only path there and the force fallback below covers the rest.
        #[cfg(unix)]
        if let Some(pid) = pid {
            let _ = std::process::Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        #[cfg(windows)]
        {
            let _ = child.start_kill();
        }

        match tokio::time::timeout(DEFAULT_STOP_TIMEOUT, child.wait()).await {
            Ok(_) => {}
            Err(_) => {
                self.logs.push(
                    self.kind.as_str(),
                    LogStream::App,
                    "graceful stop timed out, forcing termination",
                );
                if let Some(pid) = pid {
                    let _ = force_kill(pid);
                }
                let _ = child.start_kill();
                let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
            }
        }

        guard.pid = None;
        guard.child = None;
        guard.config = None;
    }

    /// Reconcile bookkeeping with reality: detect a core that died on its own.
    pub async fn refresh(&self) -> CoreState {
        let mut guard = self.inner.lock().await;

        // Adopted processes have no child handle, so liveness is a PID probe.
        if guard.child.is_none() {
            if let Some(pid) = guard.pid {
                if !pid_alive(pid) {
                    guard.pid = None;
                    guard.started_at = None;
                    guard.state = CoreState::Failed;
                    let reason = format!("process {pid} is no longer alive");
                    guard.last_error = Some(reason.clone());
                    self.logs.push(self.kind.as_str(), LogStream::App, reason);
                }
            }
            return guard.state;
        }

        let exit = match guard.child.as_mut() {
            Some(child) => child.try_wait(),
            None => return guard.state,
        };
        if let Ok(Some(status)) = exit {
            let previous = guard.state;
            guard.child = None;
            guard.pid = None;
            guard.started_at = None;
            guard.state = CoreState::Failed;
            let reason = format!("process exited unexpectedly ({status})");
            guard.last_error = Some(reason.clone());
            self.logs.push(self.kind.as_str(), LogStream::App, reason);
            if previous.is_running() {
                tracing::warn!(core = self.kind.as_str(), "core exited unexpectedly");
            }
        }
        guard.state
    }

    pub async fn health(&self) -> CoreHealth {
        self.refresh().await;

        let (state, pid, uptime_secs, port, last_error) = {
            let guard = self.inner.lock().await;
            (
                guard.state,
                guard.pid,
                guard.started_at.map(|t| t.elapsed().as_secs()),
                guard.listen_port,
                guard.last_error.clone(),
            )
        };

        let latency = match (state.is_running(), port) {
            (true, Some(port)) => {
                let cached = self
                    .inner
                    .lock()
                    .await
                    .probe
                    .filter(|(at, _)| at.elapsed() < PROBE_TTL)
                    .map(|(_, value)| value);
                match cached {
                    Some(value) => value,
                    None => {
                        let value = probe_tcp(port).await;
                        self.inner.lock().await.probe = Some((Instant::now(), value));
                        value
                    }
                }
            }
            _ => None,
        };

        CoreHealth {
            state,
            pid,
            uptime_secs,
            listen_port: port,
            listening: if state.is_running() {
                Some(latency.is_some())
            } else {
                None
            },
            latency_ms: latency,
            last_error,
        }
    }
}

/// Forward one pipe of core output into the ring buffer and, when configured,
/// into the per-core log file.
///
/// File output is what makes `triproxy logs` meaningful: the ring buffer dies
/// with the CLI process, the file does not.
fn spawn_line_reader<R>(
    reader: R,
    logs: Arc<LogBuffer>,
    kind: CoreKind,
    stream: LogStream,
    log_file: Option<PathBuf>,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        use std::io::Write;

        let mut file = log_file.and_then(|path| {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            // Keep a single generation so a long-running core cannot fill the disk.
            if let Ok(meta) = std::fs::metadata(&path) {
                if meta.len() > MAX_LOG_FILE_BYTES {
                    let _ = std::fs::rename(&path, path.with_extension("log.1"));
                }
            }
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .ok()
        });

        let mut lines = BufReader::new(reader).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    // Colour codes are forced by the cores even on a pipe; they
                    // would otherwise show up as `[36m` noise in the panel and
                    // break level detection.
                    let line = crate::logging::strip_ansi(&line);
                    let level = crate::logging::infer_level(stream, &line);
                    if let Some(file) = file.as_mut() {
                        let _ = writeln!(file, "{}\t{}\t{}", now_ms(), level.as_char(), line);
                        let _ = file.flush();
                    }
                    logs.push(kind.as_str(), stream, line);
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }
    });
}

fn render_args(args: &[OsString]) -> String {
    args.iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

async fn wait_port_free(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if port_is_free(port) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_pid_gone(pid: u32, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !pid_alive(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Force-terminate a process, including its children.
pub fn force_kill(pid: u32) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        let status = hidden_command("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        let _ = status;
        Ok(())
    }
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        Ok(())
    }
}

/// A `Command` that cannot flash a console window on Windows.
///
/// The GUI is built as a windows-subsystem binary and owns no console, so every
/// console program it launches gets a fresh window of its own. `taskkill` on
/// stop and `netsh` on TUN cleanup each produced a black box on screen for a few
/// hundred milliseconds; users read that as the app doing something suspicious.
pub(crate) fn hidden_command(program: &str) -> std::process::Command {
    #[allow(unused_mut)]
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Is the given PID still alive?
///
/// Windows answers this through the kernel instead of running `tasklist`. That
/// is not a micro-optimisation: the status poll asks once every two seconds, and
/// with a core adopted from a previous session (the normal state right after an
/// in-app update) every single poll spawned a console program — a console window
/// appearing twice a second, forever.
#[cfg(windows)]
pub fn pid_alive(pid: u32) -> bool {
    use std::ffi::c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn GetExitCodeProcess(handle: *mut c_void, code: *mut u32) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }

    // LIMITED_INFORMATION rather than QUERY_INFORMATION: it also opens processes
    // at a higher integrity level, which is what an elevated core is when the
    // shell that adopted it is not.
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        ok != 0 && code == STILL_ACTIVE
    }
}

#[cfg(unix)]
pub fn pid_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
        || std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_are_rendered_for_logging() {
        let args = vec![OsString::from("-f"), OsString::from("config.yaml")];
        assert_eq!(render_args(&args), "-f config.yaml");
    }

    /// The liveness probe runs on every status poll, so it has to be right in
    /// both directions — a false positive leaves a dead core looking alive, a
    /// false negative adopts nothing and kills a healthy one.
    #[test]
    fn pid_alive_tells_self_from_an_unused_pid() {
        assert!(pid_alive(std::process::id()));
        // Above the Windows PID ceiling and unused on unix.
        assert!(!pid_alive(0x7FFF_FFF0));
    }

    #[tokio::test]
    async fn spawn_of_missing_binary_fails_cleanly() {
        let logs = Arc::new(LogBuffer::default());
        let supervisor = Supervisor::new(
            CoreKind::Mihomo,
            PathBuf::from("definitely-not-a-real-binary-xyz"),
            logs,
            None,
        );
        let err = supervisor
            .spawn(
                Path::new("config.yaml"),
                vec![],
                7899,
                Duration::from_millis(300),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, Error::StartupFailed { .. }));
        assert_eq!(supervisor.state().await, CoreState::Failed);
    }

    #[tokio::test]
    async fn stop_is_idempotent_on_a_stopped_core() {
        let logs = Arc::new(LogBuffer::default());
        let supervisor = Supervisor::new(CoreKind::Xray, PathBuf::from("xray"), logs, None);
        supervisor.stop().await.unwrap();
        supervisor.stop().await.unwrap();
        assert_eq!(supervisor.state().await, CoreState::Stopped);
    }
}

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

const MAX_BUFFER_BYTES: usize = 256 * 1024;
const READ_CHUNK_BYTES: usize = 4096;
const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

/// Maximum number of live PTY sessions. One per channel in practice,
/// but this cap prevents unbounded growth.
const MAX_LIVE_SESSIONS: usize = 20;

/// Env vars inherited from the Tauri app's Hermit activation that should
/// be stripped from spawned shells so the user gets a clean environment.
const HERMIT_METADATA_ENV_KEYS: &[&str] = &[
    "ACTIVE_HERMIT",
    "HERMIT_BIN",
    "HERMIT_BIN_CHANGE",
    "HERMIT_ENV",
    "HERMIT_ENV_OPS",
];

const MACOS_APP_ENV_KEYS: &[&str] = &["__CFBundleIdentifier", "XPC_SERVICE_NAME"];

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalDataPayload {
    session_id: String,
    data: String,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalExitPayload {
    session_id: String,
    exit_code: i32,
}

pub struct TerminalRegistry {
    sessions: Arc<Mutex<TerminalSessions>>,
}

#[derive(Default)]
struct TerminalSessions {
    by_session_id: HashMap<String, TerminalSession>,
    /// Maps channel_id → session_id for single-instance-per-channel semantics.
    session_id_by_channel: HashMap<String, String>,
}

struct TerminalSession {
    session_id: String,
    channel_id: String,
    #[allow(dead_code)]
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    buffer: Arc<Mutex<Vec<u8>>>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOpenInput {
    pub channel_id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOpenOutput {
    pub session_id: String,
    pub created: bool,
    pub initial_data: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalWriteInput {
    pub session_id: String,
    pub data: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalResizeInput {
    pub session_id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalCloseInput {
    pub session_id: String,
}

impl TerminalRegistry {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(TerminalSessions::default())),
        }
    }

    pub fn open_session(
        &self,
        app: &AppHandle,
        input: &TerminalOpenInput,
    ) -> Result<TerminalOpenOutput, String> {
        let size = normalize_size(input.cols, input.rows);
        let mut guard = self.sessions.lock().map_err(|e| e.to_string())?;

        // Return existing session for this channel if one exists.
        if let Some(existing_id) = guard.session_id_by_channel.get(&input.channel_id).cloned() {
            if let Some(session) = guard.by_session_id.get(&existing_id) {
                // Resize the existing session to match the new dimensions.
                session
                    .master
                    .resize(size)
                    .map_err(|e| format!("resize failed: {e}"))?;
                let initial_data = snapshot_buffer(&session.buffer)?;
                return Ok(TerminalOpenOutput {
                    session_id: existing_id,
                    created: false,
                    initial_data,
                });
            }
            // Stale mapping — remove it.
            guard.session_id_by_channel.remove(&input.channel_id);
        }

        // Enforce session cap via LRU eviction (by insertion order here — simple).
        let mut evicted: Vec<TerminalSession> = Vec::new();
        while guard.by_session_id.len() >= MAX_LIVE_SESSIONS {
            // Evict the first session we find that isn't for this channel.
            let evict_id = guard
                .by_session_id
                .keys()
                .find(|id| {
                    guard
                        .by_session_id
                        .get(*id)
                        .map(|s| s.channel_id != input.channel_id)
                        .unwrap_or(true)
                })
                .cloned();
            let Some(evict_id) = evict_id else {
                break;
            };
            if let Some(session) = guard.by_session_id.remove(&evict_id) {
                guard.session_id_by_channel.remove(&session.channel_id);
                evicted.push(session);
            }
        }
        drop(guard);

        // Kill evicted sessions outside the lock.
        for mut session in evicted {
            let _ = session.killer.kill();
        }

        // Spawn a new PTY session.
        let session = spawn_session(app.clone(), self.sessions.clone(), &input.channel_id, size)?;
        let session_id = session.session_id.clone();

        let mut guard = self.sessions.lock().map_err(|e| e.to_string())?;
        guard
            .session_id_by_channel
            .insert(input.channel_id.clone(), session_id.clone());
        guard.by_session_id.insert(session_id.clone(), session);

        Ok(TerminalOpenOutput {
            session_id,
            created: true,
            initial_data: None,
        })
    }

    pub fn write(&self, session_id: &str, data: &[u8]) -> Result<(), String> {
        let mut guard = self.sessions.lock().map_err(|e| e.to_string())?;
        let session = guard
            .by_session_id
            .get_mut(session_id)
            .ok_or_else(|| format!("session not found: {session_id}"))?;
        session.writer.write_all(data).map_err(|e| e.to_string())?;
        session.writer.flush().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn resize(&self, session_id: &str, cols: u16, rows: u16) -> Result<(), String> {
        let guard = self.sessions.lock().map_err(|e| e.to_string())?;
        let session = guard
            .by_session_id
            .get(session_id)
            .ok_or_else(|| format!("session not found: {session_id}"))?;
        session
            .master
            .resize(normalize_size(cols, rows))
            .map_err(|e| format!("resize failed: {e}"))?;
        Ok(())
    }

    pub fn close_session(&self, session_id: &str) -> Result<bool, String> {
        let mut guard = self.sessions.lock().map_err(|e| e.to_string())?;
        let Some(mut session) = guard.by_session_id.remove(session_id) else {
            return Ok(false);
        };
        guard.session_id_by_channel.remove(&session.channel_id);
        drop(guard);
        let _ = session.killer.kill();
        Ok(true)
    }

    /// Close all active PTY sessions. Called on workspace switch to prevent
    /// orphaned shell processes from accumulating.
    pub fn close_all_sessions(&self) -> Result<u32, String> {
        let mut guard = self.sessions.lock().map_err(|e| e.to_string())?;
        let count = guard.by_session_id.len() as u32;
        guard.session_id_by_channel.clear();
        let sessions = std::mem::take(&mut guard.by_session_id);
        drop(guard);

        for mut session in sessions.into_values() {
            let _ = session.killer.kill();
        }
        Ok(count)
    }
}

impl Drop for TerminalRegistry {
    fn drop(&mut self) {
        let sessions = match self.sessions.lock() {
            Ok(mut guard) => {
                guard.session_id_by_channel.clear();
                std::mem::take(&mut guard.by_session_id)
            }
            Err(_) => return,
        };

        for mut session in sessions.into_values() {
            let _ = session.killer.kill();
        }
    }
}

fn normalize_size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        cols: if cols == 0 { DEFAULT_COLS } else { cols },
        rows: if rows == 0 { DEFAULT_ROWS } else { rows },
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn resolve_shell() -> String {
    if cfg!(windows) {
        return "powershell.exe".to_string();
    }

    if let Ok(shell) = std::env::var("SHELL") {
        let trimmed = shell.trim().to_string();
        if !trimmed.is_empty() && std::path::Path::new(&trimmed).exists() {
            return trimmed;
        }
    }

    for candidate in ["/bin/zsh", "/bin/bash", "/bin/sh"] {
        if std::path::Path::new(candidate).exists() {
            return candidate.to_string();
        }
    }

    "/bin/sh".to_string()
}

fn spawn_session(
    app: AppHandle,
    sessions: Arc<Mutex<TerminalSessions>>,
    channel_id: &str,
    size: PtySize,
) -> Result<TerminalSession, String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(size)
        .map_err(|e| format!("failed to open pty: {e}"))?;

    let shell = resolve_shell();
    let mut cmd = CommandBuilder::new(&shell);

    // Set working directory to user's home.
    if let Some(home) = dirs::home_dir() {
        cmd.cwd(home);
    }

    // Set terminal identification.
    cmd.env("TERM", "xterm-256color");
    cmd.env("TERM_PROGRAM", "Sprout");

    // Scrub Hermit env vars so the user gets a clean shell.
    for key in HERMIT_METADATA_ENV_KEYS {
        cmd.env_remove(key);
    }
    for key in MACOS_APP_ENV_KEYS {
        cmd.env_remove(key);
    }

    // Strip Hermit PATH entries.
    scrub_hermit_path(&mut cmd);

    // Add interactive flag.
    if !cfg!(windows) {
        if shell.contains("zsh") {
            cmd.args(["-il"]);
        } else {
            cmd.arg("-i");
        }
    }

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("failed to spawn shell: {e}"))?;
    drop(pair.slave);

    let session_id = Uuid::new_v4().to_string();
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("failed to clone reader: {e}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("failed to take writer: {e}"))?;
    let killer = child.clone_killer();
    let buffer = Arc::new(Mutex::new(Vec::new()));

    spawn_reader_thread(app.clone(), session_id.clone(), reader, buffer.clone());
    spawn_wait_thread(app, sessions, session_id.clone());

    Ok(TerminalSession {
        session_id,
        channel_id: channel_id.to_string(),
        master: pair.master,
        writer,
        killer,
        buffer,
    })
}

fn spawn_reader_thread(
    app: AppHandle,
    session_id: String,
    mut reader: Box<dyn Read + Send>,
    buffer: Arc<Mutex<Vec<u8>>>,
) {
    thread::spawn(move || {
        let mut chunk = [0u8; READ_CHUNK_BYTES];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => {
                    // Append to buffer.
                    if let Ok(mut guard) = buffer.lock() {
                        guard.extend_from_slice(&chunk[..count]);
                        if guard.len() > MAX_BUFFER_BYTES {
                            let extra = guard.len() - MAX_BUFFER_BYTES;
                            guard.drain(0..extra);
                        }
                    }

                    let payload = TerminalDataPayload {
                        session_id: session_id.clone(),
                        data: String::from_utf8_lossy(&chunk[..count]).into_owned(),
                    };
                    if let Err(e) = app.emit("terminal:data", payload) {
                        eprintln!("[terminal] emit terminal:data failed: {e}");
                    }
                }
                Err(_) => break,
            }
        }
    });
}

fn spawn_wait_thread(app: AppHandle, sessions: Arc<Mutex<TerminalSessions>>, session_id: String) {
    thread::spawn(move || {
        // Wait for the child process to exit by polling the session's existence.
        // The reader thread will break when the PTY closes, which happens when
        // the child exits. We detect this by the reader thread ending (the PTY
        // master read returns 0). For simplicity, we poll.
        loop {
            std::thread::sleep(std::time::Duration::from_millis(200));
            let guard = match sessions.lock() {
                Ok(g) => g,
                Err(_) => break,
            };
            if !guard.by_session_id.contains_key(&session_id) {
                break;
            }
            drop(guard);
        }

        let payload = TerminalExitPayload {
            session_id: session_id.clone(),
            exit_code: 0,
        };
        let _ = app.emit("terminal:exit", payload);

        // Clean up from registry.
        if let Ok(mut guard) = sessions.lock() {
            if let Some(session) = guard.by_session_id.remove(&session_id) {
                guard.session_id_by_channel.remove(&session.channel_id);
            }
        }
    });
}

fn snapshot_buffer(buffer: &Arc<Mutex<Vec<u8>>>) -> Result<Option<String>, String> {
    let guard = buffer.lock().map_err(|e| e.to_string())?;
    if guard.is_empty() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&guard).into_owned()))
}

fn scrub_hermit_path(cmd: &mut CommandBuilder) {
    let hermit_env = cmd.get_env("HERMIT_ENV").map(|v| v.to_os_string());
    let active_hermit = cmd.get_env("ACTIVE_HERMIT").map(|v| v.to_os_string());

    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    if let Some(val) = hermit_env {
        let p = std::path::PathBuf::from(val);
        if !p.as_os_str().is_empty() && !roots.contains(&p) {
            roots.push(p);
        }
    }
    if let Some(val) = active_hermit {
        let p = std::path::PathBuf::from(val);
        if !p.as_os_str().is_empty() && !roots.contains(&p) {
            roots.push(p);
        }
    }

    if roots.is_empty() {
        return;
    }

    let Some(path_val) = cmd.get_env("PATH").map(|v| v.to_os_string()) else {
        return;
    };

    let filtered: Vec<std::path::PathBuf> = std::env::split_paths(&path_val)
        .filter(|entry| !roots.iter().any(|root| entry.starts_with(root)))
        .collect();

    if let Ok(joined) = std::env::join_paths(filtered) {
        cmd.env("PATH", joined);
    }
}

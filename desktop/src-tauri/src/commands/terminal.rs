use tauri::{AppHandle, State};

use crate::app_state::AppState;
use crate::terminal::{
    TerminalCloseInput, TerminalOpenInput, TerminalResizeInput, TerminalWriteInput,
};

#[tauri::command]
pub fn terminal_open_session(
    app: AppHandle,
    state: State<'_, AppState>,
    input: TerminalOpenInput,
) -> Result<crate::terminal::TerminalOpenOutput, String> {
    state.terminal_registry.open_session(&app, &input)
}

#[tauri::command]
pub fn terminal_write(state: State<'_, AppState>, input: TerminalWriteInput) -> Result<(), String> {
    state
        .terminal_registry
        .write(&input.session_id, input.data.as_bytes())
}

#[tauri::command]
pub fn terminal_resize(
    state: State<'_, AppState>,
    input: TerminalResizeInput,
) -> Result<(), String> {
    state
        .terminal_registry
        .resize(&input.session_id, input.cols, input.rows)
}

#[tauri::command]
pub fn terminal_close_session(
    state: State<'_, AppState>,
    input: TerminalCloseInput,
) -> Result<bool, String> {
    state.terminal_registry.close_session(&input.session_id)
}

/// Close all active PTY sessions. Called on workspace switch.
#[tauri::command]
pub fn terminal_close_all_sessions(state: State<'_, AppState>) -> Result<u32, String> {
    state.terminal_registry.close_all_sessions()
}

/// Check if a PTY session exists for a channel.
#[tauri::command]
pub fn terminal_has_session(
    state: State<'_, AppState>,
    channel_id: String,
) -> Result<bool, String> {
    state.terminal_registry.has_session(&channel_id)
}

/// Get the last line of output from a channel's terminal.
#[tauri::command]
pub fn terminal_get_last_line(
    state: State<'_, AppState>,
    channel_id: String,
) -> Result<Option<String>, String> {
    state.terminal_registry.get_last_line(&channel_id)
}

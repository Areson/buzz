//! Agent templates and agent-record export.
//!
//! Templates are static starter data for the Create Agent wizard — selecting
//! one prefills the create form; no persona record is involved. Export maps a
//! managed agent's pinned config onto the shareable `.persona.json` card
//! interchange format.

use tauri::{AppHandle, State};

use crate::{
    app_state::AppState,
    managed_agents::{load_managed_agents, load_personas},
};

/// Built-in agent templates for the Create Agent wizard. Static data — no
/// store access, no lock.
#[tauri::command]
pub fn list_agent_templates() -> Vec<crate::managed_agents::AgentTemplate> {
    crate::managed_agents::builtin_agent_templates()
}

/// Export a managed agent's pinned config as a shareable `.persona.json`
/// card (the interchange format). `env_vars` are deliberately excluded —
/// cards are shareable artifacts and must never carry credentials.
#[tauri::command]
pub async fn export_agent_to_json(
    pubkey: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    // Load the record under lock, then drop the lock before the dialog.
    let (name, system_prompt, avatar_url, runtime, model, provider) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let records = load_managed_agents(&app)?;
        let record = records
            .iter()
            .find(|r| r.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;
        let personas = load_personas(&app).unwrap_or_default();
        let effective_command = crate::managed_agents::effective_agent_command(
            record.persona_id.as_deref(),
            &personas,
            record.agent_command_override.as_deref(),
        );
        let runtime =
            crate::managed_agents::known_acp_runtime(&effective_command).map(|r| r.id.to_string());
        (
            record.name.clone(),
            record.system_prompt.clone().unwrap_or_default(),
            record.avatar_url.clone(),
            runtime,
            record.model.clone(),
            record.provider.clone(),
        )
    };

    let json_bytes = crate::managed_agents::encode_persona_json(
        &name,
        &system_prompt,
        avatar_url.as_deref(),
        runtime.as_deref(),
        model.as_deref(),
        provider.as_deref(),
        &[],
    )?;

    let slug = crate::util::slugify(&name, "agent", 50);
    let filename = format!("{slug}.persona.json");
    super::export_util::save_json_with_dialog(&app, &filename, &json_bytes).await
}

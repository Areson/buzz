//! Agent templates and agent-record export.
//!
//! Templates are starter data for the Create Agent wizard — selecting one
//! prefills the create form; the submit creates a plain managed agent.
//! Built-ins are static; saved templates are persona records (relay-synced).
//! Export maps a managed agent's pinned config onto the shareable
//! `.persona.json` card interchange format.

use tauri::{AppHandle, Manager, State};

use crate::{
    app_state::AppState,
    managed_agents::{
        agent_template_from_persona, builtin_agent_templates, load_managed_agents, load_personas,
        save_personas, try_regenerate_nest, AgentTemplate, PersonaRecord,
    },
    util::now_iso,
};

/// Templates for the Create Agent wizard: static built-ins followed by saved
/// templates (active persona records), sorted by display name. A saved record
/// whose id shadows a built-in id (a demoted legacy built-in copy) is skipped
/// so the catalog never shows the same starter twice.
#[tauri::command]
pub async fn list_agent_templates(app: AppHandle) -> Result<Vec<AgentTemplate>, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        Ok(assemble_agent_templates(&load_personas(&app)?))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Pure core of [`list_agent_templates`]: built-ins first, then the saved
/// templates derived from `personas` — active records only, records whose id
/// shadows a built-in id skipped, sorted by lowercased display name (id as
/// tiebreaker).
fn assemble_agent_templates(personas: &[PersonaRecord]) -> Vec<AgentTemplate> {
    let mut templates = builtin_agent_templates();
    let mut saved: Vec<AgentTemplate> = personas
        .iter()
        .filter(|persona| persona.is_active)
        .filter(|persona| !templates.iter().any(|builtin| builtin.id == persona.id))
        .map(agent_template_from_persona)
        .collect();
    saved.sort_by(|a, b| {
        a.display_name
            .to_lowercase()
            .cmp(&b.display_name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    templates.extend(saved);
    templates
}

/// Save a managed agent's pinned config as a reusable template (a persona
/// record) so it shows up in the New Agent catalog. Saving errors when an
/// active in-app template (or a built-in starter) already uses the same
/// display name — silently updating in place could clobber a template the
/// user didn't mean to touch. Delete the existing template or rename the
/// agent first; a deliberate refresh affordance can come later.
/// `env_vars` are deliberately excluded: templates are shareable definitions
/// and must never carry credentials. The record is retained for relay sync
/// (kind:30175), so the template reaches the owner's other devices.
#[tauri::command]
pub async fn save_agent_as_template(
    pubkey: String,
    app: AppHandle,
) -> Result<AgentTemplate, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let records = load_managed_agents(&app)?;
        let record = records
            .iter()
            .find(|r| r.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;

        let name = record.name.trim().to_string();
        if name.is_empty() {
            return Err("agent has no name to save as a template".to_string());
        }

        let mut personas = load_personas(&app)?;
        let effective_command = crate::managed_agents::effective_agent_command(
            record.persona_id.as_deref(),
            &personas,
            record.agent_command_override.as_deref(),
        );
        let runtime =
            crate::managed_agents::known_acp_runtime(&effective_command).map(|r| r.id.to_string());

        let persona = insert_template_persona(
            &mut personas,
            name,
            record.avatar_url.clone(),
            record.system_prompt.clone().unwrap_or_default(),
            runtime,
            record.model.clone(),
            record.provider.clone(),
            now_iso(),
        )?;
        save_personas(&app, &personas)?;
        super::personas::retain_persona_pending(&app, &state, &persona);
        try_regenerate_nest(&app);
        Ok(agent_template_from_persona(&persona))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Pure core of [`save_agent_as_template`]: push a fresh record, or error
/// when the trimmed display name (case-insensitive) collides with an active
/// in-app template or a built-in starter — never update an existing template
/// in place, so a save can't silently clobber one. Inactive and team-sourced
/// records (`source_team.is_some()`) don't block: they aren't in the catalog
/// as saved in-app templates, and a fresh record must not hijack them either.
/// `env_vars` stay empty: templates are shareable definitions and must never
/// carry credentials.
#[allow(clippy::too_many_arguments)]
fn insert_template_persona(
    personas: &mut Vec<PersonaRecord>,
    name: String,
    avatar_url: Option<String>,
    system_prompt: String,
    runtime: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    now: String,
) -> Result<PersonaRecord, String> {
    let collides = personas.iter().any(|p| {
        p.is_active && p.source_team.is_none() && p.display_name.trim().eq_ignore_ascii_case(&name)
    }) || builtin_agent_templates()
        .iter()
        .any(|builtin| builtin.display_name.trim().eq_ignore_ascii_case(&name));
    if collides {
        return Err(format!(
            "A template named \u{201c}{name}\u{201d} already exists. Delete it or rename this agent before saving."
        ));
    }

    let persona = PersonaRecord {
        id: uuid::Uuid::new_v4().to_string(),
        display_name: name,
        avatar_url,
        system_prompt,
        runtime,
        model,
        provider,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        env_vars: Default::default(),
        created_at: now.clone(),
        updated_at: now,
    };
    personas.push(persona.clone());
    Ok(persona)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::AgentTemplateSource;
    use std::collections::BTreeMap;

    fn persona(id: &str, display_name: &str) -> PersonaRecord {
        PersonaRecord {
            id: id.to_string(),
            display_name: display_name.to_string(),
            avatar_url: None,
            system_prompt: "saved prompt".to_string(),
            runtime: Some("goose".to_string()),
            model: Some("opus".to_string()),
            provider: Some("anthropic".to_string()),
            name_pool: Vec::new(),
            is_builtin: false,
            is_active: true,
            source_team: None,
            source_team_persona_slug: None,
            env_vars: BTreeMap::new(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
        }
    }

    // ── list_agent_templates core ────────────────────────────────────────

    #[test]
    fn list_filters_inactive_and_builtin_shadow_records() {
        let builtin_count = builtin_agent_templates().len();
        let shadow_id = builtin_agent_templates()[0].id.clone();

        let mut inactive = persona("saved-inactive", "Inactive Saved");
        inactive.is_active = false;
        // A demoted legacy built-in copy: same id as a compiled-in starter.
        let shadow = persona(&shadow_id, "Shadow Copy");
        let kept = persona("saved-kept", "Kept Saved");

        let templates = assemble_agent_templates(&[inactive, shadow, kept]);

        assert_eq!(
            templates.len(),
            builtin_count + 1,
            "only the active non-shadow record joins the built-ins"
        );
        assert!(
            !templates.iter().any(|t| t.id == "saved-inactive"),
            "inactive record must not appear in the catalog"
        );
        assert_eq!(
            templates.iter().filter(|t| t.id == shadow_id).count(),
            1,
            "builtin-shadow record must not duplicate the built-in"
        );
        let kept_template = templates.iter().find(|t| t.id == "saved-kept").unwrap();
        assert_eq!(kept_template.source, AgentTemplateSource::Saved);
    }

    #[test]
    fn list_orders_builtins_first_then_saved_sorted_by_name() {
        let builtin_count = builtin_agent_templates().len();
        let saved = vec![persona("saved-b", "zeta"), persona("saved-a", "Alpha")];

        let templates = assemble_agent_templates(&saved);

        assert!(templates[..builtin_count]
            .iter()
            .all(|t| t.source == AgentTemplateSource::Builtin));
        let saved_ids: Vec<&str> = templates[builtin_count..]
            .iter()
            .map(|t| t.id.as_str())
            .collect();
        // Case-insensitive name sort: "Alpha" before "zeta".
        assert_eq!(saved_ids, vec!["saved-a", "saved-b"]);
    }

    // ── save_agent_as_template core ──────────────────────────────────────

    #[test]
    fn insert_errors_when_active_saved_template_shares_the_name() {
        let mut existing = persona("existing-id", "My Agent");
        existing.env_vars = BTreeMap::from([("API_KEY".to_string(), "secret".to_string())]);
        let mut personas = vec![existing];

        let result = insert_template_persona(
            &mut personas,
            "my agent".to_string(), // case-insensitive match
            Some("https://example.com/new.png".to_string()),
            "new prompt".to_string(),
            Some("acp".to_string()),
            Some("sonnet".to_string()),
            Some("openai".to_string()),
            "2025-06-01T00:00:00Z".to_string(),
        );

        let err = result.expect_err("same-name save must error, not update in place");
        assert!(
            err.contains("already exists"),
            "error should explain the collision: {err}"
        );
        assert_eq!(personas.len(), 1, "nothing inserted on collision");
        let p = &personas[0];
        assert_eq!(p.system_prompt, "saved prompt", "existing record untouched");
        assert_eq!(p.updated_at, "2025-01-01T00:00:00Z", "no silent refresh");
    }

    #[test]
    fn insert_errors_when_name_collides_with_a_builtin_starter() {
        let builtin_name = builtin_agent_templates()[0].display_name.clone();
        let mut personas = Vec::new();

        let result = insert_template_persona(
            &mut personas,
            builtin_name.to_uppercase(),
            None,
            "prompt".to_string(),
            None,
            None,
            None,
            "2025-06-01T00:00:00Z".to_string(),
        );

        assert!(
            result.is_err(),
            "built-in starter names are reserved in the catalog"
        );
        assert!(personas.is_empty());
    }

    #[test]
    fn insert_unique_name_creates_fresh_record_without_env_vars() {
        let mut personas = vec![persona("other-id", "Other")];

        let result = insert_template_persona(
            &mut personas,
            "Brand New".to_string(),
            None,
            "prompt".to_string(),
            None,
            None,
            None,
            "2025-06-01T00:00:00Z".to_string(),
        )
        .expect("unique name saves cleanly");

        assert_eq!(personas.len(), 2, "unique name inserts a new record");
        assert!(result.is_active);
        assert!(!result.is_builtin);
        assert!(
            result.env_vars.is_empty(),
            "templates never carry credentials"
        );
        assert_eq!(result.created_at, "2025-06-01T00:00:00Z");
    }

    #[test]
    fn insert_ignores_inactive_and_team_sourced_records_with_same_name() {
        let mut inactive = persona("inactive-id", "My Agent");
        inactive.is_active = false;
        let mut team_owned = persona("team-id", "My Agent");
        team_owned.source_team = Some("team-1".to_string());
        let mut personas = vec![inactive, team_owned];

        let result = insert_template_persona(
            &mut personas,
            "My Agent".to_string(),
            None,
            "prompt".to_string(),
            None,
            None,
            None,
            "2025-06-01T00:00:00Z".to_string(),
        )
        .expect("inactive/team-sourced records neither block nor get hijacked");

        assert_eq!(
            personas.len(),
            3,
            "neither the inactive nor the team record may be hijacked"
        );
        assert_ne!(result.id, "inactive-id");
        assert_ne!(result.id, "team-id");
    }
}

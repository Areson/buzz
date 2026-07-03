//! One-time flattening migration: agents own their config outright.
//!
//! Personas used to be the user-facing template primitive — the Agents grid
//! rendered one card per active persona, and agents linked back via
//! `persona_id`. The agents-first model quarantines persona records to team
//! packs (directory-backed teams genuinely need them for pack sync and
//! writeback); every other agent carries its full config on its own record.
//!
//! The migration, run post-identity on every boot (idempotent, cheap no-op
//! once flattened):
//!
//! 1. **Materialize** a stopped managed agent for every `is_active` persona
//!    with no linked agent, so persona-only cards don't visibly disappear.
//!    Pack personas keep their `persona_id` link; others are created flat.
//! 2. **Rewrite teams**: non-pack persona members become agent-pubkey members
//!    (`TeamRecord.agent_pubkeys`); `persona_ids` remains only for pack teams.
//! 3. **Flatten agents**: for every agent linked to a non-pack persona, pin
//!    the live persona config onto the record (mirroring what the next spawn
//!    re-snapshot would have done) and clear `persona_id`.
//! 4. **Deactivate** flattened non-pack personas. Records stay on disk and on
//!    the relay for rollback; the UI never renders them.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use nostr::{Keys, ToBech32};
use tauri::{AppHandle, Manager};

use crate::{
    app_state::AppState,
    managed_agents::{
        effective_agent_command, known_acp_runtime, load_managed_agents, load_personas, load_teams,
        managed_agent_avatar_url, managed_agents_base_dir, normalize_agent_args,
        persona_events::persona_snapshot, save_managed_agents, save_personas, save_teams,
        ManagedAgentRecord, PersonaRecord, TeamRecord, DEFAULT_ACP_COMMAND,
        DEFAULT_AGENT_PARALLELISM, DEFAULT_AGENT_TURN_TIMEOUT_SECONDS,
    },
    util::now_iso,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct FlattenReport {
    pub agents_materialized: usize,
    pub agents_flattened: usize,
    pub teams_rewritten: usize,
    pub personas_deactivated: usize,
}

impl FlattenReport {
    pub fn is_noop(&self) -> bool {
        self.agents_materialized == 0
            && self.agents_flattened == 0
            && self.teams_rewritten == 0
            && self.personas_deactivated == 0
    }
}

fn is_pack_persona(persona: &PersonaRecord) -> bool {
    persona.source_team.is_some()
}

/// Compute the NIP-OA auth tag for a freshly generated agent keypair.
fn compute_agent_auth_tag(owner_keys: &Keys, agent_keys: &Keys) -> Result<String, String> {
    let compat_owner = nostr::Keys::parse(&owner_keys.secret_key().to_secret_hex())
        .map_err(|e| format!("failed to bridge owner keys: {e}"))?;
    let compat_agent = nostr::PublicKey::from_hex(&agent_keys.public_key().to_hex())
        .map_err(|e| format!("failed to bridge agent pubkey: {e}"))?;
    buzz_sdk_pkg::nip_oa::compute_auth_tag(&compat_owner, &compat_agent, "")
        .map_err(|e| format!("failed to compute NIP-OA auth tag: {e}"))
}

/// Build a stopped managed-agent record from a persona's fields.
///
/// The record is created WITH the `persona_id` link — for non-pack personas
/// the flattening pass (step 3) immediately pins the config and clears the
/// link in the same run; pack personas keep it (plus pack metadata rooted at
/// `teams_base_dir`) so pack sync and ACP pack resolution keep working.
fn materialize_agent_from_persona(
    persona: &PersonaRecord,
    personas: &[PersonaRecord],
    owner_keys: &Keys,
    teams_base_dir: Option<&Path>,
) -> Result<ManagedAgentRecord, String> {
    let agent_keys = Keys::generate();
    let pubkey = agent_keys.public_key().to_hex();
    let private_key_nsec = agent_keys
        .secret_key()
        .to_bech32()
        .map_err(|e| format!("failed to encode private key: {e}"))?;
    let auth_tag = compute_agent_auth_tag(owner_keys, &agent_keys)?;

    let agent_command = effective_agent_command(Some(&persona.id), personas, None);
    let agent_args = normalize_agent_args(&agent_command, Vec::new());
    let mcp_command = known_acp_runtime(&agent_command)
        .and_then(|r| r.mcp_command)
        .unwrap_or("")
        .to_string();
    let avatar_url = persona
        .avatar_url
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| managed_agent_avatar_url(&agent_command));

    let snapshot = persona_snapshot(persona, &BTreeMap::new());

    let pack_metadata: Option<(std::path::PathBuf, String)> = persona
        .source_team
        .as_deref()
        .zip(persona.source_team_persona_slug.as_deref())
        .zip(teams_base_dir)
        .map(|((team_id, slug), base)| (base.join(team_id), slug.to_owned()));

    let now = now_iso();
    Ok(ManagedAgentRecord {
        pubkey,
        name: persona.display_name.clone(),
        persona_id: Some(persona.id.clone()),
        private_key_nsec,
        auth_tag: Some(auth_tag),
        relay_url: String::new(),
        avatar_url,
        acp_command: DEFAULT_ACP_COMMAND.to_string(),
        agent_command: agent_command.clone(),
        agent_command_override: None,
        agent_args,
        mcp_command,
        turn_timeout_seconds: DEFAULT_AGENT_TURN_TIMEOUT_SECONDS,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: DEFAULT_AGENT_PARALLELISM,
        system_prompt: snapshot.system_prompt,
        model: snapshot.model,
        provider: snapshot.provider,
        persona_source_version: Some(snapshot.source_version),
        mcp_toolsets: None,
        env_vars: snapshot.env_vars,
        // Materialized agents come up stopped and stay manual-start: nothing
        // should silently boot a fleet of formerly-dormant persona cards.
        start_on_app_launch: false,
        runtime_pid: None,
        backend: Default::default(),
        backend_agent_id: None,
        provider_binary_path: None,
        persona_team_dir: pack_metadata.as_ref().map(|(path, _)| path.clone()),
        persona_name_in_team: pack_metadata.map(|(_, slug)| slug),
        created_at: now.clone(),
        updated_at: now,
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        respond_to: Default::default(),
        respond_to_allowlist: Vec::new(),
        relay_mesh: None,
    })
}

/// Materialize stopped agents for active personas with no linked agent
/// (pure core — mutates `agents` in place, returns the number created).
fn materialize_missing_agents(
    personas: &[PersonaRecord],
    agents: &mut Vec<ManagedAgentRecord>,
    owner_keys: &Keys,
    teams_base_dir: Option<&Path>,
) -> usize {
    let mut created = 0usize;
    for persona in personas.iter().filter(|p| p.is_active) {
        let has_agent = agents
            .iter()
            .any(|agent| agent.persona_id.as_deref() == Some(persona.id.as_str()));
        if has_agent {
            continue;
        }
        match materialize_agent_from_persona(persona, personas, owner_keys, teams_base_dir) {
            Ok(record) => {
                eprintln!(
                    "buzz-desktop: flatten: materialized stopped agent '{}' for persona {}",
                    record.name, persona.id
                );
                agents.push(record);
                created += 1;
            }
            Err(e) => {
                eprintln!(
                    "buzz-desktop: flatten: failed to materialize agent for persona {}: {e}",
                    persona.id
                );
            }
        }
    }
    created
}

/// Pure flattening core over in-memory stores. Returns what changed so the
/// IO wrapper can save only touched files.
fn flatten_in_place(
    personas: &mut [PersonaRecord],
    agents: &mut Vec<ManagedAgentRecord>,
    teams: &mut [TeamRecord],
    owner_keys: &Keys,
    teams_base_dir: Option<&Path>,
) -> FlattenReport {
    // ── Step 1: materialize agents for active personas with no agent ────────
    let mut report = FlattenReport {
        agents_materialized: materialize_missing_agents(
            personas,
            agents,
            owner_keys,
            teams_base_dir,
        ),
        ..FlattenReport::default()
    };

    // ── Step 2: rewrite team membership for non-pack personas ───────────────
    // First agent per persona id wins as the team-member representative.
    let persona_agent: HashMap<String, String> = {
        let mut map = HashMap::new();
        for agent in agents.iter() {
            if let Some(pid) = agent.persona_id.as_deref() {
                map.entry(pid.to_string())
                    .or_insert_with(|| agent.pubkey.clone());
            }
        }
        map
    };

    for team in teams.iter_mut() {
        let mut remaining_persona_ids = Vec::with_capacity(team.persona_ids.len());
        let mut team_changed = false;
        for pid in std::mem::take(&mut team.persona_ids) {
            let persona = personas.iter().find(|p| p.id == pid);
            let keep_as_persona = persona.is_some_and(is_pack_persona);
            if keep_as_persona {
                remaining_persona_ids.push(pid);
                continue;
            }
            // Non-pack (or dangling) member: swap in the linked agent when one
            // exists, otherwise drop the member.
            if let Some(pubkey) = persona_agent.get(&pid) {
                if !team.agent_pubkeys.iter().any(|pk| pk == pubkey) {
                    team.agent_pubkeys.push(pubkey.clone());
                }
            } else {
                eprintln!(
                    "buzz-desktop: flatten: dropping team member {pid} from '{}' (no agent to migrate to)",
                    team.name
                );
            }
            team_changed = true;
        }
        team.persona_ids = remaining_persona_ids;
        if team_changed {
            team.updated_at = now_iso();
            report.teams_rewritten += 1;
        }
    }

    // ── Step 3: flatten agents linked to non-pack personas ──────────────────
    for agent in agents.iter_mut() {
        let Some(pid) = agent.persona_id.clone() else {
            continue;
        };
        let Some(persona) = personas.iter().find(|p| p.id == pid) else {
            // Orphaned link — the pinned record fields are all that remain.
            agent.persona_id = None;
            agent.persona_source_version = None;
            agent.updated_at = now_iso();
            report.agents_flattened += 1;
            continue;
        };
        if is_pack_persona(persona) {
            continue;
        }

        // Pin the live persona config onto the record — exactly what the next
        // spawn's re-snapshot would have applied (persona env layered under
        // the agent's own env overrides).
        let snapshot = persona_snapshot(persona, &agent.env_vars);
        if let Some(prompt) = snapshot.system_prompt {
            agent.system_prompt = Some(prompt);
        }
        agent.model = snapshot.model;
        agent.provider = snapshot.provider;
        agent.env_vars = snapshot.env_vars;
        if agent
            .avatar_url
            .as_deref()
            .map(str::trim)
            .is_none_or(str::is_empty)
        {
            agent.avatar_url = persona.avatar_url.clone();
        }

        // Pin the effective harness before the persona link goes away.
        let effective_command = effective_agent_command(
            Some(&pid),
            personas,
            agent.agent_command_override.as_deref(),
        );
        agent.agent_args = normalize_agent_args(&effective_command, agent.agent_args.clone());
        if agent.mcp_command.trim().is_empty() {
            agent.mcp_command = known_acp_runtime(&effective_command)
                .and_then(|r| r.mcp_command)
                .unwrap_or("")
                .to_string();
        }
        agent.agent_command = effective_command.clone();
        if agent.agent_command_override.is_none() {
            agent.agent_command_override = Some(effective_command);
        }

        agent.persona_id = None;
        agent.persona_source_version = None;
        agent.updated_at = now_iso();
        report.agents_flattened += 1;
    }

    // ── Step 4: deactivate flattened non-pack personas ──────────────────────
    for persona in personas.iter_mut() {
        if !is_pack_persona(persona) && persona.is_active {
            persona.is_active = false;
            persona.updated_at = now_iso();
            report.personas_deactivated += 1;
        }
    }

    report
}

/// Materialize stopped agents for active personas with no linked agent.
///
/// MUST be called with the `managed_agents_store_lock` already held — this
/// reads and writes the agent store without acquiring it. Used by team pack
/// install/sync so freshly synced pack personas immediately appear as
/// (stopped) agents in the grid. Returns the number of agents created.
pub fn materialize_agents_for_active_personas_locked(
    app: &AppHandle,
    owner_keys: &Keys,
) -> Result<usize, String> {
    let personas = load_personas(app)?;
    let mut agents = load_managed_agents(app)?;
    let teams_base = managed_agents_base_dir(app)?.join("teams");

    let created = materialize_missing_agents(&personas, &mut agents, owner_keys, Some(&teams_base));
    if created > 0 {
        save_managed_agents(app, &agents)?;
    }
    Ok(created)
}

/// Run the full flattening migration. Acquires the store lock itself — call
/// from boot setup after the owner identity is resolved, never from a command
/// that already holds the lock.
pub fn flatten_personas_into_agents(
    app: &AppHandle,
    owner_keys: &Keys,
) -> Result<FlattenReport, String> {
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;

    let mut personas = load_personas(app)?;
    let mut agents = load_managed_agents(app)?;
    let mut teams = load_teams(app)?;
    let teams_base = managed_agents_base_dir(app)?.join("teams");

    let report = flatten_in_place(
        &mut personas,
        &mut agents,
        &mut teams,
        owner_keys,
        Some(&teams_base),
    );

    if report.is_noop() {
        return Ok(report);
    }

    // Reference holders first (teams, agents), then personas — matches the
    // crash-ordering convention in `sync_team_personas`.
    if report.teams_rewritten > 0 {
        save_teams(app, &teams)?;
    }
    if report.agents_materialized > 0 || report.agents_flattened > 0 {
        save_managed_agents(app, &agents)?;
    }
    if report.personas_deactivated > 0 {
        save_personas(app, &personas)?;
    }

    Ok(report)
}

#[cfg(test)]
#[path = "flatten_tests.rs"]
mod tests;

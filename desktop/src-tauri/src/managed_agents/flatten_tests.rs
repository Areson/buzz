use std::collections::BTreeMap;

use nostr::Keys;

use super::flatten_in_place;
use crate::managed_agents::{ManagedAgentRecord, PersonaRecord, TeamRecord};

fn persona(id: &str, name: &str) -> PersonaRecord {
    PersonaRecord {
        id: id.to_string(),
        display_name: name.to_string(),
        avatar_url: Some(format!("https://example.com/{id}.png")),
        system_prompt: format!("You are {name}."),
        runtime: Some("goose".to_string()),
        model: Some("claude-test".to_string()),
        provider: Some("anthropic".to_string()),
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        env_vars: BTreeMap::from([("PERSONA_KEY".to_string(), "persona-value".to_string())]),
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

fn pack_persona(id: &str, name: &str) -> PersonaRecord {
    PersonaRecord {
        source_team: Some("com.example.pack".to_string()),
        source_team_persona_slug: Some("scout".to_string()),
        ..persona(id, name)
    }
}

fn agent(pubkey: &str, name: &str, persona_id: Option<&str>) -> ManagedAgentRecord {
    ManagedAgentRecord {
        pubkey: pubkey.to_string(),
        name: name.to_string(),
        persona_id: persona_id.map(str::to_string),
        private_key_nsec: String::new(),
        auth_tag: None,
        relay_url: String::new(),
        avatar_url: None,
        acp_command: "buzz-acp".to_string(),
        agent_command: "goose".to_string(),
        agent_command_override: None,
        agent_args: vec!["acp".to_string()],
        mcp_command: String::new(),
        turn_timeout_seconds: 320,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: Some("hash".to_string()),
        mcp_toolsets: None,
        start_on_app_launch: false,
        runtime_pid: None,
        backend: Default::default(),
        backend_agent_id: None,
        provider_binary_path: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        env_vars: BTreeMap::from([("AGENT_KEY".to_string(), "agent-value".to_string())]),
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        respond_to: Default::default(),
        respond_to_allowlist: vec![],
        relay_mesh: None,
    }
}

fn team(id: &str, persona_ids: &[&str]) -> TeamRecord {
    TeamRecord {
        id: id.to_string(),
        name: id.to_string(),
        description: None,
        persona_ids: persona_ids.iter().map(|s| s.to_string()).collect(),
        agent_pubkeys: Vec::new(),
        is_builtin: false,
        source_dir: None,
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

fn owner_keys() -> Keys {
    Keys::generate()
}

#[test]
fn flattens_persona_backed_agent_and_pins_config() {
    let mut personas = vec![persona("p1", "Honey")];
    let mut agents = vec![agent("agent-pk", "Honey", Some("p1"))];
    let mut teams = vec![];

    let report = flatten_in_place(&mut personas, &mut agents, &mut teams, &owner_keys(), None);

    assert_eq!(report.agents_flattened, 1);
    assert_eq!(report.agents_materialized, 0);
    let flat = &agents[0];
    assert_eq!(flat.persona_id, None);
    assert_eq!(flat.persona_source_version, None);
    assert_eq!(flat.system_prompt.as_deref(), Some("You are Honey."));
    assert_eq!(flat.model.as_deref(), Some("claude-test"));
    assert_eq!(flat.provider.as_deref(), Some("anthropic"));
    // Persona env layered under agent env — both keys survive.
    assert_eq!(
        flat.env_vars.get("PERSONA_KEY").map(String::as_str),
        Some("persona-value")
    );
    assert_eq!(
        flat.env_vars.get("AGENT_KEY").map(String::as_str),
        Some("agent-value")
    );
    // Avatar backfilled from the persona.
    assert_eq!(
        flat.avatar_url.as_deref(),
        Some("https://example.com/p1.png")
    );
    // Harness pinned so losing the persona link doesn't fall back to default.
    assert!(flat.agent_command_override.is_some());
    // Persona deactivated.
    assert_eq!(report.personas_deactivated, 1);
    assert!(!personas[0].is_active);
}

#[test]
fn agent_env_overrides_win_over_persona_env() {
    let mut personas = vec![persona("p1", "Honey")];
    personas[0]
        .env_vars
        .insert("SHARED".to_string(), "persona".to_string());
    let mut agents = vec![agent("agent-pk", "Honey", Some("p1"))];
    agents[0]
        .env_vars
        .insert("SHARED".to_string(), "agent".to_string());
    let mut teams = vec![];

    flatten_in_place(&mut personas, &mut agents, &mut teams, &owner_keys(), None);

    assert_eq!(
        agents[0].env_vars.get("SHARED").map(String::as_str),
        Some("agent")
    );
}

#[test]
fn materializes_stopped_agent_for_active_persona_without_agent() {
    let mut personas = vec![persona("p1", "Honey")];
    let mut agents = vec![];
    let mut teams = vec![];

    let report = flatten_in_place(&mut personas, &mut agents, &mut teams, &owner_keys(), None);

    assert_eq!(report.agents_materialized, 1);
    assert_eq!(agents.len(), 1);
    let created = &agents[0];
    assert_eq!(created.name, "Honey");
    // Materialized flat: persona link cleared during flattening.
    assert_eq!(created.persona_id, None);
    assert_eq!(created.system_prompt.as_deref(), Some("You are Honey."));
    assert!(!created.start_on_app_launch);
    assert!(created.auth_tag.is_some());
    assert!(!created.private_key_nsec.is_empty());
    assert_eq!(created.backend, crate::managed_agents::BackendKind::Local);
    // Persona ends up deactivated.
    assert!(!personas[0].is_active);
}

#[test]
fn inactive_persona_without_agent_is_not_materialized() {
    let mut personas = vec![PersonaRecord {
        is_active: false,
        ..persona("p1", "Dormant")
    }];
    let mut agents = vec![];
    let mut teams = vec![];

    let report = flatten_in_place(&mut personas, &mut agents, &mut teams, &owner_keys(), None);

    assert_eq!(report.agents_materialized, 0);
    assert!(agents.is_empty());
}

#[test]
fn pack_personas_and_their_agents_are_untouched() {
    let mut personas = vec![pack_persona("pack-p", "Scout")];
    let mut agents = vec![agent("agent-pk", "Scout", Some("pack-p"))];
    let mut teams = vec![];

    let report = flatten_in_place(&mut personas, &mut agents, &mut teams, &owner_keys(), None);

    assert_eq!(report.agents_flattened, 0);
    assert_eq!(report.personas_deactivated, 0);
    assert_eq!(agents[0].persona_id.as_deref(), Some("pack-p"));
    assert!(personas[0].is_active);
}

#[test]
fn pack_persona_without_agent_materializes_with_persona_link() {
    let mut personas = vec![pack_persona("pack-p", "Scout")];
    let mut agents = vec![];
    let mut teams = vec![];

    let report = flatten_in_place(&mut personas, &mut agents, &mut teams, &owner_keys(), None);

    assert_eq!(report.agents_materialized, 1);
    assert_eq!(agents[0].persona_id.as_deref(), Some("pack-p"));
    assert!(agents[0].persona_source_version.is_some());
}

#[test]
fn team_persona_members_migrate_to_agent_pubkeys() {
    let mut personas = vec![persona("p1", "Honey"), pack_persona("pack-p", "Scout")];
    let mut agents = vec![
        agent("honey-pk", "Honey", Some("p1")),
        agent("scout-pk", "Scout", Some("pack-p")),
    ];
    let mut teams = vec![team("t1", &["p1", "pack-p"])];

    let report = flatten_in_place(&mut personas, &mut agents, &mut teams, &owner_keys(), None);

    assert_eq!(report.teams_rewritten, 1);
    // Non-pack member became an agent pubkey; pack member stayed a persona id.
    assert_eq!(teams[0].agent_pubkeys, vec!["honey-pk".to_string()]);
    assert_eq!(teams[0].persona_ids, vec!["pack-p".to_string()]);
}

#[test]
fn team_member_without_agent_is_materialized_then_migrated() {
    let mut personas = vec![persona("p1", "Honey")];
    let mut agents = vec![];
    let mut teams = vec![team("t1", &["p1"])];

    let report = flatten_in_place(&mut personas, &mut agents, &mut teams, &owner_keys(), None);

    assert_eq!(report.agents_materialized, 1);
    assert_eq!(teams[0].persona_ids, Vec::<String>::new());
    assert_eq!(teams[0].agent_pubkeys, vec![agents[0].pubkey.clone()]);
}

#[test]
fn dangling_team_member_is_dropped() {
    let mut personas = vec![];
    let mut agents = vec![];
    let mut teams = vec![team("t1", &["ghost"])];

    let report = flatten_in_place(&mut personas, &mut agents, &mut teams, &owner_keys(), None);

    assert_eq!(report.teams_rewritten, 1);
    assert!(teams[0].persona_ids.is_empty());
    assert!(teams[0].agent_pubkeys.is_empty());
}

#[test]
fn orphaned_agent_link_is_cleared() {
    let mut personas = vec![];
    let mut agents = vec![agent("agent-pk", "Ghosted", Some("gone"))];
    let mut teams = vec![];

    let report = flatten_in_place(&mut personas, &mut agents, &mut teams, &owner_keys(), None);

    assert_eq!(report.agents_flattened, 1);
    assert_eq!(agents[0].persona_id, None);
}

#[test]
fn second_run_is_a_noop() {
    let mut personas = vec![persona("p1", "Honey"), pack_persona("pack-p", "Scout")];
    let mut agents = vec![agent("honey-pk", "Honey", Some("p1"))];
    let mut teams = vec![team("t1", &["p1", "pack-p"])];

    let keys = owner_keys();
    let first = flatten_in_place(&mut personas, &mut agents, &mut teams, &keys, None);
    assert!(!first.is_noop());

    let second = flatten_in_place(&mut personas, &mut agents, &mut teams, &keys, None);
    assert!(
        second.is_noop(),
        "expected idempotent second run, got {second:?}"
    );
}

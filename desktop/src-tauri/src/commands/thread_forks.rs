use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::{
    app_state::AppState,
    events,
    huddle::relay_api::{fetch_channel_members, parse_channel_uuid, validate_pubkey_hex},
    relay::{get_relay_json, query_relay, submit_event},
};

const THREAD_FORK_TTL_SECONDS: i32 = 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadForkInfo {
    pub parent_channel_id: String,
    pub child_channel_id: String,
    pub root_event_id: String,
    pub creator_pubkey: String,
    pub active: bool,
    pub added: Vec<String>,
    pub errors: Vec<ThreadForkMemberError>,
    #[serde(skip)]
    latest_created_at_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadForkMemberError {
    pub pubkey: String,
    pub error: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ThreadForkActiveState {
    parent_channel_id: String,
    child_channel_id: String,
    root_event_id: String,
    creator_pubkey: String,
    latest_created_at_secs: Option<i64>,
}

fn normalize_thread_fork_channel_name(candidate: Option<String>, fallback: &str) -> String {
    let normalized = candidate
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let name = if normalized.is_empty() {
        fallback
    } else {
        normalized.as_str()
    };
    name.chars().take(80).collect()
}

fn channel_id_from_tags(ev: &nostr::Event) -> Option<String> {
    ev.tags.iter().find_map(|tag| {
        let parts = tag.as_slice();
        if parts.len() >= 2 && parts[0] == "h" {
            Some(parts[1].clone())
        } else {
            None
        }
    })
}

async fn assert_root_belongs_to_parent(
    parent_channel_id: &str,
    root_event_id: &str,
    state: &AppState,
) -> Result<(), String> {
    nostr::EventId::from_hex(root_event_id).map_err(|e| format!("invalid event ID: {e}"))?;
    let events = query_relay(
        state,
        &[serde_json::json!({
            "ids": [root_event_id],
            "limit": 1,
        })],
    )
    .await?;
    let root = events
        .first()
        .ok_or_else(|| "thread root event not found".to_string())?;
    match channel_id_from_tags(root) {
        Some(channel_id) if channel_id == parent_channel_id => Ok(()),
        Some(_) => Err("thread root belongs to a different channel".to_string()),
        None => Err("thread root has no channel association".to_string()),
    }
}

fn monotonic_lifecycle_timestamp(after: Option<u64>) -> nostr::Timestamp {
    let now = nostr::Timestamp::now().as_secs();
    let secs = after
        .and_then(|prior| prior.checked_add(1))
        .map_or(now, |next| now.max(next));
    nostr::Timestamp::from(secs)
}

fn thread_fork_active_state_path(parent_channel_id: &str, root_event_id: &str) -> String {
    format!(
        "/thread-forks/{}/{}/active",
        parent_channel_id,
        root_event_id.to_ascii_lowercase()
    )
}

async fn fetch_thread_fork_state(
    parent_channel_id: &str,
    root_event_id: &str,
    state: &AppState,
) -> Result<Option<ThreadForkInfo>, String> {
    let path = thread_fork_active_state_path(parent_channel_id, root_event_id);
    let active: Option<ThreadForkActiveState> = get_relay_json(state, &path).await?;
    Ok(active.map(|active| ThreadForkInfo {
        parent_channel_id: active.parent_channel_id,
        child_channel_id: active.child_channel_id,
        root_event_id: active.root_event_id,
        creator_pubkey: active.creator_pubkey,
        active: true,
        added: Vec::new(),
        errors: Vec::new(),
        latest_created_at_secs: active
            .latest_created_at_secs
            .and_then(|secs| u64::try_from(secs).ok()),
    }))
}

async fn remove_child_agents(child_channel_id: &str, state: &AppState) {
    let Ok(child_uuid) = parse_channel_uuid(child_channel_id) else {
        return;
    };
    let agent_pubkeys = match fetch_channel_members(child_channel_id, Some("bot"), state).await {
        Ok(pubkeys) => pubkeys,
        Err(error) => {
            eprintln!("buzz-desktop: fetch thread-fork agents for cleanup failed: {error}");
            return;
        }
    };

    for pubkey in agent_pubkeys {
        let Ok(builder) = events::build_remove_member(child_uuid, &pubkey) else {
            continue;
        };
        if let Err(error) = submit_event(builder, state).await {
            eprintln!("buzz-desktop: remove thread-fork agent {pubkey} failed: {error}");
        }
    }
}

/// Return the latest lifecycle state for a thread fork rooted in a parent channel.
#[tauri::command]
pub async fn get_thread_fork_state(
    parent_channel_id: String,
    root_event_id: String,
    state: State<'_, AppState>,
) -> Result<Option<ThreadForkInfo>, String> {
    parse_channel_uuid(&parent_channel_id)?;
    nostr::EventId::from_hex(&root_event_id).map_err(|e| format!("invalid event ID: {e}"))?;
    fetch_thread_fork_state(&parent_channel_id, &root_event_id, &state).await
}

/// User-initiated lane start: create a temporary child channel, enroll selected
/// agents, and post the creator-signed parent-channel link.
#[tauri::command]
pub async fn start_thread_fork(
    parent_channel_id: String,
    root_event_id: String,
    agent_pubkeys: Vec<String>,
    channel_name: Option<String>,
    state: State<'_, AppState>,
) -> Result<ThreadForkInfo, String> {
    parse_channel_uuid(&parent_channel_id)?;
    nostr::EventId::from_hex(&root_event_id).map_err(|e| format!("invalid event ID: {e}"))?;
    assert_root_belongs_to_parent(&parent_channel_id, &root_event_id, &state).await?;
    let creator_pubkey = state.signing_keys()?.public_key().to_hex();

    let existing = fetch_thread_fork_state(&parent_channel_id, &root_event_id, &state).await?;
    if let Some(existing) = existing.as_ref().filter(|fork| fork.active) {
        return Ok(existing.clone());
    }
    let lifecycle_created_at =
        monotonic_lifecycle_timestamp(existing.and_then(|fork| fork.latest_created_at_secs));

    let mut seen = BTreeSet::new();
    let mut agents = Vec::new();
    for pubkey in agent_pubkeys {
        validate_pubkey_hex(&pubkey)?;
        let normalized = pubkey.to_ascii_lowercase();
        if seen.insert(normalized.clone()) {
            agents.push(normalized);
        }
    }

    let child_uuid = Uuid::new_v4();
    let child_channel_id = child_uuid.to_string();
    let fallback_name = format!("lane-{}-{}", &root_event_id[..8], &child_channel_id[..8]);
    let channel_name = normalize_thread_fork_channel_name(channel_name, &fallback_name);

    let mut child_created = false;
    let mut added = Vec::new();
    let mut errors = Vec::new();

    let result: Result<(), String> = async {
        let create_builder = events::build_create_channel(
            child_uuid,
            &channel_name,
            "private",
            "stream",
            Some("Temporary child channel for a thread lane."),
            Some(THREAD_FORK_TTL_SECONDS),
        )?;
        submit_event(create_builder, &state).await?;
        child_created = true;

        for pubkey in &agents {
            let add_builder = match events::build_add_member(child_uuid, pubkey, Some("bot")) {
                Ok(builder) => builder,
                Err(error) => {
                    errors.push(ThreadForkMemberError {
                        pubkey: pubkey.clone(),
                        error,
                    });
                    continue;
                }
            };
            match submit_event(add_builder, &state).await {
                Ok(_) => added.push(pubkey.clone()),
                Err(error) => errors.push(ThreadForkMemberError {
                    pubkey: pubkey.clone(),
                    error,
                }),
            }
        }

        let started_builder = events::build_thread_fork_started(
            &parent_channel_id,
            &child_channel_id,
            &root_event_id,
        )?
        .custom_created_at(lifecycle_created_at);
        submit_event(started_builder, &state).await?;
        Ok(())
    }
    .await;

    if let Err(error) = result {
        if child_created {
            if let Ok(archive_builder) = events::build_archive(child_uuid) {
                if let Err(archive_error) = submit_event(archive_builder, &state).await {
                    eprintln!(
                        "buzz-desktop: rollback archive of thread fork {child_channel_id} failed: {archive_error}"
                    );
                }
            }
        }
        return Err(error);
    }

    Ok(ThreadForkInfo {
        parent_channel_id,
        child_channel_id,
        root_event_id: root_event_id.to_ascii_lowercase(),
        creator_pubkey,
        active: true,
        added,
        errors,
        latest_created_at_secs: Some(lifecycle_created_at.as_secs()),
    })
}

/// User-initiated lane stop: post the parent-channel end advisory, remove child
/// bot members, and archive the temporary child channel best-effort.
#[tauri::command]
pub async fn end_thread_fork(
    parent_channel_id: String,
    child_channel_id: String,
    root_event_id: String,
    state: State<'_, AppState>,
) -> Result<ThreadForkInfo, String> {
    parse_channel_uuid(&parent_channel_id)?;
    let child_uuid = parse_channel_uuid(&child_channel_id)?;
    nostr::EventId::from_hex(&root_event_id).map_err(|e| format!("invalid event ID: {e}"))?;
    let current_pubkey = state.signing_keys()?.public_key().to_hex();
    let active_fork = fetch_thread_fork_state(&parent_channel_id, &root_event_id, &state)
        .await?
        .filter(|fork| fork.active)
        .ok_or_else(|| "thread lane is not active".to_string())?;
    if active_fork.child_channel_id != child_channel_id {
        return Err("active thread lane targets a different child channel".to_string());
    }
    if !active_fork
        .creator_pubkey
        .eq_ignore_ascii_case(&current_pubkey)
    {
        return Err("only the thread lane creator can end this lane".to_string());
    }
    let lifecycle_created_at = monotonic_lifecycle_timestamp(active_fork.latest_created_at_secs);

    let ended_builder =
        events::build_thread_fork_ended(&parent_channel_id, &child_channel_id, &root_event_id)?
            .custom_created_at(lifecycle_created_at);
    submit_event(ended_builder, &state).await?;

    remove_child_agents(&child_channel_id, &state).await;
    if let Ok(archive_builder) = events::build_archive(child_uuid) {
        if let Err(error) = submit_event(archive_builder, &state).await {
            eprintln!("buzz-desktop: archive thread-fork child channel failed: {error}");
        }
    }

    Ok(ThreadForkInfo {
        parent_channel_id,
        child_channel_id,
        root_event_id: root_event_id.to_ascii_lowercase(),
        creator_pubkey: current_pubkey,
        active: false,
        added: Vec::new(),
        errors: Vec::new(),
        latest_created_at_secs: Some(lifecycle_created_at.as_secs()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_fork_active_state_path_lowercases_root_id() {
        let parent_channel_id = uuid::Uuid::new_v4().to_string();
        let root_event_id = "AB".repeat(32);

        assert_eq!(
            thread_fork_active_state_path(&parent_channel_id, &root_event_id),
            format!(
                "/thread-forks/{}/{}/active",
                parent_channel_id,
                root_event_id.to_ascii_lowercase()
            )
        );
    }

    #[test]
    fn monotonic_lifecycle_timestamp_advances_past_prior_event() {
        let prior = nostr::Timestamp::now().as_secs() + 60;

        let timestamp = monotonic_lifecycle_timestamp(Some(prior));

        assert_eq!(timestamp.as_secs(), prior + 1);
    }
}

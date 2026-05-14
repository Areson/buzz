use super::{
    find_managed_agent_mut, kill_stale_tracked_processes, load_managed_agents, save_managed_agents,
    spawn_agent_child, sync_managed_agent_processes, BackendKind, ManagedAgentProcess,
};
use crate::app_state::AppState;
use crate::commands::{collect_profile_sync_params, ProfileSyncParams};
use crate::relay::sync_managed_agent_profile;
use crate::util;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;

/// Restore managed agents that were running before the app was closed.
///
/// Split into four phases to minimise lock contention with the frontend:
///   A (under lock): sync process state, cleanup, collect agents to start
///   B (no locks):   resolve commands and spawn processes in parallel
///   C (re-lock):    write back PIDs and status to records on disk
///   D (no locks):   fire-and-forget profile sync for started agents
pub fn restore_managed_agents_on_launch(
    app: &tauri::AppHandle,
    shutdown_started: &AtomicBool,
) -> Result<(), String> {
    if shutdown_started.load(Ordering::SeqCst) {
        return Ok(());
    }

    let state = app.state::<AppState>();

    // ── Phase A (under lock): housekeeping + collect agents to restore ──
    let agents_to_start: Vec<super::ManagedAgentRecord>;
    {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;

        if shutdown_started.load(Ordering::SeqCst) {
            return Ok(());
        }

        let mut records = load_managed_agents(app)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|error| error.to_string())?;
        let mut changed = sync_managed_agent_processes(&mut records, &mut runtimes);
        changed |= kill_stale_tracked_processes(&mut records, &runtimes);

        let tracked_pids: Vec<u32> = records
            .iter()
            .filter_map(|r| r.runtime_pid)
            .chain(runtimes.values().map(|rt| rt.child.id()))
            .collect();
        super::sweep_orphaned_agent_processes(app, &tracked_pids);

        let candidates: Vec<String> = records
            .iter()
            .filter(|record| record.start_on_app_launch && record.backend == BackendKind::Local)
            .map(|record| record.pubkey.clone())
            .collect();

        let mut to_start = Vec::new();
        for pubkey in &candidates {
            if let Some(runtime) = runtimes.get_mut(pubkey) {
                if runtime.child.try_wait().ok().flatten().is_none() {
                    continue;
                }
            }
            if let Some(record) = records.iter().find(|r| r.pubkey == *pubkey) {
                if let Some(pid) = record.runtime_pid {
                    if super::process_is_running(pid) {
                        continue;
                    }
                }
                to_start.push(record.clone());
            }
        }
        agents_to_start = to_start;

        if changed {
            save_managed_agents(app, &records)?;
        }
    }

    if agents_to_start.is_empty() {
        return Ok(());
    }

    // Snapshot the workspace owner pubkey once for the legacy auth_tag fallback.
    // Read outside the per-agent spawn loop so all parallel spawns see the same
    // value and we don't lock `state.keys` repeatedly.
    let owner_hex: Option<String> = state
        .keys
        .lock()
        .map_err(|e| e.to_string())
        .ok()
        .map(|k| k.public_key().to_hex());

    // ── Phase B (no locks): resolve commands and spawn processes in parallel ──
    let spawn_results: Vec<(
        String,
        Result<(std::process::Child, std::path::PathBuf), String>,
    )> = std::thread::scope(|scope| {
        let owner_hex_ref = owner_hex.as_deref();
        let handles: Vec<_> = agents_to_start
            .iter()
            .filter(|_| !shutdown_started.load(Ordering::SeqCst))
            .map(|record| {
                let pubkey = record.pubkey.clone();
                let handle = scope.spawn(move || {
                    let result = spawn_agent_child(app, record, owner_hex_ref);
                    (pubkey, result)
                });
                handle
            })
            .collect();

        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    if spawn_results.is_empty() {
        return Ok(());
    }

    // ── Phase C (re-acquire lock): write back PIDs, collect profile sync params ──
    let sync_params_list: Vec<ProfileSyncParams>;
    {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut records = load_managed_agents(app)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|error| error.to_string())?;

        let mut started_pubkeys = Vec::new();
        for (pubkey, result) in spawn_results {
            let record = match find_managed_agent_mut(&mut records, &pubkey) {
                Ok(r) => r,
                Err(_) => continue,
            };
            match result {
                Ok((child, log_path)) => {
                    let now = util::now_iso();
                    record.updated_at = now.clone();
                    record.runtime_pid = Some(child.id());
                    record.last_started_at = Some(now);
                    record.last_stopped_at = None;
                    record.last_exit_code = None;
                    record.last_error = None;
                    runtimes.insert(pubkey.clone(), ManagedAgentProcess { child, log_path });
                    started_pubkeys.push(pubkey);
                }
                Err(error) => {
                    record.updated_at = util::now_iso();
                    record.last_error = Some(error);
                }
            }
        }

        // Collect profile sync params for successfully started agents.
        sync_params_list = started_pubkeys
            .iter()
            .filter_map(|pk| {
                let record = records.iter().find(|r| r.pubkey == *pk)?;
                collect_profile_sync_params(app, record)
            })
            .collect();

        save_managed_agents(app, &records)?;
    } // lock dropped

    // ── Phase D (no locks): fire-and-forget profile sync for started agents ──
    for params in sync_params_list {
        let app_handle = app.clone();
        tauri::async_runtime::spawn(async move {
            let state = app_handle.state::<AppState>();
            if let Err(e) = sync_managed_agent_profile(
                &state,
                &params.relay_url,
                &params.agent_keys,
                &params.display_name,
                params.avatar_url.as_deref(),
                params.auth_tag.as_deref(),
                Some(params.respond_to.as_str()),
            )
            .await
            {
                eprintln!("sprout-desktop: profile sync on restore failed for agent: {e}");
            }
        });
    }

    Ok(())
}

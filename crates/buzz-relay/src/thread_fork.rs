//! Fork-linked thread delivery authorization.

use uuid::Uuid;

use buzz_core::{CommunityId, StoredEvent};

use crate::state::AppState;

fn marked_thread_event_ids(event: &nostr::Event) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    event.tags.iter().fold((None, None), |mut acc, tag| {
        let parts = tag.as_slice();
        if parts.len() >= 4 && parts[0] == "e" {
            if let Some(id) = hex::decode(&parts[1]).ok().filter(|id| id.len() == 32) {
                match parts[3].as_str() {
                    "root" => acc.0 = Some(id),
                    "reply" => acc.1 = Some(id),
                    _ => {}
                }
            }
        }
        acc
    })
}

/// Resolve the NIP-10 thread root for an event.
pub(crate) fn thread_root_event_id(event: &nostr::Event) -> Option<Vec<u8>> {
    let (root_event_id, reply_event_id) = marked_thread_event_ids(event);
    match (root_event_id, reply_event_id) {
        (Some(root), Some(_reply)) => Some(root),
        (None, Some(reply)) => Some(reply),
        (Some(_), None) | (None, None) => None,
    }
}

/// Verify the single C2 authorization predicate:
///
/// CHILD is a creator-signed fork of PARENT for this thread root.
pub(crate) async fn fork_link_authorizes(
    state: &AppState,
    community_id: CommunityId,
    parent_channel_id: Uuid,
    child_channel_id: Uuid,
    root_event_id: &[u8],
) -> Result<bool, String> {
    if parent_channel_id == child_channel_id || root_event_id.len() != 32 {
        return Ok(false);
    }

    let child_channel = state
        .db
        .get_channel(community_id, child_channel_id)
        .await
        .map_err(|e| format!("db error loading child channel: {e}"))?;

    state
        .db
        .thread_fork_link_is_active(
            community_id,
            parent_channel_id,
            child_channel_id,
            root_event_id,
            &child_channel.created_by,
        )
        .await
        .map_err(|e| format!("db error checking thread fork link: {e}"))
}

/// Return the parent channel that may receive this child event through C2, if
/// and only if the fork-link predicate authorizes the exact thread root.
pub(crate) async fn authorized_parent_channel_for_event(
    state: &AppState,
    community_id: CommunityId,
    event: &StoredEvent,
) -> Result<Option<Uuid>, String> {
    let Some(child_channel_id) = event.channel_id else {
        return Ok(None);
    };
    let Some(root_event_id) = thread_root_event_id(&event.event) else {
        return Ok(None);
    };

    let root_event = state
        .db
        .get_event_by_id(community_id, &root_event_id)
        .await
        .map_err(|e| format!("db error loading thread root: {e}"))?;
    let Some(parent_channel_id) = root_event.and_then(|root| root.channel_id) else {
        return Ok(None);
    };
    if parent_channel_id == child_channel_id {
        return Ok(None);
    }

    if fork_link_authorizes(
        state,
        community_id,
        parent_channel_id,
        child_channel_id,
        &root_event_id,
    )
    .await?
    {
        Ok(Some(parent_channel_id))
    } else {
        Ok(None)
    }
}

/// Return whether a row outside the viewer's directly accessible channels is
/// readable through a verified fork link from one of those channels.
pub(crate) async fn event_accessible_via_thread_fork(
    state: &AppState,
    community_id: CommunityId,
    event: &StoredEvent,
    accessible_channels: &[Uuid],
) -> Result<bool, String> {
    let Some(parent_channel_id) =
        authorized_parent_channel_for_event(state, community_id, event).await?
    else {
        return Ok(false);
    };
    Ok(accessible_channels.contains(&parent_channel_id))
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Kind, Tag};

    use super::thread_root_event_id;

    fn signed_event(tags: Vec<Tag>) -> nostr::Event {
        EventBuilder::new(Kind::Custom(9), "reply")
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .expect("sign")
    }

    #[test]
    fn thread_root_prefers_root_marker() {
        let root = "a".repeat(64);
        let reply = "b".repeat(64);
        let event = signed_event(vec![
            Tag::parse(["e", &root, "", "root"]).expect("root tag"),
            Tag::parse(["e", &reply, "", "reply"]).expect("reply tag"),
        ]);

        assert_eq!(thread_root_event_id(&event), Some(vec![0xaa; 32]));
    }

    #[test]
    fn thread_root_falls_back_to_reply_marker() {
        let reply = "b".repeat(64);
        let event = signed_event(vec![Tag::parse(["e", &reply, "", "reply"]).expect("tag")]);

        assert_eq!(thread_root_event_id(&event), Some(vec![0xbb; 32]));
    }

    #[test]
    fn thread_root_rejects_root_marker_without_reply_marker() {
        let root = "a".repeat(64);
        let event = signed_event(vec![Tag::parse(["e", &root, "", "root"]).expect("tag")]);

        assert_eq!(thread_root_event_id(&event), None);
    }
}

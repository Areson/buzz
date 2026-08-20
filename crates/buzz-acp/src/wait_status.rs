use std::collections::{HashMap, HashSet};

use nostr::Event;
use uuid::Uuid;

use crate::observer;
use crate::queue::{self, ThreadTags};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WaitAction {
    Start,
    End,
}

#[derive(Clone, Debug)]
pub(crate) struct WaitMarker {
    pub action: WaitAction,
    pub channel_id: Uuid,
    pub task_id: String,
    pub thread_tags: ThreadTags,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct WaitKey {
    channel_id: Uuid,
    task_id: String,
}

#[derive(Clone, Debug)]
struct WaitEntry {
    channel_id: Uuid,
    task_id: String,
    thread_tags: ThreadTags,
    turn_id: String,
}

#[derive(Default)]
pub(crate) struct BackgroundWaits {
    waits: HashMap<WaitKey, WaitEntry>,
}

impl BackgroundWaits {
    pub fn apply_marker(
        &mut self,
        marker: WaitMarker,
        observer: Option<&observer::ObserverHandle>,
    ) -> WaitApplyOutcome {
        match marker.action {
            WaitAction::Start => self.start(marker, observer),
            WaitAction::End => {
                if self.end(marker.channel_id, &marker.task_id, observer) {
                    WaitApplyOutcome::Ended
                } else {
                    WaitApplyOutcome::UnknownEnd
                }
            }
        }
    }

    pub fn emit_liveness(&self, observer: Option<&observer::ObserverHandle>) {
        let Some(observer) = observer else {
            return;
        };
        for wait in self.waits.values() {
            observer.emit(
                "turn_liveness",
                None,
                &observer::context_for(Some(wait.channel_id), None, Some(wait.turn_id.clone())),
                serde_json::json!({
                    "source": "background_wait",
                    "taskId": wait.task_id,
                }),
            );
        }
    }

    pub fn typing_scopes(&self) -> Vec<(Uuid, ThreadTags)> {
        let mut seen = HashSet::new();
        let mut scopes = Vec::new();
        for wait in self.waits.values() {
            let key = (
                wait.channel_id,
                wait.thread_tags.root_event_id.clone(),
                wait.thread_tags.parent_event_id.clone(),
            );
            if seen.insert(key) {
                scopes.push((wait.channel_id, wait.thread_tags.clone()));
            }
        }
        scopes
    }

    #[cfg(test)]
    pub fn has_channel(&self, channel_id: Uuid) -> bool {
        self.waits.keys().any(|key| key.channel_id == channel_id)
    }

    pub fn clear_channel(
        &mut self,
        channel_id: Uuid,
        observer: Option<&observer::ObserverHandle>,
    ) -> usize {
        let keys: Vec<WaitKey> = self
            .waits
            .keys()
            .filter(|key| key.channel_id == channel_id)
            .cloned()
            .collect();
        let count = keys.len();
        for key in keys {
            if let Some(wait) = self.waits.remove(&key) {
                emit_completed(&wait, observer);
            }
        }
        count
    }

    pub fn clear_all(&mut self, observer: Option<&observer::ObserverHandle>) -> usize {
        let count = self.waits.len();
        for (_, wait) in self.waits.drain() {
            emit_completed(&wait, observer);
        }
        count
    }

    fn start(
        &mut self,
        marker: WaitMarker,
        observer: Option<&observer::ObserverHandle>,
    ) -> WaitApplyOutcome {
        let key = WaitKey {
            channel_id: marker.channel_id,
            task_id: marker.task_id.clone(),
        };
        if self.waits.contains_key(&key) {
            return WaitApplyOutcome::DuplicateStart;
        }

        let wait = WaitEntry {
            channel_id: marker.channel_id,
            task_id: marker.task_id,
            thread_tags: marker.thread_tags,
            turn_id: synthetic_turn_id(marker.channel_id, &key.task_id),
        };
        emit_started(&wait, observer);
        self.waits.insert(key, wait);
        WaitApplyOutcome::Started
    }

    fn end(
        &mut self,
        channel_id: Uuid,
        task_id: &str,
        observer: Option<&observer::ObserverHandle>,
    ) -> bool {
        let key = WaitKey {
            channel_id,
            task_id: task_id.to_string(),
        };
        let Some(wait) = self.waits.remove(&key) else {
            return false;
        };
        emit_completed(&wait, observer);
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WaitApplyOutcome {
    Started,
    DuplicateStart,
    Ended,
    UnknownEnd,
}

pub(crate) fn parse_wait_marker(
    event: &Event,
    channel_id: Uuid,
    expected_agent_pubkey_hex: &str,
) -> Option<WaitMarker> {
    if event.pubkey.to_hex() != expected_agent_pubkey_hex {
        return None;
    }

    let mut action = None;
    let mut task_id = None;
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        match parts.first().map(|s| s.as_str()) {
            Some("wait") if parts.len() >= 2 => {
                action = match parts[1].as_str() {
                    "start" => Some(WaitAction::Start),
                    "end" => Some(WaitAction::End),
                    _ => None,
                };
            }
            Some("task") if parts.len() >= 2 => {
                task_id = Some(parts[1].clone());
            }
            _ => {}
        }
    }

    let task_id = task_id?;
    if task_id.is_empty() {
        return None;
    }

    Some(WaitMarker {
        action: action?,
        channel_id,
        task_id,
        thread_tags: queue::parse_thread_tags(event),
    })
}

fn synthetic_turn_id(channel_id: Uuid, task_id: &str) -> String {
    format!("background-wait:{channel_id}:{task_id}")
}

fn emit_started(wait: &WaitEntry, observer: Option<&observer::ObserverHandle>) {
    let Some(observer) = observer else {
        return;
    };
    observer.emit(
        "turn_started",
        None,
        &observer::context_for(Some(wait.channel_id), None, Some(wait.turn_id.clone())),
        serde_json::json!({
            "source": "background_wait",
            "taskId": wait.task_id,
        }),
    );
}

fn emit_completed(wait: &WaitEntry, observer: Option<&observer::ObserverHandle>) {
    let Some(observer) = observer else {
        return;
    };
    observer.emit(
        "turn_completed",
        None,
        &observer::context_for(Some(wait.channel_id), None, Some(wait.turn_id.clone())),
        serde_json::json!({
            "source": "background_wait",
            "taskId": wait.task_id,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn marker_event(
        keys: &Keys,
        channel_id: Uuid,
        action: &str,
        task_id: &str,
        thread_root: Option<&str>,
    ) -> Event {
        let mut tags = vec![
            Tag::parse(["h", &channel_id.to_string()]).unwrap(),
            Tag::parse(["wait", action]).unwrap(),
            Tag::parse(["task", task_id]).unwrap(),
        ];
        if let Some(root) = thread_root {
            tags.push(Tag::parse(["e", root, "", "reply"]).unwrap());
        }
        EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_AGENT_WAIT_STATUS as u16),
            "",
        )
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap()
    }

    #[test]
    fn parse_marker_requires_agent_signature() {
        let agent_keys = Keys::generate();
        let other_keys = Keys::generate();
        let channel_id = Uuid::new_v4();
        let event = marker_event(&other_keys, channel_id, "start", "task-1", None);

        assert!(parse_wait_marker(&event, channel_id, &agent_keys.public_key().to_hex()).is_none());
    }

    #[test]
    fn parse_marker_preserves_thread_scope() {
        let keys = Keys::generate();
        let channel_id = Uuid::new_v4();
        let root = "a".repeat(64);
        let event = marker_event(&keys, channel_id, "start", "task-1", Some(&root));

        let marker = parse_wait_marker(&event, channel_id, &keys.public_key().to_hex()).unwrap();

        assert_eq!(marker.action, WaitAction::Start);
        assert_eq!(marker.task_id, "task-1");
        assert_eq!(
            marker.thread_tags.root_event_id.as_deref(),
            Some(root.as_str())
        );
        assert_eq!(
            marker.thread_tags.parent_event_id.as_deref(),
            Some(root.as_str())
        );
    }

    #[test]
    fn duplicate_start_is_idempotent() {
        let observer = observer::ObserverHandle::in_process();
        let channel_id = Uuid::new_v4();
        let mut waits = BackgroundWaits::default();
        let marker = WaitMarker {
            action: WaitAction::Start,
            channel_id,
            task_id: "task-1".into(),
            thread_tags: ThreadTags::default(),
        };

        assert_eq!(
            waits.apply_marker(marker.clone(), Some(&observer)),
            WaitApplyOutcome::Started
        );
        assert_eq!(
            waits.apply_marker(marker, Some(&observer)),
            WaitApplyOutcome::DuplicateStart
        );

        let events = observer.snapshot();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == "turn_started")
                .count(),
            1
        );
    }

    #[test]
    fn end_clears_only_matching_task() {
        let observer = observer::ObserverHandle::in_process();
        let channel_id = Uuid::new_v4();
        let mut waits = BackgroundWaits::default();
        for task_id in ["task-1", "task-2"] {
            waits.apply_marker(
                WaitMarker {
                    action: WaitAction::Start,
                    channel_id,
                    task_id: task_id.into(),
                    thread_tags: ThreadTags::default(),
                },
                Some(&observer),
            );
        }

        assert!(waits.end(channel_id, "task-1", Some(&observer)));

        assert!(waits.has_channel(channel_id));
        let events = observer.snapshot();
        let completed: Vec<&observer::ObserverEvent> = events
            .iter()
            .filter(|event| event.kind == "turn_completed")
            .collect();
        assert_eq!(completed.len(), 1);
        assert!(completed[0]
            .turn_id
            .as_deref()
            .unwrap_or_default()
            .ends_with(":task-1"));
    }

    #[test]
    fn unknown_end_is_noop() {
        let observer = observer::ObserverHandle::in_process();
        let channel_id = Uuid::new_v4();
        let mut waits = BackgroundWaits::default();

        assert_eq!(
            waits.apply_marker(
                WaitMarker {
                    action: WaitAction::End,
                    channel_id,
                    task_id: "ghost".into(),
                    thread_tags: ThreadTags::default(),
                },
                Some(&observer),
            ),
            WaitApplyOutcome::UnknownEnd
        );
        assert!(observer.snapshot().is_empty());
    }

    #[test]
    fn liveness_uses_synthetic_turn_id() {
        let observer = observer::ObserverHandle::in_process();
        let channel_id = Uuid::new_v4();
        let mut waits = BackgroundWaits::default();
        waits.apply_marker(
            WaitMarker {
                action: WaitAction::Start,
                channel_id,
                task_id: "task-1".into(),
                thread_tags: ThreadTags::default(),
            },
            Some(&observer),
        );

        waits.emit_liveness(Some(&observer));

        let events = observer.snapshot();
        let liveness = events
            .iter()
            .find(|event| event.kind == "turn_liveness")
            .expect("liveness emitted");
        let expected_channel_id = channel_id.to_string();
        assert_eq!(
            liveness.channel_id.as_deref(),
            Some(expected_channel_id.as_str())
        );
        let expected_turn_id = format!("background-wait:{channel_id}:task-1");
        assert_eq!(liveness.turn_id.as_deref(), Some(expected_turn_id.as_str()));
    }

    #[test]
    fn typing_scopes_deduplicate_by_channel_and_thread() {
        let channel_id = Uuid::new_v4();
        let mut waits = BackgroundWaits::default();
        let thread_tags = ThreadTags {
            root_event_id: Some("root".into()),
            parent_event_id: Some("root".into()),
            mentioned_pubkeys: Vec::new(),
        };
        for task_id in ["task-1", "task-2"] {
            waits.apply_marker(
                WaitMarker {
                    action: WaitAction::Start,
                    channel_id,
                    task_id: task_id.into(),
                    thread_tags: thread_tags.clone(),
                },
                None,
            );
        }

        let scopes = waits.typing_scopes();
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].0, channel_id);
        assert_eq!(scopes[0].1.root_event_id.as_deref(), Some("root"));
    }

    #[test]
    fn clear_channel_emits_completion_for_each_matching_wait() {
        let observer = observer::ObserverHandle::in_process();
        let channel_id = Uuid::new_v4();
        let other_channel_id = Uuid::new_v4();
        let mut waits = BackgroundWaits::default();
        for (channel_id, task_id) in [
            (channel_id, "task-1"),
            (channel_id, "task-2"),
            (other_channel_id, "task-3"),
        ] {
            waits.apply_marker(
                WaitMarker {
                    action: WaitAction::Start,
                    channel_id,
                    task_id: task_id.into(),
                    thread_tags: ThreadTags::default(),
                },
                Some(&observer),
            );
        }

        assert_eq!(waits.clear_channel(channel_id, Some(&observer)), 2);

        assert!(!waits.has_channel(channel_id));
        assert!(waits.has_channel(other_channel_id));
        let completed_turn_ids: HashSet<String> = observer
            .snapshot()
            .iter()
            .filter(|event| event.kind == "turn_completed")
            .filter_map(|event| event.turn_id.clone())
            .collect();
        assert_eq!(completed_turn_ids.len(), 2);
        assert!(completed_turn_ids.contains(&format!("background-wait:{channel_id}:task-1")));
        assert!(completed_turn_ids.contains(&format!("background-wait:{channel_id}:task-2")));
    }

    #[test]
    fn clear_all_emits_completion_for_every_wait() {
        let observer = observer::ObserverHandle::in_process();
        let channel_id = Uuid::new_v4();
        let other_channel_id = Uuid::new_v4();
        let mut waits = BackgroundWaits::default();
        for (channel_id, task_id) in [(channel_id, "task-1"), (other_channel_id, "task-2")] {
            waits.apply_marker(
                WaitMarker {
                    action: WaitAction::Start,
                    channel_id,
                    task_id: task_id.into(),
                    thread_tags: ThreadTags::default(),
                },
                Some(&observer),
            );
        }

        assert_eq!(waits.clear_all(Some(&observer)), 2);

        assert!(!waits.has_channel(channel_id));
        assert!(!waits.has_channel(other_channel_id));
        let completed_count = observer
            .snapshot()
            .iter()
            .filter(|event| event.kind == "turn_completed")
            .count();
        assert_eq!(completed_count, 2);
    }

    #[test]
    fn liveness_without_explicit_end_keeps_wait_visible() {
        let observer = observer::ObserverHandle::in_process();
        let channel_id = Uuid::new_v4();
        let mut waits = BackgroundWaits::default();
        waits.apply_marker(
            WaitMarker {
                action: WaitAction::Start,
                channel_id,
                task_id: "task-1".into(),
                thread_tags: ThreadTags::default(),
            },
            Some(&observer),
        );

        waits.emit_liveness(Some(&observer));

        assert!(waits.has_channel(channel_id));
        let events = observer.snapshot();
        assert!(events.iter().any(|event| event.kind == "turn_liveness"));
        assert!(!events.iter().any(|event| event.kind == "turn_completed"));
    }
}

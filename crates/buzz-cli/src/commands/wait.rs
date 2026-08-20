use crate::client::{normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::{parse_uuid, validate_hex64};

/// Maximum length for an agent background wait task id.
const MAX_TASK_ID_LEN: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaitAction {
    Start,
    End,
}

impl WaitAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::End => "end",
        }
    }
}

fn validate_task_id(task_id: &str) -> Result<(), CliError> {
    if task_id.is_empty() {
        return Err(CliError::Usage("--task-id must not be empty".into()));
    }
    if task_id.len() > MAX_TASK_ID_LEN {
        return Err(CliError::Usage(format!(
            "--task-id must be at most {MAX_TASK_ID_LEN} bytes"
        )));
    }
    if task_id.chars().any(char::is_control) {
        return Err(CliError::Usage(
            "--task-id must not contain control characters".into(),
        ));
    }
    Ok(())
}

fn build_wait_marker_event(
    channel_id: uuid::Uuid,
    action: WaitAction,
    task_id: &str,
    thread_root: Option<&str>,
) -> Result<nostr::EventBuilder, CliError> {
    validate_task_id(task_id)?;
    if let Some(root) = thread_root {
        validate_hex64(root)?;
    }

    use nostr::{EventBuilder, Kind, Tag};

    let mut tags = vec![
        Tag::parse(["h", &channel_id.to_string()])
            .map_err(|e| CliError::Other(format!("tag error: {e}")))?,
        Tag::parse(["wait", action.as_str()])
            .map_err(|e| CliError::Other(format!("tag error: {e}")))?,
        Tag::parse(["task", task_id]).map_err(|e| CliError::Other(format!("tag error: {e}")))?,
    ];

    if let Some(root) = thread_root {
        tags.push(
            Tag::parse(["e", root, "", "reply"])
                .map_err(|e| CliError::Other(format!("tag error: {e}")))?,
        );
    }

    let content = serde_json::json!({
        "type": "background_wait",
        "action": action.as_str(),
        "taskId": task_id,
    })
    .to_string();

    Ok(EventBuilder::new(
        Kind::Custom(buzz_core::kind::KIND_AGENT_WAIT_STATUS as u16),
        content,
    )
    .tags(tags))
}

async fn cmd_wait(
    client: &BuzzClient,
    action: WaitAction,
    channel: &str,
    task_id: &str,
    thread_root: Option<&str>,
) -> Result<(), CliError> {
    let channel_id = parse_uuid(channel)?;
    let builder = build_wait_marker_event(channel_id, action, task_id, thread_root)?;
    let event = client.sign_event(builder)?;
    let resp = client.publish_ephemeral_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn dispatch(cmd: crate::WaitCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::WaitCmd;
    match cmd {
        WaitCmd::Start {
            channel,
            task_id,
            thread_root,
        } => {
            cmd_wait(
                client,
                WaitAction::Start,
                &channel,
                &task_id,
                thread_root.as_deref(),
            )
            .await
        }
        WaitCmd::End {
            channel,
            task_id,
            thread_root,
        } => {
            cmd_wait(
                client,
                WaitAction::End,
                &channel,
                &task_id,
                thread_root.as_deref(),
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    #[test]
    fn wait_marker_event_carries_scope_action_and_task_tags() {
        let channel_id = uuid::Uuid::new_v4();
        let keys = Keys::generate();
        let root = "a".repeat(64);
        let task_id = "b09rghn1p";
        let event = build_wait_marker_event(channel_id, WaitAction::Start, task_id, Some(&root))
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();

        assert_eq!(
            event.kind.as_u16() as u32,
            buzz_core::kind::KIND_AGENT_WAIT_STATUS
        );
        let tags: Vec<Vec<String>> = event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect();
        let channel_id_s = channel_id.to_string();
        assert!(has_tag_value(&tags, "h", &channel_id_s));
        assert!(has_tag_value(&tags, "wait", "start"));
        assert!(has_tag_value(&tags, "task", task_id));
        assert!(tags
            .iter()
            .any(|tag| tag.first().map(String::as_str) == Some("e")
                && tag.get(1).map(String::as_str) == Some(root.as_str())
                && tag.get(3).map(String::as_str) == Some("reply")));
    }

    #[test]
    fn wait_marker_rejects_empty_task_id() {
        let channel_id = uuid::Uuid::new_v4();
        let err = build_wait_marker_event(channel_id, WaitAction::Start, "", None).unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
    }

    fn has_tag_value(tags: &[Vec<String>], name: &str, value: &str) -> bool {
        tags.iter().any(|tag| {
            tag.first().map(String::as_str) == Some(name)
                && tag.get(1).map(String::as_str) == Some(value)
        })
    }
}

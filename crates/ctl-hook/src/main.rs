use std::io::{self, Read};
use std::{env, fs, io::ErrorKind, path::PathBuf};

use ctl_core::registration::{register_user_hooks, user_settings_path};
use ctl_core::storage::{apply_event, now_secs, state_path};
use ctl_core::{SessionEvent, SessionEventKind};
use serde_json::Value;

const ACTIVE_SESSION_TIMEOUT_SECS: u64 = 2 * 60 * 60;

fn main() {
    if env::args().nth(1).as_deref() == Some("install") {
        if let Err(error) = install() {
            eprintln!("ctl-hook install failed: {error}");
            std::process::exit(1);
        }
        return;
    }

    let mut payload = Vec::new();
    if io::stdin().read_to_end(&mut payload).is_err() {
        return;
    }

    let Ok(payload) = serde_json::from_slice::<Value>(&payload) else {
        return;
    };
    let Some(event) = event_from_payload(&payload, now_secs()) else {
        return;
    };
    let Ok(path) = state_path() else {
        return;
    };

    let _ = apply_event(&path, event, ACTIVE_SESSION_TIMEOUT_SECS);
}

fn install() -> io::Result<()> {
    let source = env::current_exe()?;
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "home directory is unavailable"))?;
    let destination = PathBuf::from(home)
        .join(".claude-traffic-light")
        .join("bin")
        .join(if cfg!(windows) {
            "ctl-hook.exe"
        } else {
            "ctl-hook"
        });

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let same_file = source
        .canonicalize()
        .ok()
        .zip(destination.canonicalize().ok())
        .is_some_and(|(source, destination)| source == destination);
    if !same_file {
        fs::copy(&source, &destination)?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o755))?;
    }

    register_user_hooks(&user_settings_path()?, &destination)?;
    println!(
        "Installed Claude Traffic Light hook at {}",
        destination.display()
    );
    Ok(())
}

fn event_from_payload(payload: &Value, timestamp_secs: u64) -> Option<SessionEvent> {
    let event_name = payload.get("hook_event_name")?.as_str()?;
    let session_id = payload.get("session_id")?.as_str()?;

    let kind = match event_name {
        "SessionStart" => SessionEventKind::SessionStart {
            cwd: payload.get("cwd")?.as_str()?.to_owned(),
        },
        "SessionEnd" => SessionEventKind::SessionEnd,
        "UserPromptSubmit" => SessionEventKind::UserPromptSubmit,
        "PreToolUse" => {
            let tool_name = payload.get("tool_name")?.as_str()?.to_owned();
            if let Some(agent_id) = payload.get("agent_id").and_then(Value::as_str) {
                SessionEventKind::SubagentPreToolUse {
                    agent_id: agent_id.to_owned(),
                    tool_name,
                }
            } else {
                SessionEventKind::PreToolUse { tool_name }
            }
        }
        "PostToolUse" => SessionEventKind::PostToolUse,
        "PermissionRequest" => SessionEventKind::PermissionRequest,
        "Stop" => SessionEventKind::Stop,
        "StopFailure" => SessionEventKind::StopFailure,
        "PostModelSwitch" => SessionEventKind::PostModelSwitch {
            model: payload.get("to_model")?.as_str()?.to_owned(),
            effort: payload
                .get("effort")
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
        "SubagentStart" => SessionEventKind::SubagentStart {
            agent_id: payload.get("agent_id")?.as_str()?.to_owned(),
            agent_type: payload.get("agent_type")?.as_str()?.to_owned(),
        },
        "SubagentStop" => SessionEventKind::SubagentStop {
            agent_id: payload.get("agent_id")?.as_str()?.to_owned(),
        },
        _ => return None,
    };

    Some(SessionEvent {
        session_id: session_id.to_owned(),
        timestamp_secs,
        kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_valid_session_start_payload() {
        let payload = serde_json::json!({
            "hook_event_name": "SessionStart",
            "session_id": "session-a",
            "cwd": "/work/project"
        });

        let event = event_from_payload(&payload, 42).unwrap();
        assert_eq!(event.session_id, "session-a");
        assert_eq!(event.timestamp_secs, 42);
        assert_eq!(
            event.kind,
            SessionEventKind::SessionStart {
                cwd: "/work/project".to_owned()
            }
        );
    }

    #[test]
    fn malformed_and_unrecognized_payloads_are_ignored() {
        assert!(event_from_payload(&Value::Null, 42).is_none());
        assert!(
            event_from_payload(
                &serde_json::json!({
                    "hook_event_name": "PreToolUse",
                    "session_id": "session-a"
                }),
                42
            )
            .is_none()
        );
        assert!(
            event_from_payload(
                &serde_json::json!({
                    "hook_event_name": "SomeFutureEvent",
                    "session_id": "session-a"
                }),
                42
            )
            .is_none()
        );
    }

    #[test]
    fn maps_subagent_tool_events_without_parent_tool_event() {
        let payload = serde_json::json!({
            "hook_event_name": "PreToolUse",
            "session_id": "parent-session",
            "agent_id": "agent-1",
            "tool_name": "Read"
        });

        assert_eq!(
            event_from_payload(&payload, 42).unwrap().kind,
            SessionEventKind::SubagentPreToolUse {
                agent_id: "agent-1".to_owned(),
                tool_name: "Read".to_owned()
            }
        );
    }
}

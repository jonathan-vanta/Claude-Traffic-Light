use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub mod prefs;
pub mod registration;
pub mod storage;

pub const STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Light {
    Dark,
    Green,
    Yellow,
    Red,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    #[default]
    Idle,
    Working,
    WaitingPermission,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SubagentState {
    pub agent_type: String,
    pub current_tool: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionState {
    pub session_id: String,
    pub cwd: String,
    pub status: SessionStatus,
    pub last_event_at_secs: u64,
    pub active_tool: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<String>,
    pub subagents: BTreeMap<String, SubagentState>,
}

impl SessionState {
    fn new(session_id: String, cwd: String, timestamp_secs: u64) -> Self {
        Self {
            session_id,
            cwd,
            status: SessionStatus::Idle,
            last_event_at_secs: timestamp_secs,
            active_tool: None,
            model: None,
            effort: None,
            permission_mode: None,
            subagents: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEventKind {
    SessionStart {
        cwd: String,
    },
    SessionEnd,
    UserPromptSubmit,
    PreToolUse {
        tool_name: String,
    },
    PostToolUse,
    PermissionRequest,
    Stop,
    StopFailure,
    PostModelSwitch {
        model: String,
        effort: Option<String>,
    },
    SubagentStart {
        agent_id: String,
        agent_type: String,
    },
    SubagentStop {
        agent_id: String,
    },
    SubagentPreToolUse {
        agent_id: String,
        tool_name: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionEvent {
    pub session_id: String,
    pub timestamp_secs: u64,
    pub kind: SessionEventKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionStore {
    pub schema_version: u32,
    pub sessions: BTreeMap<String, SessionState>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub cwd: String,
    pub status: SessionStatus,
    pub age_secs: u64,
    pub active_tool: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub subagent_count: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StateSnapshot {
    pub light: Light,
    pub active_session_count: usize,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            sessions: BTreeMap::new(),
        }
    }
}

impl SessionStore {
    pub fn apply(&mut self, event: SessionEvent) {
        if let SessionEventKind::SessionStart { cwd } = &event.kind {
            let mut session =
                SessionState::new(event.session_id.clone(), cwd.clone(), event.timestamp_secs);
            if let Some(previous) = self.sessions.get(&event.session_id) {
                session.model.clone_from(&previous.model);
                session.effort.clone_from(&previous.effort);
                session
                    .permission_mode
                    .clone_from(&previous.permission_mode);
            }
            self.sessions.insert(event.session_id, session);
            return;
        }

        if matches!(event.kind, SessionEventKind::SessionEnd) {
            self.sessions.remove(&event.session_id);
            return;
        }

        let Some(session) = self.sessions.get_mut(&event.session_id) else {
            return;
        };

        session.last_event_at_secs = event.timestamp_secs;
        match event.kind {
            SessionEventKind::UserPromptSubmit => {
                session.status = SessionStatus::Working;
                session.active_tool = None;
            }
            SessionEventKind::PreToolUse { tool_name } => {
                session.status = SessionStatus::Working;
                session.active_tool = Some(tool_name);
            }
            SessionEventKind::PostToolUse => {
                session.status = SessionStatus::Working;
                session.active_tool = None;
            }
            SessionEventKind::PermissionRequest => {
                session.status = SessionStatus::WaitingPermission;
                session.active_tool = None;
            }
            SessionEventKind::Stop | SessionEventKind::StopFailure => {
                session.status = SessionStatus::Idle;
                session.active_tool = None;
            }
            SessionEventKind::PostModelSwitch { model, effort } => {
                session.model = Some(model);
                session.effort = effort;
            }
            SessionEventKind::SubagentStart {
                agent_id,
                agent_type,
            } => {
                session.subagents.insert(
                    agent_id,
                    SubagentState {
                        agent_type,
                        current_tool: None,
                    },
                );
            }
            SessionEventKind::SubagentStop { agent_id } => {
                session.subagents.remove(&agent_id);
            }
            SessionEventKind::SubagentPreToolUse {
                agent_id,
                tool_name,
            } => {
                if let Some(agent) = session.subagents.get_mut(&agent_id) {
                    agent.current_tool = Some(tool_name);
                }
            }
            SessionEventKind::SessionStart { .. } | SessionEventKind::SessionEnd => {}
        }
    }

    pub fn prune_stale(&mut self, now_secs: u64, timeout_secs: u64) {
        self.sessions.retain(|_, session| {
            now_secs.saturating_sub(session.last_event_at_secs) <= timeout_secs
        });
    }

    pub fn aggregate_light(&self, now_secs: u64, timeout_secs: u64) -> Light {
        let mut has_session = false;
        let mut result = Light::Green;

        for session in self
            .sessions
            .values()
            .filter(|session| now_secs.saturating_sub(session.last_event_at_secs) <= timeout_secs)
        {
            has_session = true;
            match session.status {
                SessionStatus::WaitingPermission => return Light::Red,
                SessionStatus::Working => result = Light::Yellow,
                SessionStatus::Idle => {}
            }
        }

        if has_session { result } else { Light::Dark }
    }

    /// Active sessions, most recently updated first. Idle ones are optional.
    pub fn summaries(
        &self,
        now_secs: u64,
        timeout_secs: u64,
        include_idle: bool,
    ) -> Vec<SessionSummary> {
        let mut rows: Vec<SessionSummary> = self
            .sessions
            .values()
            .filter(|session| now_secs.saturating_sub(session.last_event_at_secs) <= timeout_secs)
            .filter(|session| include_idle || session.status != SessionStatus::Idle)
            .map(|session| SessionSummary {
                session_id: session.session_id.clone(),
                cwd: session.cwd.clone(),
                status: session.status,
                age_secs: now_secs.saturating_sub(session.last_event_at_secs),
                active_tool: session.active_tool.clone(),
                model: session.model.clone(),
                effort: session.effort.clone(),
                subagent_count: session.subagents.len(),
            })
            .collect();
        rows.sort_by_key(|row| row.age_secs);
        rows
    }

    pub fn snapshot(&self, now_secs: u64, timeout_secs: u64) -> StateSnapshot {
        StateSnapshot {
            light: self.aggregate_light(now_secs, timeout_secs),
            active_session_count: self
                .sessions
                .values()
                .filter(|session| {
                    now_secs.saturating_sub(session.last_event_at_secs) <= timeout_secs
                })
                .count(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(session_id: &str, timestamp_secs: u64, kind: SessionEventKind) -> SessionEvent {
        SessionEvent {
            session_id: session_id.to_owned(),
            timestamp_secs,
            kind,
        }
    }

    fn started_store(session_id: &str, timestamp_secs: u64) -> SessionStore {
        let mut store = SessionStore::default();
        store.apply(event(
            session_id,
            timestamp_secs,
            SessionEventKind::SessionStart {
                cwd: "/work/project".to_owned(),
            },
        ));
        store
    }

    #[test]
    fn aggregates_idle_working_and_permission_states() {
        let mut store = started_store("one", 100);
        assert_eq!(store.aggregate_light(100, 900), Light::Green);
        store.apply(event("one", 101, SessionEventKind::Stop));
        assert_eq!(store.aggregate_light(101, 900), Light::Green);

        store.apply(event(
            "one",
            102,
            SessionEventKind::PreToolUse {
                tool_name: "Read".to_owned(),
            },
        ));
        assert_eq!(store.aggregate_light(102, 900), Light::Yellow);

        store.apply(event("one", 103, SessionEventKind::PermissionRequest));
        assert_eq!(store.aggregate_light(103, 900), Light::Red);

        store.apply(event("one", 104, SessionEventKind::PostToolUse));
        assert_eq!(store.aggregate_light(104, 900), Light::Yellow);
    }

    #[test]
    fn red_beats_yellow_and_green_across_sessions() {
        let mut store = started_store("idle", 100);
        store.apply(event("idle", 101, SessionEventKind::Stop));
        store.apply(event(
            "working",
            101,
            SessionEventKind::SessionStart {
                cwd: "/work/other".to_owned(),
            },
        ));
        store.apply(event("working", 102, SessionEventKind::UserPromptSubmit));
        assert_eq!(store.aggregate_light(102, 900), Light::Yellow);

        store.apply(event("waiting", 102, SessionEventKind::PermissionRequest));
        store.apply(event(
            "waiting",
            101,
            SessionEventKind::SessionStart {
                cwd: "/work/waiting".to_owned(),
            },
        ));
        store.apply(event("waiting", 102, SessionEventKind::PermissionRequest));
        assert_eq!(store.aggregate_light(102, 900), Light::Red);
    }

    #[test]
    fn stale_sessions_do_not_affect_light_and_can_be_pruned() {
        let mut store = started_store("old", 100);
        assert_eq!(store.aggregate_light(1_001, 900), Light::Dark);

        store.prune_stale(1_001, 900);
        assert!(store.sessions.is_empty());
    }

    #[test]
    fn subagent_events_do_not_change_parent_status_or_tool() {
        let mut store = started_store("parent", 100);
        store.apply(event(
            "parent",
            101,
            SessionEventKind::PreToolUse {
                tool_name: "Task".to_owned(),
            },
        ));
        store.apply(event(
            "parent",
            102,
            SessionEventKind::SubagentStart {
                agent_id: "agent-1".to_owned(),
                agent_type: "Explore".to_owned(),
            },
        ));
        store.apply(event(
            "parent",
            103,
            SessionEventKind::SubagentPreToolUse {
                agent_id: "agent-1".to_owned(),
                tool_name: "Grep".to_owned(),
            },
        ));

        let parent = &store.sessions["parent"];
        assert_eq!(parent.status, SessionStatus::Working);
        assert_eq!(parent.active_tool.as_deref(), Some("Task"));
        assert_eq!(
            parent.subagents["agent-1"].current_tool.as_deref(),
            Some("Grep")
        );

        store.apply(event(
            "parent",
            104,
            SessionEventKind::SubagentStop {
                agent_id: "agent-1".to_owned(),
            },
        ));
        assert!(store.sessions["parent"].subagents.is_empty());
    }

    #[test]
    fn summaries_filter_idle_and_stale_and_sort_by_recency() {
        let mut store = started_store("idle", 100);
        store.apply(event("idle", 101, SessionEventKind::Stop));
        store.apply(event(
            "busy",
            102,
            SessionEventKind::SessionStart {
                cwd: "/work/busy".to_owned(),
            },
        ));
        store.apply(event("busy", 105, SessionEventKind::UserPromptSubmit));

        let all = store.summaries(110, 900, true);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].session_id, "busy");

        let active_only = store.summaries(110, 900, false);
        assert_eq!(active_only.len(), 1);
        assert_eq!(active_only[0].session_id, "busy");

        assert!(store.summaries(2_000, 900, true).is_empty());
    }

    #[test]
    fn state_round_trips_as_json() {
        let store = started_store("session", 123);
        let encoded = serde_json::to_string(&store).unwrap();
        let decoded: SessionStore = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, store);
    }
}

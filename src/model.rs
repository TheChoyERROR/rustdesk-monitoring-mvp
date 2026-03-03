use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventType {
    SessionStarted,
    SessionEnded,
    RecordingStarted,
    RecordingStopped,
    ParticipantJoined,
    ParticipantLeft,
    ControlChanged,
    ParticipantActivity,
}

impl SessionEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SessionStarted => "session_started",
            Self::SessionEnded => "session_ended",
            Self::RecordingStarted => "recording_started",
            Self::RecordingStopped => "recording_stopped",
            Self::ParticipantJoined => "participant_joined",
            Self::ParticipantLeft => "participant_left",
            Self::ControlChanged => "control_changed",
            Self::ParticipantActivity => "participant_activity",
        }
    }

    pub fn affects_presence(&self) -> bool {
        matches!(
            self,
            Self::SessionEnded
                | Self::ParticipantJoined
                | Self::ParticipantLeft
                | Self::ControlChanged
                | Self::ParticipantActivity
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionDirection {
    Incoming,
    Outgoing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostInfo {
    pub hostname: String,
    pub os: String,
    pub app_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEventV1 {
    pub event_id: Uuid,
    pub event_type: SessionEventType,
    pub session_id: String,
    pub user_id: String,
    pub direction: SessionDirection,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_info: Option<HostInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceParticipantV1 {
    pub participant_id: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub is_active: bool,
    pub is_control_active: bool,
    pub last_activity_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionPresenceV1 {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_participant_id: Option<String>,
    pub participants: Vec<PresenceParticipantV1>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceSessionSummaryV1 {
    pub session_id: String,
    pub active_participants: u64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthRoleV1 {
    Supervisor,
}

impl AuthRoleV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supervisor => "supervisor",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserV1 {
    pub id: i64,
    pub username: String,
    pub role: AuthRoleV1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthLoginRequestV1 {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthLoginResponseV1 {
    pub user: AuthUserV1,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardSummaryV1 {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub events_total: u64,
    pub sessions_started: u64,
    pub sessions_ended: u64,
    pub active_sessions: u64,
    pub webhook_pending: u64,
    pub webhook_failed: u64,
    pub webhook_delivered: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTimelineItemV1 {
    pub event_id: Uuid,
    pub event_type: SessionEventType,
    pub session_id: String,
    pub user_id: String,
    pub direction: SessionDirection,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_info: Option<HostInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedResponseV1<T> {
    pub items: Vec<T>,
    pub page: u64,
    pub page_size: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionReportRowV1 {
    pub session_id: String,
    pub started_at: DateTime<Utc>,
    pub last_event_at: DateTime<Utc>,
    pub events_total: u64,
    pub users: Vec<String>,
}

impl SessionEventV1 {
    pub fn validate(&self) -> Result<(), EventValidationError> {
        if self.session_id.trim().is_empty() {
            return Err(EventValidationError::EmptyField("session_id"));
        }
        if self.user_id.trim().is_empty() {
            return Err(EventValidationError::EmptyField("user_id"));
        }
        if let Some(meta) = &self.meta {
            if !meta.is_object() {
                return Err(EventValidationError::MetaMustBeObject);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum EventValidationError {
    #[error("the field '{0}' cannot be empty")]
    EmptyField(&'static str),
    #[error("meta field must be a JSON object")]
    MetaMustBeObject,
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    use super::{SessionDirection, SessionEventType, SessionEventV1};

    #[test]
    fn invalid_meta_is_rejected() {
        let event = SessionEventV1 {
            event_id: Uuid::new_v4(),
            event_type: SessionEventType::SessionStarted,
            session_id: "sess-1".to_string(),
            user_id: "user-1".to_string(),
            direction: SessionDirection::Incoming,
            timestamp: Utc::now(),
            host_info: None,
            meta: Some(json!(["invalid", "meta"])),
        };

        assert!(event.validate().is_err());
    }
}

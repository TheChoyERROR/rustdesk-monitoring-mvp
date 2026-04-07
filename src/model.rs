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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionActorTypeV1 {
    Agent,
    Client,
    Unknown,
}

impl SessionActorTypeV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Client => "client",
            Self::Unknown => "unknown",
        }
    }
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
    pub actor_type: SessionActorTypeV1,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HelpdeskAgentStatus {
    Offline,
    Available,
    Opening,
    Busy,
    Away,
}

impl HelpdeskAgentStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Available => "available",
            Self::Opening => "opening",
            Self::Busy => "busy",
            Self::Away => "away",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HelpdeskTicketStatus {
    New,
    Queued,
    Assigned,
    Opening,
    InProgress,
    Resolved,
    Cancelled,
    Failed,
}

impl HelpdeskTicketStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Queued => "queued",
            Self::Assigned => "assigned",
            Self::Opening => "opening",
            Self::InProgress => "in_progress",
            Self::Resolved => "resolved",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpdeskAgentPresenceUpdateV1 {
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub status: HelpdeskAgentStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpdeskAgentV1 {
    pub agent_id: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub status: HelpdeskAgentStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_ticket_id: Option<String>,
    pub last_heartbeat_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpdeskAuthorizedAgentV1 {
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpdeskAuthorizedAgentUpsertRequestV1 {
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpdeskAgentAuthorizationStatusV1 {
    pub agent_id: String,
    pub authorized: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpdeskTicketCreateRequestV1 {
    pub client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub difficulty: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_minutes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpdeskTicketV1 {
    pub ticket_id: String,
    pub client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub difficulty: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_minutes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub status: HelpdeskTicketStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assigned_agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_agent_report: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_agent_report_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_agent_report_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opening_deadline_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpdeskAssignmentV1 {
    pub ticket: HelpdeskTicketV1,
    pub agent: HelpdeskAgentV1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpdeskAuditEventV1 {
    pub entity_type: String,
    pub entity_id: String,
    pub event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpdeskOperationalSummaryV1 {
    pub tickets_new: u64,
    pub tickets_queued: u64,
    pub tickets_opening: u64,
    pub tickets_in_progress: u64,
    pub tickets_resolved: u64,
    pub tickets_cancelled: u64,
    pub tickets_failed: u64,
    pub agents_offline: u64,
    pub agents_available: u64,
    pub agents_opening: u64,
    pub agents_busy: u64,
    pub agents_away: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpdeskAssignmentStartRequestV1 {
    pub ticket_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpdeskTicketAssignRequestV1 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpdeskTicketOperationalUpdateRequestV1 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub difficulty: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_minutes: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpdeskTicketAgentReportCreateRequestV1 {
    pub agent_id: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpdeskTicketResolveRequestV1 {
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_agent_status: Option<HelpdeskAgentStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpdeskTicketSupervisorActionRequestV1 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_agent_status: Option<HelpdeskAgentStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
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

impl HelpdeskAgentPresenceUpdateV1 {
    pub fn validate(&self) -> Result<(), EventValidationError> {
        if self.agent_id.trim().is_empty() {
            return Err(EventValidationError::EmptyField("agent_id"));
        }
        Ok(())
    }
}

impl HelpdeskAuthorizedAgentUpsertRequestV1 {
    pub fn validate(&self) -> Result<(), EventValidationError> {
        if self.agent_id.trim().is_empty() {
            return Err(EventValidationError::EmptyField("agent_id"));
        }
        if let Some(display_name) = &self.display_name {
            if display_name.trim().is_empty() {
                return Err(EventValidationError::EmptyField("display_name"));
            }
        }
        Ok(())
    }
}

impl HelpdeskTicketCreateRequestV1 {
    pub fn validate(&self) -> Result<(), EventValidationError> {
        if self.client_id.trim().is_empty() {
            return Err(EventValidationError::EmptyField("client_id"));
        }
        if let Some(title) = &self.title {
            if title.trim().is_empty() {
                return Err(EventValidationError::EmptyField("title"));
            }
        }
        if let Some(description) = &self.description {
            if description.trim().is_empty() {
                return Err(EventValidationError::EmptyField("description"));
            }
        }
        if let Some(difficulty) = &self.difficulty {
            if difficulty.trim().is_empty() {
                return Err(EventValidationError::EmptyField("difficulty"));
            }
        }
        if matches!(self.estimated_minutes, Some(0)) {
            return Err(EventValidationError::InvalidHelpdeskEstimatedMinutes);
        }
        if let Some(preferred_agent_id) = &self.preferred_agent_id {
            if preferred_agent_id.trim().is_empty() {
                return Err(EventValidationError::EmptyField("preferred_agent_id"));
            }
        }
        Ok(())
    }
}

impl HelpdeskAssignmentStartRequestV1 {
    pub fn validate(&self) -> Result<(), EventValidationError> {
        if self.ticket_id.trim().is_empty() {
            return Err(EventValidationError::EmptyField("ticket_id"));
        }
        Ok(())
    }
}

impl HelpdeskTicketAssignRequestV1 {
    pub fn validate(&self) -> Result<(), EventValidationError> {
        match self.agent_id.as_deref() {
            Some(agent_id) if !agent_id.trim().is_empty() => Ok(()),
            _ => Err(EventValidationError::EmptyField("agent_id")),
        }
    }
}

impl HelpdeskTicketOperationalUpdateRequestV1 {
    pub fn validate(&self) -> Result<(), EventValidationError> {
        if let Some(difficulty) = &self.difficulty {
            if difficulty.trim().is_empty() {
                return Err(EventValidationError::EmptyField("difficulty"));
            }
        }
        if matches!(self.estimated_minutes, Some(0)) {
            return Err(EventValidationError::InvalidHelpdeskEstimatedMinutes);
        }
        if self.difficulty.is_none() && self.estimated_minutes.is_none() {
            return Err(EventValidationError::EmptyHelpdeskOperationalUpdate);
        }
        Ok(())
    }
}

impl HelpdeskTicketAgentReportCreateRequestV1 {
    pub fn validate(&self) -> Result<(), EventValidationError> {
        if self.agent_id.trim().is_empty() {
            return Err(EventValidationError::EmptyField("agent_id"));
        }
        if self.note.trim().is_empty() {
            return Err(EventValidationError::EmptyField("note"));
        }
        Ok(())
    }
}

impl HelpdeskTicketResolveRequestV1 {
    pub fn validate(&self) -> Result<(), EventValidationError> {
        if self.agent_id.trim().is_empty() {
            return Err(EventValidationError::EmptyField("agent_id"));
        }
        if let Some(status) = self.next_agent_status {
            if !matches!(
                status,
                HelpdeskAgentStatus::Available | HelpdeskAgentStatus::Away
            ) {
                return Err(EventValidationError::InvalidHelpdeskAgentTerminalStatus);
            }
        }
        Ok(())
    }
}

impl HelpdeskTicketSupervisorActionRequestV1 {
    pub fn validate(&self) -> Result<(), EventValidationError> {
        if let Some(status) = self.next_agent_status {
            if !matches!(
                status,
                HelpdeskAgentStatus::Available | HelpdeskAgentStatus::Away
            ) {
                return Err(EventValidationError::InvalidHelpdeskAgentTerminalStatus);
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
    #[error("at least one operational ticket field must be provided")]
    EmptyHelpdeskOperationalUpdate,
    #[error("estimated_minutes must be greater than zero")]
    InvalidHelpdeskEstimatedMinutes,
    #[error("next_agent_status must be either 'available' or 'away'")]
    InvalidHelpdeskAgentTerminalStatus,
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

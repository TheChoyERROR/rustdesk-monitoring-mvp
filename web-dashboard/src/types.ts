export type AuthRole = 'supervisor';

export interface AuthUser {
  id: number;
  username: string;
  role: AuthRole;
}

export interface AuthLoginRequest {
  username: string;
  password: string;
}

export interface AuthLoginResponse {
  user: AuthUser;
  expires_at: string;
}

export interface DashboardSummary {
  from: string;
  to: string;
  events_total: number;
  sessions_started: number;
  sessions_ended: number;
  active_sessions: number;
  webhook_pending: number;
  webhook_failed: number;
  webhook_delivered: number;
}

export type SessionEventType =
  | 'session_started'
  | 'session_ended'
  | 'recording_started'
  | 'recording_stopped'
  | 'participant_joined'
  | 'participant_left'
  | 'control_changed'
  | 'participant_activity';

export interface SessionTimelineItem {
  event_id: string;
  event_type: SessionEventType;
  session_id: string;
  user_id: string;
  direction: 'incoming' | 'outgoing';
  timestamp: string;
  host_info?: {
    hostname: string;
    os: string;
    app_version: string;
  };
  meta?: Record<string, unknown>;
}

export interface PaginatedResponse<T> {
  items: T[];
  page: number;
  page_size: number;
  total: number;
}

export interface PresenceParticipant {
  participant_id: string;
  display_name: string;
  avatar_url?: string | null;
  is_active: boolean;
  is_control_active: boolean;
  last_activity_at: string;
}

export interface SessionPresence {
  session_id: string;
  control_participant_id?: string;
  participants: PresenceParticipant[];
  updated_at: string;
}

export interface PresenceSessionSummary {
  session_id: string;
  active_participants: number;
  updated_at: string;
}

export type HelpdeskAgentStatus = 'offline' | 'available' | 'opening' | 'busy' | 'away';

export type HelpdeskTicketStatus =
  | 'new'
  | 'queued'
  | 'opening'
  | 'in_progress'
  | 'resolved'
  | 'cancelled'
  | 'failed';

export interface HelpdeskAgent {
  agent_id: string;
  display_name: string;
  status: HelpdeskAgentStatus;
  current_ticket_id?: string | null;
  last_heartbeat_at: string;
  updated_at: string;
}

export interface HelpdeskTicket {
  ticket_id: string;
  client_id: string;
  client_display_name?: string | null;
  device_id?: string | null;
  requested_by?: string | null;
  summary?: string | null;
  status: HelpdeskTicketStatus;
  assigned_agent_id?: string | null;
  opening_deadline_at?: string | null;
  created_at: string;
  updated_at: string;
}

export interface HelpdeskOperationalSummary {
  tickets_new: number;
  tickets_queued: number;
  tickets_opening: number;
  tickets_in_progress: number;
  tickets_resolved: number;
  tickets_cancelled: number;
  tickets_failed: number;
  agents_offline: number;
  agents_available: number;
  agents_opening: number;
  agents_busy: number;
  agents_away: number;
}

export interface HelpdeskAuditEvent {
  entity_type: string;
  entity_id: string;
  event_type: string;
  payload?: Record<string, unknown> | null;
  created_at: string;
}

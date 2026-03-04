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

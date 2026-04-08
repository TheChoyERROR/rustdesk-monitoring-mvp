import type {
  AuthLoginRequest,
  AuthLoginResponse,
  DashboardSummary,
  HelpdeskAuditEvent,
  HelpdeskAgent,
  HelpdeskAuthorizedAgent,
  HelpdeskOperationalSummary,
  HelpdeskTicket,
  PaginatedResponse,
  PresenceSessionSummary,
  SessionPresence,
  SessionActorType,
  SessionTimelineItem,
  SessionEventType,
} from './types';

class ApiError extends Error {
  status: number;
  payload: unknown;

  constructor(message: string, status: number, payload: unknown) {
    super(message);
    this.status = status;
    this.payload = payload;
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    credentials: 'include',
    headers: {
      'Content-Type': 'application/json',
      ...(init?.headers ?? {}),
    },
  });

  const hasJson = response.headers.get('content-type')?.includes('application/json') ?? false;
  const payload = hasJson ? await response.json() : await response.text();

  if (!response.ok) {
    const message =
      typeof payload === 'object' && payload && 'message' in payload
        ? String((payload as { message: unknown }).message)
        : `HTTP ${response.status}`;
    throw new ApiError(message, response.status, payload);
  }

  return payload as T;
}

function buildQuery(params: Record<string, unknown>): string {
  const search = new URLSearchParams();
  Object.entries(params).forEach(([key, value]) => {
    if (typeof value === 'string' && value !== '') {
      search.set(key, value);
      return;
    }
    if (typeof value === 'number' && Number.isFinite(value)) {
      search.set(key, String(value));
    }
  });
  const output = search.toString();
  return output ? `?${output}` : '';
}

function helpdeskBasePath(): string {
  return '/api/v1/helpdesk';
}

export async function apiLogin(body: AuthLoginRequest): Promise<AuthLoginResponse> {
  return request<AuthLoginResponse>('/api/v1/auth/login', {
    method: 'POST',
    body: JSON.stringify(body),
  });
}

export async function apiLogout(): Promise<void> {
  await request<{ status: string }>('/api/v1/auth/logout', { method: 'POST' });
}

export async function apiMe(): Promise<AuthLoginResponse> {
  return request<AuthLoginResponse>('/api/v1/auth/me');
}

export async function apiSummary(from?: string, to?: string): Promise<DashboardSummary> {
  const query = buildQuery({ from, to });
  return request<DashboardSummary>(`/api/v1/dashboard/summary${query}`);
}

export interface EventsQuery {
  session_id?: string;
  user_id?: string;
  actor_type?: SessionActorType;
  event_type?: SessionEventType;
  from?: string;
  to?: string;
  page?: number;
  page_size?: number;
}

export async function apiEvents(query: EventsQuery): Promise<PaginatedResponse<SessionTimelineItem>> {
  return request<PaginatedResponse<SessionTimelineItem>>(
    `/api/v1/events${buildQuery(query as Record<string, unknown>)}`,
  );
}

export async function apiSessionTimeline(
  sessionId: string,
  page = 1,
  pageSize = 50,
  actorType?: SessionActorType,
): Promise<PaginatedResponse<SessionTimelineItem>> {
  return request<PaginatedResponse<SessionTimelineItem>>(
    `/api/v1/sessions/${encodeURIComponent(sessionId)}/timeline${buildQuery({
      actor_type: actorType,
      page,
      page_size: pageSize,
    })}`,
  );
}

export async function apiSessionPresence(sessionId: string): Promise<SessionPresence | null> {
  try {
    const response = await request<{ presence: SessionPresence }>(
      `/api/v1/sessions/${encodeURIComponent(sessionId)}/presence`,
    );
    return response.presence;
  } catch (error) {
    if (error instanceof ApiError && error.status === 404) {
      return null;
    }
    throw error;
  }
}

export async function apiPresenceSessions(): Promise<PresenceSessionSummary[]> {
  const response = await request<{ sessions: PresenceSessionSummary[] }>('/api/v1/sessions/presence');
  return response.sessions;
}

export async function apiHelpdeskSummary(): Promise<HelpdeskOperationalSummary> {
  return request<HelpdeskOperationalSummary>(`${helpdeskBasePath()}/summary`);
}

export async function apiHelpdeskAgents(): Promise<HelpdeskAgent[]> {
  const response = await request<{ agents: HelpdeskAgent[] }>(`${helpdeskBasePath()}/agents`);
  return response.agents;
}

export async function apiHelpdeskAuthorizedAgents(): Promise<HelpdeskAuthorizedAgent[]> {
  const response = await request<{ agents: HelpdeskAuthorizedAgent[] }>(
    `${helpdeskBasePath()}/agent-authorizations`,
  );
  return response.agents;
}

export async function apiHelpdeskTickets(): Promise<HelpdeskTicket[]> {
  const response = await request<{ tickets: HelpdeskTicket[] }>(`${helpdeskBasePath()}/tickets`);
  return response.tickets;
}

export interface HelpdeskAuthorizedAgentUpsertBody {
  agent_id: string;
  display_name?: string;
}

export async function apiHelpdeskAuthorizedAgentUpsert(
  body: HelpdeskAuthorizedAgentUpsertBody,
): Promise<HelpdeskAuthorizedAgent> {
  const response = await request<{ agent: HelpdeskAuthorizedAgent }>(
    `${helpdeskBasePath()}/agent-authorizations`,
    {
      method: 'POST',
      body: JSON.stringify(body),
    },
  );
  return response.agent;
}

export async function apiHelpdeskAuthorizedAgentDelete(agentId: string): Promise<void> {
  await request<null>(`${helpdeskBasePath()}/agent-authorizations/${encodeURIComponent(agentId)}`, {
    method: 'DELETE',
  });
}

export interface HelpdeskCreateTicketBody {
  client_id: string;
  client_display_name?: string;
  device_id?: string;
  requested_by?: string;
  title?: string;
  description?: string;
  difficulty?: string;
  estimated_minutes?: number;
  summary?: string;
  preferred_agent_id?: string;
}

export async function apiHelpdeskCreateTicket(body: HelpdeskCreateTicketBody): Promise<HelpdeskTicket> {
  const response = await request<{ ticket: HelpdeskTicket }>(`${helpdeskBasePath()}/tickets`, {
    method: 'POST',
    body: JSON.stringify(body),
  });
  return response.ticket;
}

export async function apiHelpdeskTicket(ticketId: string): Promise<HelpdeskTicket | null> {
  try {
    const response = await request<{ ticket: HelpdeskTicket }>(
      `${helpdeskBasePath()}/tickets/${encodeURIComponent(ticketId)}`,
    );
    return response.ticket;
  } catch (error) {
    if (error instanceof ApiError && error.status === 404) {
      return null;
    }
    throw error;
  }
}

export async function apiHelpdeskTicketAudit(
  ticketId: string,
  limit = 100,
): Promise<HelpdeskAuditEvent[]> {
  const response = await request<{ events: HelpdeskAuditEvent[] }>(
    `${helpdeskBasePath()}/tickets/${encodeURIComponent(ticketId)}/audit${buildQuery({ limit })}`,
  );
  return response.events;
}

interface HelpdeskSupervisorActionBody {
  next_agent_status?: 'available' | 'away';
  reason?: string;
}

interface HelpdeskTicketAssignBody {
  agent_id: string;
  reason?: string;
}

export async function apiHelpdeskTicketAssign(
  ticketId: string,
  body: HelpdeskTicketAssignBody,
): Promise<HelpdeskTicket> {
  const response = await request<{ ticket: HelpdeskTicket }>(
    `${helpdeskBasePath()}/tickets/${encodeURIComponent(ticketId)}/assign`,
    {
      method: 'POST',
      body: JSON.stringify(body),
    },
  );
  return response.ticket;
}

interface HelpdeskTicketOperationalBody {
  difficulty?: string;
  estimated_minutes?: number;
}

export async function apiHelpdeskTicketUpdateOperational(
  ticketId: string,
  body: HelpdeskTicketOperationalBody,
): Promise<HelpdeskTicket> {
  const response = await request<{ ticket: HelpdeskTicket }>(
    `${helpdeskBasePath()}/tickets/${encodeURIComponent(ticketId)}/operational`,
    {
      method: 'POST',
      body: JSON.stringify(body),
    },
  );
  return response.ticket;
}

export async function apiHelpdeskTicketRequeue(
  ticketId: string,
  body: HelpdeskSupervisorActionBody,
): Promise<HelpdeskTicket | null> {
  const response = await request<{ ticket: HelpdeskTicket | null }>(
    `${helpdeskBasePath()}/tickets/${encodeURIComponent(ticketId)}/requeue`,
    {
      method: 'POST',
      body: JSON.stringify(body),
    },
  );
  return response.ticket;
}

export async function apiHelpdeskTicketCancel(
  ticketId: string,
  body: HelpdeskSupervisorActionBody,
): Promise<HelpdeskTicket | null> {
  const response = await request<{ ticket: HelpdeskTicket | null }>(
    `${helpdeskBasePath()}/tickets/${encodeURIComponent(ticketId)}/cancel`,
    {
      method: 'POST',
      body: JSON.stringify(body),
    },
  );
  return response.ticket;
}

export function sessionPresenceStreamUrl(sessionId: string): string {
  return `/api/v1/sessions/${encodeURIComponent(sessionId)}/presence/stream`;
}

export function sessionsCsvUrl(from?: string, to?: string, userId?: string, actorType?: SessionActorType): string {
  return `/api/v1/reports/sessions.csv${buildQuery({ from, to, user_id: userId, actor_type: actorType })}`;
}

export { ApiError };

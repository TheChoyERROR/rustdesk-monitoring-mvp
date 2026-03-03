import type {
  AuthLoginRequest,
  AuthLoginResponse,
  DashboardSummary,
  PaginatedResponse,
  PresenceSessionSummary,
  SessionPresence,
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
): Promise<PaginatedResponse<SessionTimelineItem>> {
  return request<PaginatedResponse<SessionTimelineItem>>(
    `/api/v1/sessions/${encodeURIComponent(sessionId)}/timeline${buildQuery({
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

export function sessionPresenceStreamUrl(sessionId: string): string {
  return `/api/v1/sessions/${encodeURIComponent(sessionId)}/presence/stream`;
}

export function sessionsCsvUrl(from?: string, to?: string, userId?: string): string {
  return `/api/v1/reports/sessions.csv${buildQuery({ from, to, user_id: userId })}`;
}

export { ApiError };

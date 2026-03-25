import type { SessionEventType, SessionTimelineItem } from '../types';

export interface SessionActivityUserSummary {
  userId: string;
  displayName: string;
  avatarUrl?: string | null;
  totalEvents: number;
  sessionCount: number;
  lastEventAt: string;
}

export interface SessionActivitySegment {
  userId: string;
  displayName: string;
  avatarUrl?: string | null;
  sessionId: string;
  direction: SessionTimelineItem['direction'];
  startMs: number;
  endMs: number;
  eventCount: number;
  lastEventType: SessionEventType;
}

export interface SessionActivityTimelineModel {
  users: SessionActivityUserSummary[];
  segments: SessionActivitySegment[];
  rangeStartIso: string;
  rangeEndIso: string;
  totalEventsLoaded: number;
  totalEventsMatching: number;
  totalSessions: number;
  truncated: boolean;
}

interface VisualEvent extends SessionTimelineItem {
  timestampMs: number;
  displayName: string;
  avatarUrl?: string | null;
}

function optionalString(record: Record<string, unknown> | undefined, key: string): string | null {
  if (!record) {
    return null;
  }
  const value = record[key];
  if (typeof value !== 'string') {
    return null;
  }
  const trimmed = value.trim();
  return trimmed === '' ? null : trimmed;
}

function parseTimestampMs(isoString: string): number | null {
  const timestampMs = new Date(isoString).getTime();
  if (Number.isNaN(timestampMs)) {
    return null;
  }
  return timestampMs;
}

function displayNameForEvent(item: SessionTimelineItem): string {
  const meta = item.meta ?? undefined;
  return (
    optionalString(meta, 'display_name') ??
    optionalString(meta, 'name') ??
    optionalString(meta, 'participant_id') ??
    item.user_id
  );
}

function avatarUrlForEvent(item: SessionTimelineItem): string | null {
  const meta = item.meta ?? undefined;
  return optionalString(meta, 'avatar_url') ?? optionalString(meta, 'avatar');
}

function eventUpdatesConnectedState(eventType: SessionEventType): boolean | null {
  switch (eventType) {
    case 'session_started':
    case 'participant_joined':
    case 'participant_activity':
    case 'recording_started':
    case 'control_changed':
      return true;
    case 'session_ended':
    case 'participant_left':
      return false;
    default:
      return null;
  }
}

export function buildSessionActivityTimeline(
  items: SessionTimelineItem[],
  totalEventsMatching: number,
): SessionActivityTimelineModel | null {
  const events = items.reduce<VisualEvent[]>((accumulator, item) => {
      const timestampMs = parseTimestampMs(item.timestamp);
      if (timestampMs === null) {
        return accumulator;
      }
      accumulator.push({
        ...item,
        timestampMs,
        displayName: displayNameForEvent(item),
        avatarUrl: avatarUrlForEvent(item),
      });
      return accumulator;
    }, []);

  events.sort((left, right) => left.timestampMs - right.timestampMs);

  if (events.length === 0) {
    return null;
  }

  const userSummaries = new Map<
    string,
    {
      userId: string;
      displayName: string;
      avatarUrl?: string | null;
      totalEvents: number;
      sessionIds: Set<string>;
      lastEventAt: string;
      lastEventMs: number;
    }
  >();
  const eventsByUserAndSession = new Map<string, VisualEvent[]>();

  events.forEach((event) => {
    const currentSummary = userSummaries.get(event.user_id);
    if (currentSummary) {
      currentSummary.totalEvents += 1;
      currentSummary.sessionIds.add(event.session_id);
      if (event.timestampMs >= currentSummary.lastEventMs) {
        currentSummary.lastEventMs = event.timestampMs;
        currentSummary.lastEventAt = event.timestamp;
      }
      if (event.displayName !== event.user_id) {
        currentSummary.displayName = event.displayName;
      }
      if (event.avatarUrl) {
        currentSummary.avatarUrl = event.avatarUrl;
      }
    } else {
      userSummaries.set(event.user_id, {
        userId: event.user_id,
        displayName: event.displayName,
        avatarUrl: event.avatarUrl,
        totalEvents: 1,
        sessionIds: new Set([event.session_id]),
        lastEventAt: event.timestamp,
        lastEventMs: event.timestampMs,
      });
    }

    const key = `${event.user_id}::${event.session_id}`;
    const bucket = eventsByUserAndSession.get(key);
    if (bucket) {
      bucket.push(event);
    } else {
      eventsByUserAndSession.set(key, [event]);
    }
  });

  const users = Array.from(userSummaries.values())
    .map((summary) => ({
      userId: summary.userId,
      displayName: summary.displayName,
      avatarUrl: summary.avatarUrl,
      totalEvents: summary.totalEvents,
      sessionCount: summary.sessionIds.size,
      lastEventAt: summary.lastEventAt,
      lastEventMs: summary.lastEventMs,
    }))
    .sort((left, right) => {
      if (right.lastEventMs !== left.lastEventMs) {
        return right.lastEventMs - left.lastEventMs;
      }
      if (right.totalEvents !== left.totalEvents) {
        return right.totalEvents - left.totalEvents;
      }
      return left.displayName.localeCompare(right.displayName);
    });

  const segments: SessionActivitySegment[] = [];

  eventsByUserAndSession.forEach((sessionEvents) => {
    sessionEvents.sort((left, right) => left.timestampMs - right.timestampMs);
    const firstEvent = sessionEvents[0];
    const lastEvent = sessionEvents[sessionEvents.length - 1];
    let openStartMs: number | null = null;
    let openEventCount = 0;
    let lastEventType: SessionEventType = firstEvent.event_type;

    sessionEvents.forEach((event) => {
      const nextState = eventUpdatesConnectedState(event.event_type);
      lastEventType = event.event_type;

      if (nextState === true) {
        if (openStartMs === null) {
          openStartMs = event.timestampMs;
          openEventCount = 1;
        } else {
          openEventCount += 1;
        }
        return;
      }

      if (nextState === false) {
        if (openStartMs !== null) {
          openEventCount += 1;
          segments.push({
            userId: event.user_id,
            displayName: event.displayName,
            avatarUrl: event.avatarUrl,
            sessionId: event.session_id,
            direction: event.direction,
            startMs: openStartMs,
            endMs: Math.max(event.timestampMs, openStartMs + 1_000),
            eventCount: openEventCount,
            lastEventType: event.event_type,
          });
          openStartMs = null;
          openEventCount = 0;
        }
        return;
      }

      if (openStartMs !== null) {
        openEventCount += 1;
      }
    });

    if (openStartMs !== null) {
      segments.push({
        userId: lastEvent.user_id,
        displayName: lastEvent.displayName,
        avatarUrl: lastEvent.avatarUrl,
        sessionId: lastEvent.session_id,
        direction: lastEvent.direction,
        startMs: openStartMs,
        endMs: Math.max(lastEvent.timestampMs + 60_000, openStartMs + 60_000),
        eventCount: Math.max(openEventCount, 1),
        lastEventType,
      });
      return;
    }

    if (segments.some((segment) => segment.sessionId === firstEvent.session_id && segment.userId === firstEvent.user_id)) {
      return;
    }

    segments.push({
      userId: firstEvent.user_id,
      displayName: firstEvent.displayName,
      avatarUrl: firstEvent.avatarUrl,
      sessionId: firstEvent.session_id,
      direction: firstEvent.direction,
      startMs: firstEvent.timestampMs,
      endMs: Math.max(lastEvent.timestampMs, firstEvent.timestampMs + 60_000),
      eventCount: sessionEvents.length,
      lastEventType,
    });
  });

  const rangeStartMs = Math.min(...segments.map((segment) => segment.startMs));
  const rangeEndMs = Math.max(...segments.map((segment) => segment.endMs));

  return {
    users: users.map(({ lastEventMs: _ignored, ...user }) => user),
    segments: segments.sort((left, right) => left.startMs - right.startMs),
    rangeStartIso: new Date(rangeStartMs).toISOString(),
    rangeEndIso: new Date(rangeEndMs).toISOString(),
    totalEventsLoaded: events.length,
    totalEventsMatching,
    totalSessions: new Set(events.map((event) => event.session_id)).size,
    truncated: totalEventsMatching > events.length,
  };
}

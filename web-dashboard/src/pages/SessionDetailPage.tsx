import { useCallback, useEffect, useMemo, useState, type CSSProperties } from 'react';
import { useParams } from 'react-router-dom';

import { apiSessionPresence, apiSessionTimeline, sessionPresenceStreamUrl } from '../api';
import { formatDateTime } from '../lib/time';
import type { PresenceParticipant, SessionPresence, SessionTimelineItem } from '../types';

function participantName(participant: PresenceParticipant): string {
  const fromDisplayName = participant.display_name?.trim();
  if (fromDisplayName) {
    return fromDisplayName;
  }
  return participant.participant_id;
}

function participantInitial(participant: PresenceParticipant): string {
  const name = participantName(participant);
  return name.charAt(0).toUpperCase();
}

function hasAvatarUrl(participant: PresenceParticipant): participant is PresenceParticipant & { avatar_url: string } {
  return typeof participant.avatar_url === 'string' && participant.avatar_url.trim().length > 0;
}

const CHART_PALETTE = ['#0f766e', '#f97316', '#0ea5e9', '#22c55e', '#ef4444', '#8b5cf6', '#f59e0b'];
const SPARKLINE_WIDTH = 260;
const SPARKLINE_HEIGHT = 68;
const SPARKLINE_PADDING = 6;
const SPARKLINE_BUCKETS = 12;
const STATUS_CONNECTED_COLOR = '#22c55e';
const STATUS_DISCONNECTED_COLOR = '#ef4444';

type TimelineWithMs = SessionTimelineItem & { timestampMs: number };

interface UserActivitySeries {
  userId: string;
  displayName: string;
  avatarUrl?: string | null;
  isActive: boolean;
  isControlActive: boolean;
  totalEvents: number;
  lastEventAt: string;
  eventCounts: number[];
  connectedByBucket: boolean[];
  color: string;
}

interface ActivityTimelineModel {
  rangeStartIso: string;
  rangeEndIso: string;
  bucketSizeMinutes: number;
  bucketSizeMs: number;
  bucketStartMs: number[];
  axisLabelIndexes: number[];
  series: UserActivitySeries[];
}

function parseTimestampMs(isoString: string): number | null {
  const ms = new Date(isoString).getTime();
  if (Number.isNaN(ms)) {
    return null;
  }
  return ms;
}

function colorForUser(userId: string): string {
  let hash = 0;
  for (let index = 0; index < userId.length; index += 1) {
    hash = (hash * 31 + userId.charCodeAt(index)) >>> 0;
  }
  return CHART_PALETTE[hash % CHART_PALETTE.length];
}

function statusPointCoordinates(index: number, totalPoints: number, isConnected: boolean): { x: number; y: number } {
  const innerWidth = SPARKLINE_WIDTH - SPARKLINE_PADDING * 2;
  const innerHeight = SPARKLINE_HEIGHT - SPARKLINE_PADDING * 2;
  const lastIndex = Math.max(totalPoints - 1, 1);
  const x = SPARKLINE_PADDING + (index * innerWidth) / lastIndex;
  const yConnected = SPARKLINE_PADDING + innerHeight * 0.2;
  const yDisconnected = SPARKLINE_PADDING + innerHeight * 0.8;
  return { x, y: isConnected ? yConnected : yDisconnected };
}

function buildStatusPolylinePoints(states: boolean[]): string {
  return states
    .map((isConnected, index) => {
      const point = statusPointCoordinates(index, states.length, isConnected);
      return `${point.x},${point.y}`;
    })
    .join(' ');
}

function formatHourMinute(isoString: string): string {
  const date = new Date(isoString);
  if (Number.isNaN(date.getTime())) {
    return '--:--';
  }
  return new Intl.DateTimeFormat(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  }).format(date);
}

function buildAxisLabelIndexes(bucketCount: number): number[] {
  if (bucketCount <= 1) {
    return [0];
  }
  const middle = Math.floor((bucketCount - 1) / 2);
  const last = bucketCount - 1;
  return Array.from(new Set([0, middle, last]));
}

function eventUpdatesConnectedState(eventType: SessionTimelineItem['event_type']): boolean | null {
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

function buildActivityTimeline(
  timeline: SessionTimelineItem[],
  presence: SessionPresence | null,
): ActivityTimelineModel | null {
  const timelineWithMs: TimelineWithMs[] = timeline
    .map((item) => {
      const timestampMs = parseTimestampMs(item.timestamp);
      if (timestampMs === null) {
        return null;
      }
      return { ...item, timestampMs };
    })
    .filter((item): item is TimelineWithMs => item !== null)
    .sort((left, right) => left.timestampMs - right.timestampMs);

  if (timelineWithMs.length === 0) {
    return null;
  }

  const presenceByParticipantId = new Map<string, PresenceParticipant>();
  (presence?.participants ?? []).forEach((participant) => {
    presenceByParticipantId.set(participant.participant_id, participant);
  });

  const rangeStartMs = timelineWithMs[0].timestampMs;
  const rangeEndMs = timelineWithMs[timelineWithMs.length - 1].timestampMs;
  const durationMs = Math.max(60_000, rangeEndMs - rangeStartMs + 1);
  const bucketSizeMs = Math.max(60_000, Math.ceil(durationMs / SPARKLINE_BUCKETS));
  const bucketStartMs = Array.from({ length: SPARKLINE_BUCKETS }, (_, index) => rangeStartMs + index * bucketSizeMs);
  const axisLabelIndexes = buildAxisLabelIndexes(SPARKLINE_BUCKETS);

  const eventsByUserId = new Map<string, TimelineWithMs[]>();
  timelineWithMs.forEach((item) => {
    const current = eventsByUserId.get(item.user_id);
    if (current) {
      current.push(item);
    } else {
      eventsByUserId.set(item.user_id, [item]);
    }
  });

  const series = Array.from(eventsByUserId.entries()).map(([userId, userTimeline]) => {
    userTimeline.sort((left, right) => left.timestampMs - right.timestampMs);

    const participant = presenceByParticipantId.get(userId);
    const displayName = participant ? participantName(participant) : userId;
    const avatarUrl = participant?.avatar_url;
    const isActive = participant?.is_active ?? false;
    const isControlActive = participant?.is_control_active ?? false;

    const eventCounts = Array.from({ length: SPARKLINE_BUCKETS }, () => 0);
    userTimeline.forEach((item) => {
      const bucketIndex = Math.min(
        SPARKLINE_BUCKETS - 1,
        Math.floor((item.timestampMs - rangeStartMs) / bucketSizeMs),
      );
      eventCounts[bucketIndex] += 1;
    });

    // Defaults to disconnected until join/start/activity/control event marks active.
    let connectedState = false;
    const connectedByBucket = Array.from({ length: SPARKLINE_BUCKETS }, () => false);
    let eventIndex = 0;

    for (let bucketIndex = 0; bucketIndex < SPARKLINE_BUCKETS; bucketIndex += 1) {
      const bucketEndMs = rangeStartMs + (bucketIndex + 1) * bucketSizeMs;
      while (eventIndex < userTimeline.length && userTimeline[eventIndex].timestampMs < bucketEndMs) {
        const nextState = eventUpdatesConnectedState(userTimeline[eventIndex].event_type);
        if (nextState !== null) {
          connectedState = nextState;
        }
        eventIndex += 1;
      }
      connectedByBucket[bucketIndex] = connectedState;
    }

    // If participant is currently active and no events indicated disconnection,
    // keep the last bucket as connected so chart matches real-time snapshot.
    if (isActive && connectedByBucket.every((value) => !value)) {
      connectedByBucket[connectedByBucket.length - 1] = true;
    }

    return {
      userId,
      displayName,
      avatarUrl,
      isActive,
      isControlActive,
      totalEvents: userTimeline.length,
      lastEventAt: userTimeline[userTimeline.length - 1].timestamp,
      eventCounts,
      connectedByBucket,
      color: colorForUser(userId),
    };
  });

  series.sort((left, right) => {
    if (right.totalEvents !== left.totalEvents) {
      return right.totalEvents - left.totalEvents;
    }
    return left.displayName.localeCompare(right.displayName);
  });

  return {
    rangeStartIso: new Date(rangeStartMs).toISOString(),
    rangeEndIso: new Date(rangeEndMs).toISOString(),
    bucketSizeMinutes: Math.max(1, Math.round(bucketSizeMs / 60_000)),
    bucketSizeMs,
    bucketStartMs,
    axisLabelIndexes,
    series,
  };
}

function hasAvatarForSeries(series: UserActivitySeries): series is UserActivitySeries & { avatarUrl: string } {
  return typeof series.avatarUrl === 'string' && series.avatarUrl.trim().length > 0;
}

function seriesInitial(series: UserActivitySeries): string {
  return (series.displayName.charAt(0) || series.userId.charAt(0) || '?').toUpperCase();
}

export default function SessionDetailPage() {
  const { sessionId = '' } = useParams();
  const decodedSessionId = useMemo(() => decodeURIComponent(sessionId), [sessionId]);

  const [timeline, setTimeline] = useState<SessionTimelineItem[]>([]);
  const [presence, setPresence] = useState<SessionPresence | null>(null);
  const [streamState, setStreamState] = useState('conectando');
  const [error, setError] = useState<string | null>(null);
  const [failedAvatarIds, setFailedAvatarIds] = useState<Record<string, boolean>>({});
  const [failedSeriesAvatarIds, setFailedSeriesAvatarIds] = useState<Record<string, boolean>>({});

  const activityTimeline = useMemo(() => buildActivityTimeline(timeline, presence), [timeline, presence]);

  const load = useCallback(async () => {
    try {
      const [timelineData, presenceData] = await Promise.all([
        apiSessionTimeline(decodedSessionId, 1, 500),
        apiSessionPresence(decodedSessionId),
      ]);
      setTimeline(timelineData.items);
      setPresence(presenceData);
      setFailedAvatarIds({});
      setFailedSeriesAvatarIds({});
      setError(null);
    } catch {
      setError('No se pudo cargar el detalle de sesion.');
    }
  }, [decodedSessionId]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    const stream = new EventSource(sessionPresenceStreamUrl(decodedSessionId), {
      withCredentials: true,
    });

    stream.addEventListener('open', () => {
      setStreamState('conectado');
    });

    stream.addEventListener('presence_snapshot', (event) => {
      try {
        const parsed = JSON.parse(event.data) as { presence?: SessionPresence | null };
        setPresence(parsed.presence ?? null);
      } catch (parseError) {
        console.error('presence_snapshot parse error', parseError);
      }
    });

    stream.addEventListener('presence_missing', () => {
      setPresence(null);
    });

    stream.addEventListener('presence_lagged', () => {
      setStreamState('recuperando');
    });

    stream.addEventListener('error', () => {
      setStreamState('reconectando');
    });

    return () => {
      stream.close();
    };
  }, [decodedSessionId]);

  return (
    <section className="stack">
      <div className="panel">
        <h2>Detalle de sesion: {decodedSessionId}</h2>
        <p>
          Stream presencia: <strong>{streamState}</strong>
        </p>
        <button type="button" className="btn secondary" onClick={load}>
          Refrescar timeline
        </button>
      </div>

      {error && <div className="panel error-text">{error}</div>}

      <div className="panel">
        <h3>Presencia en tiempo real</h3>
        {!presence ? (
          <p>Sin datos activos de presencia.</p>
        ) : (
          <>
            <p>
              Ultima actualizacion: {formatDateTime(presence.updated_at)} | Control activo:{' '}
              {presence.control_participant_id ?? 'ninguno'}
            </p>
            <div className="participants-grid">
              {presence.participants.map((participant) => (
                <article key={participant.participant_id} className="participant-card">
                  <div className="participant-heading">
                    {hasAvatarUrl(participant) ? (
                      failedAvatarIds[participant.participant_id] ? (
                        <div className="participant-avatar-fallback" aria-hidden="true">
                          {participantInitial(participant)}
                        </div>
                      ) : (
                        <img
                          src={participant.avatar_url}
                          alt={`Avatar de ${participantName(participant)}`}
                          className="participant-avatar"
                          loading="lazy"
                          onError={() => {
                            setFailedAvatarIds((prev) => ({
                              ...prev,
                              [participant.participant_id]: true,
                            }));
                          }}
                        />
                      )
                    ) : (
                      <div className="participant-avatar-fallback" aria-hidden="true">
                        {participantInitial(participant)}
                      </div>
                    )}
                    <div>
                      <h4>{participantName(participant)}</h4>
                      {participant.is_control_active && <span className="participant-badge">Control activo</span>}
                    </div>
                  </div>
                  <p>ID: {participant.participant_id}</p>
                  <p>Activo: {participant.is_active ? 'si' : 'no'}</p>
                  <p>Avatar: {hasAvatarUrl(participant) ? 'si' : 'no'}</p>
                  <p>Ultima actividad: {formatDateTime(participant.last_activity_at)}</p>
                </article>
              ))}
            </div>
          </>
        )}
      </div>

      <div className="panel">
        <h3>Actividad por usuario</h3>
        {!activityTimeline || activityTimeline.series.length === 0 ? (
          <p>Sin eventos suficientes para construir timeline de actividad.</p>
        ) : (
          <>
            <p className="activity-summary-line">
              Ventana: {formatDateTime(activityTimeline.rangeStartIso)} -{' '}
              {formatDateTime(activityTimeline.rangeEndIso)} | Intervalo: {activityTimeline.bucketSizeMinutes} min
            </p>
            <div className="activity-legend" role="note" aria-label="Leyenda del grafico de actividad">
              <span className="activity-legend-item">
                <span className="activity-legend-dot connected" aria-hidden="true" /> Nodo verde: conectado
              </span>
              <span className="activity-legend-item">
                <span className="activity-legend-dot disconnected" aria-hidden="true" /> Nodo rojo: sin conexion/sin hook
              </span>
              <span className="activity-legend-item">
                <span className="activity-legend-dot neutral" aria-hidden="true" /> La linea muestra cambio de estado por minuto
              </span>
            </div>
            <div className="activity-users-grid">
              {activityTimeline.series.map((series) => {
                const style = { '--series-color': series.color } as CSSProperties;
                return (
                  <article key={series.userId} className="activity-user-card" style={style}>
                    <div className="activity-user-header">
                      {hasAvatarForSeries(series) ? (
                        failedSeriesAvatarIds[series.userId] ? (
                          <div className="participant-avatar-fallback" aria-hidden="true">
                            {seriesInitial(series)}
                          </div>
                        ) : (
                          <img
                            src={series.avatarUrl}
                            alt={`Avatar de ${series.displayName}`}
                            className="participant-avatar"
                            loading="lazy"
                            onError={() => {
                              setFailedSeriesAvatarIds((prev) => ({
                                ...prev,
                                [series.userId]: true,
                              }));
                            }}
                          />
                        )
                      ) : (
                        <div className="participant-avatar-fallback" aria-hidden="true">
                          {seriesInitial(series)}
                        </div>
                      )}
                      <div>
                        <h4>{series.displayName}</h4>
                        <p className="activity-user-meta">ID: {series.userId}</p>
                        {series.isControlActive && <span className="participant-badge">Control activo</span>}
                      </div>
                    </div>

                    <div className="activity-user-stats">
                      <p>Total eventos: {series.totalEvents}</p>
                      <p>Ultimo evento: {formatDateTime(series.lastEventAt)}</p>
                      <p>Estado: {series.isActive ? 'activo' : 'inactivo'}</p>
                    </div>

                    <div className="activity-sparkline-wrap">
                      <svg
                        className="activity-sparkline"
                        viewBox={`0 0 ${SPARKLINE_WIDTH} ${SPARKLINE_HEIGHT}`}
                        role="img"
                        aria-label={`Timeline de actividad para ${series.displayName}`}
                      >
                        <line
                          className="activity-sparkline-baseline"
                          x1={SPARKLINE_PADDING}
                          y1={SPARKLINE_HEIGHT / 2}
                          x2={SPARKLINE_WIDTH - SPARKLINE_PADDING}
                          y2={SPARKLINE_HEIGHT / 2}
                        />
                        <polyline
                          className="activity-sparkline-line"
                          points={buildStatusPolylinePoints(series.connectedByBucket)}
                        />
                        {series.connectedByBucket.map((isConnected, bucketIndex) => {
                          const point = statusPointCoordinates(
                            bucketIndex,
                            series.connectedByBucket.length,
                            isConnected,
                          );
                          const bucketStartMs = activityTimeline.bucketStartMs[bucketIndex];
                          const bucketEndMs = bucketStartMs + activityTimeline.bucketSizeMs;
                          const bucketStartIso = new Date(bucketStartMs).toISOString();
                          const bucketEndIso = new Date(bucketEndMs).toISOString();
                          return (
                            <circle
                              key={`${series.userId}-bucket-${bucketIndex}`}
                              className={`activity-status-node ${isConnected ? 'connected' : 'disconnected'}`}
                              cx={point.x}
                              cy={point.y}
                              r={3.8}
                              fill={isConnected ? STATUS_CONNECTED_COLOR : STATUS_DISCONNECTED_COLOR}
                            >
                              <title>
                                {`Ventana ${formatHourMinute(bucketStartIso)}-${formatHourMinute(bucketEndIso)} | Estado: ${
                                  isConnected ? 'conectado' : 'sin conexion'
                                } | Eventos: ${series.eventCounts[bucketIndex]}`}
                              </title>
                            </circle>
                          );
                        })}
                      </svg>
                      <div className="activity-axis-labels" aria-hidden="true">
                        {activityTimeline.axisLabelIndexes.map((bucketIndex) => {
                          const left =
                            activityTimeline.bucketStartMs.length <= 1
                              ? 0
                              : (bucketIndex / (activityTimeline.bucketStartMs.length - 1)) * 100;
                          return (
                            <span
                              key={`${series.userId}-axis-${bucketIndex}`}
                              className="activity-axis-label"
                              style={{ left: `${left}%` }}
                            >
                              {formatHourMinute(new Date(activityTimeline.bucketStartMs[bucketIndex]).toISOString())}
                            </span>
                          );
                        })}
                      </div>
                    </div>
                  </article>
                );
              })}
            </div>
          </>
        )}
      </div>

      <div className="panel">
        <h3>Timeline</h3>
        {timeline.length === 0 ? (
          <p>Sin eventos para esta sesion.</p>
        ) : (
          <table>
            <thead>
              <tr>
                <th>Timestamp</th>
                <th>Evento</th>
                <th>User</th>
                <th>Direccion</th>
              </tr>
            </thead>
            <tbody>
              {timeline.map((item) => (
                <tr key={item.event_id}>
                  <td>{formatDateTime(item.timestamp)}</td>
                  <td>{item.event_type}</td>
                  <td>{item.user_id}</td>
                  <td>{item.direction}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </section>
  );
}

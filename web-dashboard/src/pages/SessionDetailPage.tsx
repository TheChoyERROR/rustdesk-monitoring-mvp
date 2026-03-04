import { useCallback, useEffect, useMemo, useState } from 'react';
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

export default function SessionDetailPage() {
  const { sessionId = '' } = useParams();
  const decodedSessionId = useMemo(() => decodeURIComponent(sessionId), [sessionId]);

  const [timeline, setTimeline] = useState<SessionTimelineItem[]>([]);
  const [presence, setPresence] = useState<SessionPresence | null>(null);
  const [streamState, setStreamState] = useState('conectando');
  const [error, setError] = useState<string | null>(null);
  const [failedAvatarIds, setFailedAvatarIds] = useState<Record<string, boolean>>({});

  const load = useCallback(async () => {
    try {
      const [timelineData, presenceData] = await Promise.all([
        apiSessionTimeline(decodedSessionId, 1, 100),
        apiSessionPresence(decodedSessionId),
      ]);
      setTimeline(timelineData.items);
      setPresence(presenceData);
      setFailedAvatarIds({});
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

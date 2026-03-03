import { useCallback, useEffect, useMemo, useState } from 'react';
import { useParams } from 'react-router-dom';

import { apiSessionPresence, apiSessionTimeline, sessionPresenceStreamUrl } from '../api';
import { formatDateTime } from '../lib/time';
import type { SessionPresence, SessionTimelineItem } from '../types';

export default function SessionDetailPage() {
  const { sessionId = '' } = useParams();
  const decodedSessionId = useMemo(() => decodeURIComponent(sessionId), [sessionId]);

  const [timeline, setTimeline] = useState<SessionTimelineItem[]>([]);
  const [presence, setPresence] = useState<SessionPresence | null>(null);
  const [streamState, setStreamState] = useState('conectando');
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [timelineData, presenceData] = await Promise.all([
        apiSessionTimeline(decodedSessionId, 1, 100),
        apiSessionPresence(decodedSessionId),
      ]);
      setTimeline(timelineData.items);
      setPresence(presenceData);
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
                  <h4>{participant.display_name}</h4>
                  <p>ID: {participant.participant_id}</p>
                  <p>Activo: {participant.is_active ? 'si' : 'no'}</p>
                  <p>Control: {participant.is_control_active ? 'si' : 'no'}</p>
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

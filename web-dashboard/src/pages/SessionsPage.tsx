import { useCallback, useEffect, useMemo, useState } from 'react';
import { Link } from 'react-router-dom';

import { apiEvents, sessionsCsvUrl } from '../api';
import { formatDateTime, fromLocalInputValue, toLocalInputValue } from '../lib/time';
import type { PaginatedResponse, SessionEventType, SessionTimelineItem } from '../types';

const EVENT_TYPES: SessionEventType[] = [
  'session_started',
  'session_ended',
  'recording_started',
  'recording_stopped',
  'participant_joined',
  'participant_left',
  'control_changed',
  'participant_activity',
];

function initialRange() {
  const to = new Date();
  const from = new Date(to.getTime() - 24 * 60 * 60 * 1000);
  return {
    from: toLocalInputValue(from.toISOString()),
    to: toLocalInputValue(to.toISOString()),
  };
}

export default function SessionsPage() {
  const range = useMemo(initialRange, []);
  const [sessionId, setSessionId] = useState('');
  const [userId, setUserId] = useState('');
  const [eventType, setEventType] = useState<string>('');
  const [from, setFrom] = useState(range.from);
  const [to, setTo] = useState(range.to);
  const [page, setPage] = useState(1);
  const [result, setResult] = useState<PaginatedResponse<SessionTimelineItem> | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async (targetPage = page) => {
    setLoading(true);
    setError(null);

    try {
      const payload = await apiEvents({
        session_id: sessionId || undefined,
        user_id: userId || undefined,
        event_type: (eventType || undefined) as SessionEventType | undefined,
        from: fromLocalInputValue(from),
        to: fromLocalInputValue(to),
        page: targetPage,
        page_size: 25,
      });
      setResult(payload);
      setPage(targetPage);
    } catch {
      setError('No se pudo cargar el listado de eventos.');
    } finally {
      setLoading(false);
    }
  }, [eventType, from, page, sessionId, to, userId]);

  useEffect(() => {
    void load(1);
  }, [load]);

  const csvUrl = sessionsCsvUrl(fromLocalInputValue(from), fromLocalInputValue(to), userId || undefined);

  return (
    <section className="stack">
      <div className="panel">
        <h2>Busqueda de sesiones/eventos</h2>
        <div className="filter-grid">
          <div>
            <label htmlFor="session-id">Session ID</label>
            <input id="session-id" value={sessionId} onChange={(event) => setSessionId(event.target.value)} />
          </div>
          <div>
            <label htmlFor="user-id">User ID</label>
            <input id="user-id" value={userId} onChange={(event) => setUserId(event.target.value)} />
          </div>
          <div>
            <label htmlFor="event-type">Tipo de evento</label>
            <select id="event-type" value={eventType} onChange={(event) => setEventType(event.target.value)}>
              <option value="">Todos</option>
              {EVENT_TYPES.map((value) => (
                <option key={value} value={value}>
                  {value}
                </option>
              ))}
            </select>
          </div>
          <div>
            <label htmlFor="from">Desde</label>
            <input id="from" type="datetime-local" value={from} onChange={(event) => setFrom(event.target.value)} />
          </div>
          <div>
            <label htmlFor="to">Hasta</label>
            <input id="to" type="datetime-local" value={to} onChange={(event) => setTo(event.target.value)} />
          </div>
        </div>
        <div className="filter-actions">
          <button type="button" className="btn primary" onClick={() => load(1)}>
            Aplicar filtros
          </button>
          <a className="btn secondary" href={csvUrl}>
            Exportar CSV
          </a>
        </div>
      </div>

      {loading && <div className="panel">Cargando...</div>}
      {error && <div className="panel error-text">{error}</div>}

      {result && (
        <div className="panel">
          <table>
            <thead>
              <tr>
                <th>Timestamp</th>
                <th>Evento</th>
                <th>Session ID</th>
                <th>User ID</th>
                <th>Direccion</th>
                <th>Detalle</th>
              </tr>
            </thead>
            <tbody>
              {result.items.map((item) => (
                <tr key={item.event_id}>
                  <td>{formatDateTime(item.timestamp)}</td>
                  <td>{item.event_type}</td>
                  <td>{item.session_id}</td>
                  <td>{item.user_id}</td>
                  <td>{item.direction}</td>
                  <td>
                    <Link to={`/sessions/${encodeURIComponent(item.session_id)}`}>Abrir</Link>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>

          <div className="pager">
            <button
              type="button"
              className="btn secondary"
              disabled={page <= 1}
              onClick={() => load(page - 1)}
            >
              Anterior
            </button>
            <span>
              Pagina {result.page} - Total {result.total}
            </span>
            <button
              type="button"
              className="btn secondary"
              disabled={result.page * result.page_size >= result.total}
              onClick={() => load(page + 1)}
            >
              Siguiente
            </button>
          </div>
        </div>
      )}
    </section>
  );
}

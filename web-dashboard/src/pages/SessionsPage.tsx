import { useCallback, useEffect, useMemo, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';

import SessionActivityChart from '../components/SessionActivityChart';
import { apiEvents, sessionsCsvUrl } from '../api';
import { buildSessionActivityTimeline } from '../lib/session-activity';
import { formatDateTime, fromLocalInputValue, toLocalInputValue } from '../lib/time';
import type { PaginatedResponse, SessionActorType, SessionEventType, SessionTimelineItem } from '../types';

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

const ACTOR_TYPES: SessionActorType[] = ['agent', 'client', 'unknown'];
const EVENTS_PER_PAGE = 200;
const RAW_TABLE_ROWS = 25;

function initialRange() {
  const to = new Date();
  const from = new Date(to.getTime() - 24 * 60 * 60 * 1000);
  return {
    from: toLocalInputValue(from.toISOString()),
    to: toLocalInputValue(to.toISOString()),
  };
}

function hasAvatarUrl(value: string | null | undefined): value is string {
  return typeof value === 'string' && value.trim().length > 0;
}

function actorTypeLabel(actorType: SessionActorType): string {
  switch (actorType) {
    case 'agent':
      return 'Agente';
    case 'client':
      return 'Cliente';
    case 'unknown':
      return 'Sin clasificar';
    default:
      return actorType;
  }
}

function actorTypeClass(actorType: SessionActorType): string {
  switch (actorType) {
    case 'agent':
      return 'status-good';
    case 'client':
      return 'status-info';
    case 'unknown':
      return 'status-neutral';
    default:
      return 'status-neutral';
  }
}

export default function SessionsPage() {
  const navigate = useNavigate();
  const range = useMemo(initialRange, []);
  const [sessionId, setSessionId] = useState('');
  const [userId, setUserId] = useState('');
  const [actorType, setActorType] = useState<SessionActorType | ''>('');
  const [eventType, setEventType] = useState<string>('');
  const [from, setFrom] = useState(range.from);
  const [to, setTo] = useState(range.to);
  const [page, setPage] = useState(1);
  const [result, setResult] = useState<PaginatedResponse<SessionTimelineItem> | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(
    async (targetPage = page) => {
      setLoading(true);
      setError(null);

      try {
        const payload = await apiEvents({
          session_id: sessionId || undefined,
          user_id: userId || undefined,
          actor_type: actorType || undefined,
          event_type: (eventType || undefined) as SessionEventType | undefined,
          from: fromLocalInputValue(from),
          to: fromLocalInputValue(to),
          page: targetPage,
          page_size: EVENTS_PER_PAGE,
        });
        setResult(payload);
        setPage(targetPage);
      } catch {
        setError('No se pudo cargar el listado de eventos.');
      } finally {
        setLoading(false);
      }
    },
    [actorType, eventType, from, page, sessionId, to, userId],
  );

  useEffect(() => {
    void load(1);
  }, [load]);

  const csvUrl = sessionsCsvUrl(
    fromLocalInputValue(from),
    fromLocalInputValue(to),
    userId || undefined,
    actorType || undefined,
  );
  const activityTimeline = useMemo(
    () => buildSessionActivityTimeline(result?.items ?? [], result?.total ?? 0),
    [result],
  );
  const latestEvents = result?.items.slice(0, RAW_TABLE_ROWS) ?? [];
  const actorCounts = useMemo(() => {
    if (!activityTimeline) {
      return { agent: 0, client: 0, unknown: 0 };
    }
    return activityTimeline.users.reduce(
      (accumulator, user) => {
        accumulator[user.actorType] += 1;
        return accumulator;
      },
      { agent: 0, client: 0, unknown: 0 } as Record<SessionActorType, number>,
    );
  }, [activityTimeline]);

  return (
    <section className="stack">
      <div className="panel">
        <h2>Busqueda de sesiones y actividad</h2>
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
            <label htmlFor="actor-type">Actor</label>
            <select id="actor-type" value={actorType} onChange={(event) => setActorType(event.target.value as SessionActorType | '')}>
              <option value="">Todos</option>
              {ACTOR_TYPES.map((value) => (
                <option key={value} value={value}>
                  {actorTypeLabel(value)}
                </option>
              ))}
            </select>
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

      {activityTimeline && (
        <>
          <div className="cards-grid">
            <article className="card-accent">
              <h3>Usuarios en timeline</h3>
              <strong>{activityTimeline.users.length}</strong>
            </article>
            <article className="card-accent">
              <h3>Sesiones visibles</h3>
              <strong>{activityTimeline.totalSessions}</strong>
            </article>
            <article className="card-muted">
              <h3>Eventos cargados</h3>
              <strong>{activityTimeline.totalEventsLoaded}</strong>
            </article>
            <article className="card-muted">
              <h3>Eventos totales filtro</h3>
              <strong>{activityTimeline.totalEventsMatching}</strong>
            </article>
            <article className="card-muted">
              <h3>Agentes visibles</h3>
              <strong>{actorCounts.agent}</strong>
            </article>
            <article className="card-muted">
              <h3>Clientes visibles</h3>
              <strong>{actorCounts.client}</strong>
            </article>
            <article className="card-muted">
              <h3>Sin clasificar</h3>
              <strong>{actorCounts.unknown}</strong>
            </article>
          </div>

          <div className="panel">
            <div className="dashboard-section-header">
              <div>
                <h3>Timeline visual por usuario</h3>
                <p className="activity-summary-line">
                  Cada barra representa un tramo de actividad por usuario y sesion. Haz clic en una barra para abrir
                  el detalle completo.
                </p>
                <p className="activity-summary-line">
                  Ventana: {formatDateTime(activityTimeline.rangeStartIso)} -{' '}
                  {formatDateTime(activityTimeline.rangeEndIso)}
                </p>
                <p className="activity-summary-line">
                  Clasificacion visible: {actorCounts.agent} agentes, {actorCounts.client} clientes y {actorCounts.unknown}{' '}
                  sin clasificar.
                </p>
                {activityTimeline.truncated ? (
                  <p className="activity-summary-line">
                    Se muestran los {activityTimeline.totalEventsLoaded} eventos mas recientes de esta pagina. Ajusta
                    el rango para una lectura mas precisa.
                  </p>
                ) : null}
              </div>
            </div>
            <SessionActivityChart
              model={activityTimeline}
              onSelectSession={(nextSessionId) => navigate(`/sessions/${encodeURIComponent(nextSessionId)}`)}
            />
          </div>

          <div className="user-summary-grid">
            {activityTimeline.users.map((user) => (
              <article key={user.userId} className="user-summary-card">
                <div className="user-summary-header">
                  {hasAvatarUrl(user.avatarUrl) ? (
                    <img
                      src={user.avatarUrl}
                      alt={`Avatar de ${user.displayName}`}
                      className="participant-avatar"
                      loading="lazy"
                    />
                  ) : (
                    <div className="participant-avatar-fallback" aria-hidden="true">
                      {(user.displayName.charAt(0) || user.userId.charAt(0) || '?').toUpperCase()}
                    </div>
                  )}
                  <div>
                    <h3>{user.displayName}</h3>
                    <p className="table-subtle">{user.userId}</p>
                    <span className={`status-pill ${actorTypeClass(user.actorType)}`}>{actorTypeLabel(user.actorType)}</span>
                  </div>
                </div>
                <div className="activity-user-stats">
                  <p>Total eventos: {user.totalEvents}</p>
                  <p>Sesiones: {user.sessionCount}</p>
                  <p>Ultimo evento: {formatDateTime(user.lastEventAt)}</p>
                </div>
              </article>
            ))}
          </div>
        </>
      )}

      {result && (
        <div className="panel">
          <div className="dashboard-section-header">
            <div>
              <h3>Eventos recientes</h3>
              <p className="activity-summary-line">
                Tabla reducida para auditoria puntual. La lectura principal ahora esta en el timeline visual.
              </p>
            </div>
          </div>

          <table>
            <thead>
                <tr>
                  <th>Timestamp</th>
                  <th>Evento</th>
                  <th>Session ID</th>
                  <th>User ID</th>
                  <th>Actor</th>
                  <th>Direccion</th>
                  <th>Detalle</th>
                </tr>
            </thead>
            <tbody>
              {latestEvents.map((item) => (
                <tr key={item.event_id}>
                  <td>{formatDateTime(item.timestamp)}</td>
                  <td>{item.event_type}</td>
                  <td>{item.session_id}</td>
                  <td>{item.user_id}</td>
                  <td>
                    <span className={`status-pill ${actorTypeClass(item.actor_type)}`}>{actorTypeLabel(item.actor_type)}</span>
                  </td>
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
              Pagina {result.page} - Total {result.total} - Mostrando {latestEvents.length} de {result.items.length}{' '}
              eventos cargados
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

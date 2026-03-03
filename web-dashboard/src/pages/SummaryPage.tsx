import { useCallback, useEffect, useMemo, useState } from 'react';

import { apiPresenceSessions, apiSummary } from '../api';
import { formatDateTime, fromLocalInputValue, toLocalInputValue } from '../lib/time';
import type { DashboardSummary, PresenceSessionSummary } from '../types';

function defaultRange() {
  const to = new Date();
  const from = new Date(to.getTime() - 24 * 60 * 60 * 1000);
  return {
    from: toLocalInputValue(from.toISOString()),
    to: toLocalInputValue(to.toISOString()),
  };
}

export default function SummaryPage() {
  const range = useMemo(defaultRange, []);
  const [from, setFrom] = useState(range.from);
  const [to, setTo] = useState(range.to);
  const [summary, setSummary] = useState<DashboardSummary | null>(null);
  const [activeSessions, setActiveSessions] = useState<PresenceSessionSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [summaryData, sessionsData] = await Promise.all([
        apiSummary(fromLocalInputValue(from), fromLocalInputValue(to)),
        apiPresenceSessions(),
      ]);
      setSummary(summaryData);
      setActiveSessions(sessionsData);
    } catch {
      setError('No se pudo cargar el resumen.');
    } finally {
      setLoading(false);
    }
  }, [from, to]);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <section className="stack">
      <div className="panel">
        <div className="filter-row">
          <div>
            <label htmlFor="summary-from">Desde</label>
            <input
              id="summary-from"
              type="datetime-local"
              value={from}
              onChange={(event) => setFrom(event.target.value)}
            />
          </div>
          <div>
            <label htmlFor="summary-to">Hasta</label>
            <input
              id="summary-to"
              type="datetime-local"
              value={to}
              onChange={(event) => setTo(event.target.value)}
            />
          </div>
          <button type="button" className="btn primary" onClick={() => void load()}>
            Actualizar
          </button>
        </div>
      </div>

      {loading && <div className="panel">Cargando...</div>}
      {error && <div className="panel error-text">{error}</div>}

      {summary && (
        <>
          <div className="cards-grid">
            <article className="card-accent">
              <h3>Eventos totales</h3>
              <strong>{summary.events_total}</strong>
            </article>
            <article className="card-accent">
              <h3>Sesiones iniciadas</h3>
              <strong>{summary.sessions_started}</strong>
            </article>
            <article className="card-accent">
              <h3>Sesiones cerradas</h3>
              <strong>{summary.sessions_ended}</strong>
            </article>
            <article className="card-accent">
              <h3>Sesiones activas</h3>
              <strong>{summary.active_sessions}</strong>
            </article>
            <article className="card-muted">
              <h3>Webhook pendientes</h3>
              <strong>{summary.webhook_pending}</strong>
            </article>
            <article className="card-muted">
              <h3>Webhook fallidos</h3>
              <strong>{summary.webhook_failed}</strong>
            </article>
            <article className="card-muted">
              <h3>Webhook entregados</h3>
              <strong>{summary.webhook_delivered}</strong>
            </article>
          </div>

          <div className="panel">
            <h2>Sesiones con presencia activa</h2>
            {activeSessions.length === 0 ? (
              <p>No hay sesiones activas en este momento.</p>
            ) : (
              <table>
                <thead>
                  <tr>
                    <th>Sesion</th>
                    <th>Participantes activos</th>
                    <th>Actualizado</th>
                  </tr>
                </thead>
                <tbody>
                  {activeSessions.map((item) => (
                    <tr key={item.session_id}>
                      <td>{item.session_id}</td>
                      <td>{item.active_participants}</td>
                      <td>{formatDateTime(item.updated_at)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>
        </>
      )}
    </section>
  );
}

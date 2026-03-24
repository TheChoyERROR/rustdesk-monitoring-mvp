import { useCallback, useEffect, useMemo, useState } from 'react';
import { Link } from 'react-router-dom';

import { apiHelpdeskAgents, apiHelpdeskSummary, apiHelpdeskTickets } from '../api';
import { formatDateTime } from '../lib/time';
import type {
  HelpdeskAgent,
  HelpdeskAgentStatus,
  HelpdeskOperationalSummary,
  HelpdeskTicket,
  HelpdeskTicketStatus,
} from '../types';

function statusLabel(status: HelpdeskTicketStatus | HelpdeskAgentStatus) {
  switch (status) {
    case 'new':
      return 'Nuevo';
    case 'queued':
      return 'En cola';
    case 'opening':
      return 'Abriendo';
    case 'in_progress':
      return 'En atencion';
    case 'resolved':
      return 'Resuelto';
    case 'cancelled':
      return 'Cancelado';
    case 'failed':
      return 'Fallido';
    case 'available':
      return 'Disponible';
    case 'busy':
      return 'Ocupado';
    case 'away':
      return 'Ausente';
    case 'offline':
      return 'Offline';
    default:
      return status;
  }
}

function statusClass(status: HelpdeskTicketStatus | HelpdeskAgentStatus) {
  switch (status) {
    case 'available':
    case 'resolved':
      return 'status-good';
    case 'opening':
    case 'new':
    case 'queued':
      return 'status-warn';
    case 'busy':
    case 'in_progress':
      return 'status-info';
    case 'cancelled':
    case 'failed':
    case 'offline':
      return 'status-bad';
    case 'away':
      return 'status-neutral';
    default:
      return 'status-neutral';
  }
}

export default function HelpdeskPage() {
  const [summary, setSummary] = useState<HelpdeskOperationalSummary | null>(null);
  const [agents, setAgents] = useState<HelpdeskAgent[]>([]);
  const [tickets, setTickets] = useState<HelpdeskTicket[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [ticketFilter, setTicketFilter] = useState<'all' | HelpdeskTicketStatus>('all');
  const [agentFilter, setAgentFilter] = useState<'all' | HelpdeskAgentStatus>('all');

  const load = useCallback(async (background = false) => {
    if (background) {
      setRefreshing(true);
    } else {
      setLoading(true);
    }
    setError(null);
    try {
      const [summaryData, agentsData, ticketsData] = await Promise.all([
        apiHelpdeskSummary(),
        apiHelpdeskAgents(),
        apiHelpdeskTickets(),
      ]);
      setSummary(summaryData);
      setAgents(agentsData);
      setTickets(ticketsData);
    } catch {
      setError('No se pudo cargar el modulo helpdesk.');
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      void load(true);
    }, 5000);
    return () => window.clearInterval(timer);
  }, [load]);

  const filteredTickets = useMemo(() => {
    return tickets.filter((ticket) => ticketFilter === 'all' || ticket.status === ticketFilter);
  }, [ticketFilter, tickets]);

  const filteredAgents = useMemo(() => {
    return agents.filter((agent) => agentFilter === 'all' || agent.status === agentFilter);
  }, [agentFilter, agents]);

  const activeQueue = summary
    ? summary.tickets_new + summary.tickets_queued + summary.tickets_opening
    : 0;

  return (
    <section className="stack">
      <div className="panel dashboard-section-header">
        <div>
          <h2>Operacion Helpdesk</h2>
          <p className="activity-summary-line">
            Cola activa, agentes y tickets en tiempo real.
          </p>
        </div>
        <button type="button" className="btn primary" onClick={() => void load()} disabled={refreshing}>
          {refreshing ? 'Actualizando...' : 'Actualizar'}
        </button>
      </div>

      {loading && <div className="panel">Cargando...</div>}
      {error && <div className="panel error-text">{error}</div>}

      {summary && (
        <>
          <div className="cards-grid">
            <article className="card-accent">
              <h3>Cola activa</h3>
              <strong>{activeQueue}</strong>
            </article>
            <article className="card-accent">
              <h3>En atencion</h3>
              <strong>{summary.tickets_in_progress}</strong>
            </article>
            <article className="card-muted">
              <h3>Agentes disponibles</h3>
              <strong>{summary.agents_available}</strong>
            </article>
            <article className="card-muted">
              <h3>Agentes ocupados</h3>
              <strong>{summary.agents_busy}</strong>
            </article>
            <article className="card-muted">
              <h3>Resueltos</h3>
              <strong>{summary.tickets_resolved}</strong>
            </article>
            <article className="card-muted">
              <h3>Fallidos</h3>
              <strong>{summary.tickets_failed}</strong>
            </article>
          </div>

          <div className="cards-grid">
            <article className="card-muted">
              <h3>Nuevos</h3>
              <strong>{summary.tickets_new}</strong>
            </article>
            <article className="card-muted">
              <h3>En cola</h3>
              <strong>{summary.tickets_queued}</strong>
            </article>
            <article className="card-muted">
              <h3>Abriendo</h3>
              <strong>{summary.tickets_opening}</strong>
            </article>
            <article className="card-muted">
              <h3>Cancelados</h3>
              <strong>{summary.tickets_cancelled}</strong>
            </article>
            <article className="card-muted">
              <h3>Agentes ausentes</h3>
              <strong>{summary.agents_away}</strong>
            </article>
            <article className="card-muted">
              <h3>Agentes offline</h3>
              <strong>{summary.agents_offline}</strong>
            </article>
          </div>
        </>
      )}

      <div className="panel">
        <div className="filter-row">
          <div>
            <label htmlFor="helpdesk-agent-filter">Estado de agente</label>
            <select
              id="helpdesk-agent-filter"
              value={agentFilter}
              onChange={(event) => setAgentFilter(event.target.value as 'all' | HelpdeskAgentStatus)}
            >
              <option value="all">Todos</option>
              <option value="available">Disponibles</option>
              <option value="opening">Abriendo</option>
              <option value="busy">Ocupados</option>
              <option value="away">Ausentes</option>
              <option value="offline">Offline</option>
            </select>
          </div>
        </div>

        <h2>Agentes</h2>
        {filteredAgents.length === 0 ? (
          <p>No hay agentes para el filtro seleccionado.</p>
        ) : (
          <table>
            <thead>
              <tr>
                <th>Agente</th>
                <th>Estado</th>
                <th>Ticket actual</th>
                <th>Ultimo heartbeat</th>
                <th>Actualizado</th>
              </tr>
            </thead>
            <tbody>
              {filteredAgents.map((agent) => (
                <tr key={agent.agent_id}>
                  <td>
                    <strong>{agent.display_name || agent.agent_id}</strong>
                    <div className="table-subtle">{agent.agent_id}</div>
                  </td>
                  <td>
                    <span className={`status-pill ${statusClass(agent.status)}`}>
                      {statusLabel(agent.status)}
                    </span>
                  </td>
                  <td>{agent.current_ticket_id || 'Sin ticket'}</td>
                  <td>{formatDateTime(agent.last_heartbeat_at)}</td>
                  <td>{formatDateTime(agent.updated_at)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>

      <div className="panel">
        <div className="filter-row">
          <div>
            <label htmlFor="helpdesk-ticket-filter">Estado de ticket</label>
            <select
              id="helpdesk-ticket-filter"
              value={ticketFilter}
              onChange={(event) => setTicketFilter(event.target.value as 'all' | HelpdeskTicketStatus)}
            >
              <option value="all">Todos</option>
              <option value="new">Nuevos</option>
              <option value="queued">En cola</option>
              <option value="opening">Abriendo</option>
              <option value="in_progress">En atencion</option>
              <option value="resolved">Resueltos</option>
              <option value="cancelled">Cancelados</option>
              <option value="failed">Fallidos</option>
            </select>
          </div>
        </div>

        <h2>Tickets</h2>
        {filteredTickets.length === 0 ? (
          <p>No hay tickets para el filtro seleccionado.</p>
        ) : (
          <table>
            <thead>
              <tr>
                <th>Ticket</th>
                <th>Cliente</th>
                <th>Estado</th>
                <th>Agente asignado</th>
                <th>Deadline</th>
                <th>Actualizado</th>
              </tr>
            </thead>
            <tbody>
              {filteredTickets.map((ticket) => (
                <tr key={ticket.ticket_id}>
                  <td>
                    <strong>
                      <Link to={`/helpdesk/tickets/${encodeURIComponent(ticket.ticket_id)}`}>
                        {ticket.ticket_id}
                      </Link>
                    </strong>
                    {ticket.summary ? <div className="table-subtle">{ticket.summary}</div> : null}
                  </td>
                  <td>
                    <strong>{ticket.client_display_name || ticket.client_id}</strong>
                    <div className="table-subtle">{ticket.client_id}</div>
                  </td>
                  <td>
                    <span className={`status-pill ${statusClass(ticket.status)}`}>
                      {statusLabel(ticket.status)}
                    </span>
                  </td>
                  <td>{ticket.assigned_agent_id || 'Sin asignar'}</td>
                  <td>{ticket.opening_deadline_at ? formatDateTime(ticket.opening_deadline_at) : '-'}</td>
                  <td>{formatDateTime(ticket.updated_at)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </section>
  );
}

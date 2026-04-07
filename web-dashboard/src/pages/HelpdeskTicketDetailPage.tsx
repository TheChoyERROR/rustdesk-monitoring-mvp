import { useCallback, useEffect, useMemo, useState } from 'react';
import { Link, useParams } from 'react-router-dom';

import {
  apiHelpdeskAgents,
  apiHelpdeskTicket,
  apiHelpdeskTicketAssign,
  apiHelpdeskTicketAudit,
  apiHelpdeskTicketCancel,
  apiHelpdeskTicketRequeue,
  apiHelpdeskTicketUpdateOperational,
} from '../api';
import { formatDateTime } from '../lib/time';
import type {
  HelpdeskAgent,
  HelpdeskAuditEvent,
  HelpdeskTicket,
  HelpdeskTicketStatus,
} from '../types';

function statusLabel(status: HelpdeskTicketStatus) {
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
    default:
      return status;
  }
}

function statusClass(status: HelpdeskTicketStatus) {
  switch (status) {
    case 'resolved':
      return 'status-good';
    case 'opening':
    case 'new':
    case 'queued':
      return 'status-warn';
    case 'in_progress':
      return 'status-info';
    case 'cancelled':
    case 'failed':
      return 'status-bad';
    default:
      return 'status-neutral';
  }
}

function difficultyLabel(rawDifficulty?: string | null) {
  switch ((rawDifficulty ?? '').trim().toLowerCase()) {
    case 'low':
      return 'Baja';
    case 'high':
      return 'Alta';
    case 'medium':
    default:
      return 'Media';
  }
}

function ticketHeadline(ticket: HelpdeskTicket): string {
  const title = ticket.title?.trim();
  if (title) {
    return title;
  }
  const summary = ticket.summary?.trim();
  if (summary) {
    return summary;
  }
  return ticket.ticket_id;
}

function auditPayloadToText(payload: HelpdeskAuditEvent['payload']) {
  if (!payload || Object.keys(payload).length === 0) {
    return '';
  }
  return JSON.stringify(payload, null, 2);
}

export default function HelpdeskTicketDetailPage() {
  const { ticketId = '' } = useParams();
  const decodedTicketId = useMemo(() => decodeURIComponent(ticketId), [ticketId]);

  const [ticket, setTicket] = useState<HelpdeskTicket | null>(null);
  const [agents, setAgents] = useState<HelpdeskAgent[]>([]);
  const [audit, setAudit] = useState<HelpdeskAuditEvent[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [actionReason, setActionReason] = useState('');
  const [nextAgentStatus, setNextAgentStatus] = useState<'available' | 'away'>('available');
  const [dispatchAgentId, setDispatchAgentId] = useState('');
  const [operationalDifficulty, setOperationalDifficulty] = useState('medium');
  const [operationalEstimatedMinutes, setOperationalEstimatedMinutes] = useState('30');
  const [operationalBusy, setOperationalBusy] = useState(false);
  const [actionBusy, setActionBusy] = useState<null | 'assign' | 'requeue' | 'cancel'>(null);

  const load = useCallback(async (background = false) => {
    if (background) {
      setRefreshing(true);
    } else {
      setLoading(true);
    }
    setError(null);
    try {
      const [ticketData, auditData, agentsData] = await Promise.all([
        apiHelpdeskTicket(decodedTicketId),
        apiHelpdeskTicketAudit(decodedTicketId, 200),
        apiHelpdeskAgents(),
      ]);
      setTicket(ticketData);
      setAudit(auditData);
      setAgents(agentsData);
    } catch {
      setError('No se pudo cargar el detalle del ticket.');
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, [decodedTicketId]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      void load(true);
    }, 5000);
    return () => window.clearInterval(timer);
  }, [load]);

  const availableAgents = useMemo(() => {
    return agents.filter((agent) => agent.status === 'available');
  }, [agents]);

  useEffect(() => {
    if (dispatchAgentId !== '' && !availableAgents.some((agent) => agent.agent_id === dispatchAgentId)) {
      setDispatchAgentId('');
    }
  }, [availableAgents, dispatchAgentId]);

  useEffect(() => {
    if (!ticket) {
      return;
    }
    setOperationalDifficulty((ticket.difficulty ?? 'medium').trim().toLowerCase() || 'medium');
    setOperationalEstimatedMinutes(ticket.estimated_minutes ? String(ticket.estimated_minutes) : '30');
  }, [ticket]);

  return (
    <section className="stack">
      <div className="panel dashboard-section-header">
        <div>
          <p className="activity-summary-line">
            <Link to="/helpdesk">Helpdesk</Link> / Ticket
          </p>
          <h2>{ticket ? ticketHeadline(ticket) : decodedTicketId}</h2>
          {ticket ? <p className="activity-summary-line">{ticket.ticket_id}</p> : null}
        </div>
        <button type="button" className="btn secondary" onClick={() => void load()} disabled={refreshing}>
          {refreshing ? 'Actualizando...' : 'Refrescar'}
        </button>
      </div>

      {loading && <div className="panel">Cargando...</div>}
      {error && <div className="panel error-text">{error}</div>}

      {!loading && !ticket && !error && <div className="panel">Ticket no encontrado.</div>}

      {ticket && (
        <>
          <div className="panel">
            <div className="detail-actions">
              <div className="filter-grid">
                <div>
                  <label htmlFor="ticket-dispatch-agent">Despachar a</label>
                  <select
                    id="ticket-dispatch-agent"
                    value={dispatchAgentId}
                    onChange={(event) => setDispatchAgentId(event.target.value)}
                  >
                    <option value="">Selecciona un agente</option>
                    {availableAgents.map((agent) => (
                      <option key={agent.agent_id} value={agent.agent_id}>
                        {agent.display_name || agent.agent_id} ({agent.agent_id})
                      </option>
                    ))}
                  </select>
                </div>
                <div>
                  <label htmlFor="ticket-next-agent-status">Estado del agente al liberar</label>
                  <select
                    id="ticket-next-agent-status"
                    value={nextAgentStatus}
                    onChange={(event) => setNextAgentStatus(event.target.value as 'available' | 'away')}
                  >
                    <option value="available">Disponible</option>
                    <option value="away">Ausente</option>
                  </select>
                </div>
                <div>
                  <label htmlFor="ticket-action-reason">Motivo operativo</label>
                  <input
                    id="ticket-action-reason"
                    value={actionReason}
                    onChange={(event) => setActionReason(event.target.value)}
                    placeholder="reintento, falso positivo, operador caido..."
                  />
                </div>
              </div>
              <div className="filter-actions">
                <button
                  type="button"
                  className="btn primary"
                  disabled={
                    actionBusy !== null ||
                    ticket.status !== 'queued' ||
                    availableAgents.length === 0 ||
                    dispatchAgentId === ''
                  }
                  onClick={async () => {
                    setActionBusy('assign');
                    setError(null);
                    try {
                      await apiHelpdeskTicketAssign(decodedTicketId, {
                        agent_id: dispatchAgentId,
                        reason: actionReason.trim() || undefined,
                      });
                      await load();
                    } catch {
                      setError('No se pudo despachar el ticket.');
                    } finally {
                      setActionBusy(null);
                    }
                  }}
                >
                  {actionBusy === 'assign' ? 'Despachando...' : 'Despachar ahora'}
                </button>
                <button
                  type="button"
                  className="btn secondary"
                  disabled={actionBusy !== null || ticket.status === 'resolved'}
                  onClick={async () => {
                    setActionBusy('requeue');
                    setError(null);
                    try {
                      await apiHelpdeskTicketRequeue(decodedTicketId, {
                        next_agent_status: nextAgentStatus,
                        reason: actionReason.trim() || undefined,
                      });
                      await load();
                    } catch {
                      setError('No se pudo reencolar el ticket.');
                    } finally {
                      setActionBusy(null);
                    }
                  }}
                >
                  {actionBusy === 'requeue' ? 'Reencolando...' : 'Reencolar'}
                </button>
                <button
                  type="button"
                  className="btn secondary"
                  disabled={actionBusy !== null || ticket.status === 'resolved' || ticket.status === 'cancelled'}
                  onClick={async () => {
                    setActionBusy('cancel');
                    setError(null);
                    try {
                      await apiHelpdeskTicketCancel(decodedTicketId, {
                        next_agent_status: nextAgentStatus,
                        reason: actionReason.trim() || undefined,
                      });
                      await load();
                    } catch {
                      setError('No se pudo cancelar el ticket.');
                    } finally {
                      setActionBusy(null);
                    }
                  }}
                >
                  {actionBusy === 'cancel' ? 'Cancelando...' : 'Cancelar'}
                </button>
              </div>
            </div>
            <div className="detail-block">
              <label>Campos operativos</label>
              <div className="filter-grid">
                <div>
                  <label htmlFor="ticket-operational-difficulty">Dificultad</label>
                  <select
                    id="ticket-operational-difficulty"
                    value={operationalDifficulty}
                    onChange={(event) => setOperationalDifficulty(event.target.value)}
                    disabled={operationalBusy}
                  >
                    <option value="low">Baja</option>
                    <option value="medium">Media</option>
                    <option value="high">Alta</option>
                  </select>
                </div>
                <div>
                  <label htmlFor="ticket-operational-estimated">Tiempo estimado (min)</label>
                  <input
                    id="ticket-operational-estimated"
                    type="number"
                    min="1"
                    step="1"
                    value={operationalEstimatedMinutes}
                    onChange={(event) => setOperationalEstimatedMinutes(event.target.value)}
                    disabled={operationalBusy}
                  />
                </div>
              </div>
              <div className="filter-actions">
                <button
                  type="button"
                  className="btn secondary"
                  disabled={operationalBusy}
                  onClick={async () => {
                    setOperationalBusy(true);
                    setError(null);
                    try {
                      await apiHelpdeskTicketUpdateOperational(decodedTicketId, {
                        difficulty: operationalDifficulty,
                        estimated_minutes:
                          Number.parseInt(operationalEstimatedMinutes.trim(), 10) || undefined,
                      });
                      await load();
                    } catch {
                      setError('No se pudieron guardar los campos operativos.');
                    } finally {
                      setOperationalBusy(false);
                    }
                  }}
                >
                  {operationalBusy ? 'Guardando...' : 'Guardar campos operativos'}
                </button>
                <p className="activity-summary-line">
                  Dificultad y tiempo estimado deben ser definidos por quien atiende o coordina el ticket.
                </p>
              </div>
            </div>
            <div className="detail-grid">
              <div>
                <label>Estado</label>
                <div>
                  <span className={`status-pill ${statusClass(ticket.status)}`}>
                    {statusLabel(ticket.status)}
                  </span>
                </div>
              </div>
              <div>
                <label>Cliente</label>
                <div>{ticket.client_display_name || ticket.client_id}</div>
                <div className="table-subtle">{ticket.client_id}</div>
              </div>
              <div>
                <label>Agente asignado</label>
                <div>{ticket.assigned_agent_id || 'Sin asignar'}</div>
              </div>
              <div>
                <label>Dificultad</label>
                <div>{ticket.difficulty ? difficultyLabel(ticket.difficulty) : '-'}</div>
              </div>
              <div>
                <label>Tiempo estimado</label>
                <div>{ticket.estimated_minutes ? `${ticket.estimated_minutes} min` : '-'}</div>
              </div>
              <div>
                <label>Deadline de apertura</label>
                <div>{ticket.opening_deadline_at ? formatDateTime(ticket.opening_deadline_at) : '-'}</div>
              </div>
              <div>
                <label>Creado</label>
                <div>{formatDateTime(ticket.created_at)}</div>
              </div>
              <div>
                <label>Actualizado</label>
                <div>{formatDateTime(ticket.updated_at)}</div>
              </div>
            </div>
            {ticket.title ? (
              <div className="detail-block">
                <label>Titulo</label>
                <div>{ticket.title}</div>
              </div>
            ) : null}
            {ticket.description ? (
              <div className="detail-block">
                <label>Descripcion</label>
                <div>{ticket.description}</div>
              </div>
            ) : null}
            {!ticket.title && ticket.summary ? (
              <div className="detail-block">
                <label>Resumen</label>
                <div>{ticket.summary}</div>
              </div>
            ) : null}
            {ticket.latest_agent_report ? (
              <div className="detail-block">
                <label>Ultimo reporte de soporte</label>
                <div>{ticket.latest_agent_report}</div>
                <div className="table-subtle">
                  {ticket.latest_agent_report_by || 'Agente'}
                  {ticket.latest_agent_report_at
                    ? ` · ${formatDateTime(ticket.latest_agent_report_at)}`
                    : ''}
                </div>
              </div>
            ) : null}
            {ticket.requested_by ? (
              <div className="detail-block">
                <label>Solicitado por</label>
                <div>{ticket.requested_by}</div>
              </div>
            ) : null}
            {ticket.device_id ? (
              <div className="detail-block">
                <label>Device ID</label>
                <div>{ticket.device_id}</div>
              </div>
            ) : null}
          </div>

          <div className="panel">
            <h3>Auditoria</h3>
            {audit.length === 0 ? (
              <p>Sin eventos de auditoria para este ticket.</p>
            ) : (
              <div className="audit-list">
                {audit.map((event, index) => (
                  <article key={`${event.entity_id}-${event.event_type}-${event.created_at}-${index}`} className="audit-item">
                    <div className="audit-item-header">
                      <strong>{event.event_type}</strong>
                      <span>{formatDateTime(event.created_at)}</span>
                    </div>
                    <div className="table-subtle">
                      {event.entity_type} / {event.entity_id}
                    </div>
                    {event.payload ? (
                      <pre className="audit-payload">{auditPayloadToText(event.payload)}</pre>
                    ) : null}
                  </article>
                ))}
              </div>
            )}
          </div>
        </>
      )}
    </section>
  );
}

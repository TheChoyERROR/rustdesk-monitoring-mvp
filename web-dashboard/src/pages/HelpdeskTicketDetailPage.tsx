import { useCallback, useEffect, useMemo, useState } from 'react';
import { Link, useParams } from 'react-router-dom';

import {
  apiHelpdeskAgents,
  apiHelpdeskTicket,
  apiHelpdeskTicketAssign,
  apiHelpdeskTicketAudit,
  apiHelpdeskTicketCancel,
  apiHelpdeskTicketRequeue,
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
  const [dispatchAgentId, setDispatchAgentId] = useState<'auto' | string>('auto');
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
    if (
      dispatchAgentId !== 'auto' &&
      !availableAgents.some((agent) => agent.agent_id === dispatchAgentId)
    ) {
      setDispatchAgentId('auto');
    }
  }, [availableAgents, dispatchAgentId]);

  return (
    <section className="stack">
      <div className="panel dashboard-section-header">
        <div>
          <p className="activity-summary-line">
            <Link to="/helpdesk">Helpdesk</Link> / Ticket
          </p>
          <h2>{decodedTicketId}</h2>
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
                    <option value="auto">Primer agente disponible</option>
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
                  disabled={actionBusy !== null || ticket.status !== 'queued' || availableAgents.length === 0}
                  onClick={async () => {
                    setActionBusy('assign');
                    setError(null);
                    try {
                      await apiHelpdeskTicketAssign(decodedTicketId, {
                        agent_id: dispatchAgentId === 'auto' ? undefined : dispatchAgentId,
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
            {ticket.summary ? (
              <div className="detail-block">
                <label>Resumen</label>
                <div>{ticket.summary}</div>
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

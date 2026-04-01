import { useCallback, useEffect, useMemo, useState, type FormEvent } from 'react';
import { Link } from 'react-router-dom';

import {
  apiHelpdeskAgents,
  apiHelpdeskAuthorizedAgentDelete,
  apiHelpdeskAuthorizedAgentUpsert,
  apiHelpdeskAuthorizedAgents,
  apiHelpdeskCreateTicket,
  apiHelpdeskSummary,
  apiHelpdeskTickets,
} from '../api';
import { formatDateTime } from '../lib/time';
import type {
  HelpdeskAgent,
  HelpdeskAuthorizedAgent,
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

function agentName(agent: HelpdeskAgent): string {
  const displayName = agent.display_name?.trim();
  if (displayName) {
    return displayName;
  }
  return agent.agent_id;
}

function agentInitial(agent: HelpdeskAgent): string {
  return agentName(agent).charAt(0).toUpperCase();
}

function hasAvatarUrl(agent: HelpdeskAgent): agent is HelpdeskAgent & { avatar_url: string } {
  return typeof agent.avatar_url === 'string' && agent.avatar_url.trim().length > 0;
}

type CreateFeedback =
  | {
      tone: 'success' | 'error';
      message: string;
    }
  | null;

export default function HelpdeskPage() {
  const [summary, setSummary] = useState<HelpdeskOperationalSummary | null>(null);
  const [agents, setAgents] = useState<HelpdeskAgent[]>([]);
  const [authorizedAgents, setAuthorizedAgents] = useState<HelpdeskAuthorizedAgent[]>([]);
  const [tickets, setTickets] = useState<HelpdeskTicket[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [ticketFilter, setTicketFilter] = useState<'all' | HelpdeskTicketStatus>('all');
  const [agentFilter, setAgentFilter] = useState<'all' | HelpdeskAgentStatus>('all');
  const [failedAvatarIds, setFailedAvatarIds] = useState<Record<string, boolean>>({});
  const [createBusy, setCreateBusy] = useState(false);
  const [createFeedback, setCreateFeedback] = useState<CreateFeedback>(null);
  const [authorizeBusy, setAuthorizeBusy] = useState(false);
  const [authorizeFeedback, setAuthorizeFeedback] = useState<CreateFeedback>(null);
  const [authorizedForm, setAuthorizedForm] = useState({
    agentId: '',
    displayName: '',
  });
  const [createForm, setCreateForm] = useState({
    clientId: '',
    clientDisplayName: '',
    title: '',
    description: '',
    difficulty: 'medium',
    estimatedMinutes: '30',
    preferredAgentId: 'auto',
  });

  const load = useCallback(async (background = false) => {
    if (background) {
      setRefreshing(true);
    } else {
      setLoading(true);
    }
    setError(null);
    try {
      const [summaryData, agentsData, authorizedAgentsData, ticketsData] = await Promise.all([
        apiHelpdeskSummary(),
        apiHelpdeskAgents(),
        apiHelpdeskAuthorizedAgents(),
        apiHelpdeskTickets(),
      ]);
      setSummary(summaryData);
      setAgents(agentsData);
      setAuthorizedAgents(authorizedAgentsData);
      setTickets(ticketsData);
      setFailedAvatarIds({});
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

  const availableAgents = useMemo(() => {
    return agents.filter((agent) => agent.status === 'available');
  }, [agents]);

  const authorizedAgentIds = useMemo(() => {
    return new Set(authorizedAgents.map((agent) => agent.agent_id));
  }, [authorizedAgents]);

  useEffect(() => {
    if (
      createForm.preferredAgentId !== 'auto' &&
      !availableAgents.some((agent) => agent.agent_id === createForm.preferredAgentId)
    ) {
      setCreateForm((current) => ({ ...current, preferredAgentId: 'auto' }));
    }
  }, [availableAgents, createForm.preferredAgentId]);

  const activeQueue = summary
    ? summary.tickets_new + summary.tickets_queued + summary.tickets_opening
    : 0;

  const handleAuthorizeAgent = useCallback(
    async (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      setAuthorizeBusy(true);
      setAuthorizeFeedback(null);
      setError(null);

      try {
        const agent = await apiHelpdeskAuthorizedAgentUpsert({
          agent_id: authorizedForm.agentId.trim(),
          display_name: authorizedForm.displayName.trim() || undefined,
        });
        setAuthorizeFeedback({
          tone: 'success',
          message: `El equipo ${agent.agent_id} quedó habilitado para usar modo agente.`,
        });
        setAuthorizedForm({ agentId: '', displayName: '' });
        await load(true);
      } catch (authorizeError) {
        setAuthorizeFeedback({
          tone: 'error',
          message:
            authorizeError instanceof Error
              ? authorizeError.message
              : 'No se pudo autorizar el agente.',
        });
      } finally {
        setAuthorizeBusy(false);
      }
    },
    [authorizedForm, load],
  );

  const handleRemoveAuthorizedAgent = useCallback(
    async (agentId: string) => {
      setAuthorizeBusy(true);
      setAuthorizeFeedback(null);
      setError(null);

      try {
        await apiHelpdeskAuthorizedAgentDelete(agentId);
        setAuthorizeFeedback({
          tone: 'success',
          message: `El equipo ${agentId} dejó de estar autorizado como agente.`,
        });
        await load(true);
      } catch (removeError) {
        setAuthorizeFeedback({
          tone: 'error',
          message:
            removeError instanceof Error
              ? removeError.message
              : 'No se pudo quitar la autorización del agente.',
        });
      } finally {
        setAuthorizeBusy(false);
      }
    },
    [load],
  );

  const handleCreateTicket = useCallback(
    async (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      setCreateBusy(true);
      setCreateFeedback(null);
      setError(null);

      const preferredAgentId =
        createForm.preferredAgentId !== 'auto' ? createForm.preferredAgentId : undefined;
      const selectedAgent = preferredAgentId
        ? agents.find((agent) => agent.agent_id === preferredAgentId) ?? null
        : null;

      try {
        const ticket = await apiHelpdeskCreateTicket({
          client_id: createForm.clientId.trim(),
          client_display_name: createForm.clientDisplayName.trim() || undefined,
          title: createForm.title.trim() || undefined,
          description: createForm.description.trim() || undefined,
          difficulty: createForm.difficulty.trim() || undefined,
          estimated_minutes:
            Number.parseInt(createForm.estimatedMinutes.trim(), 10) || undefined,
          summary: createForm.title.trim() || undefined,
          preferred_agent_id: preferredAgentId,
        });

        const assignedAgentId = ticket.assigned_agent_id?.trim() || '';
        const assignedAgent =
          assignedAgentId !== ''
            ? agents.find((agent) => agent.agent_id === assignedAgentId) ?? null
            : null;

        if (ticket.status === 'opening' && assignedAgentId !== '') {
          setCreateFeedback({
            tone: 'success',
            message: `Ticket ${ticket.ticket_id} enviado a ${
              assignedAgent ? agentName(assignedAgent) : assignedAgentId
            }. La app del agente intentará conectarse automáticamente al equipo ${ticket.client_id}.`,
          });
        } else if (selectedAgent) {
          setCreateFeedback({
            tone: 'success',
            message: `Ticket ${ticket.ticket_id} creado en cola. ${agentName(
              selectedAgent,
            )} ya no estaba disponible para recibirlo.`,
          });
        } else {
          setCreateFeedback({
            tone: 'success',
            message: `Ticket ${ticket.ticket_id} creado en cola. Se despachará cuando haya un agente disponible.`,
          });
        }

        setCreateForm((current) => ({
          ...current,
          clientId: '',
          clientDisplayName: '',
          title: '',
          description: '',
          estimatedMinutes: '30',
        }));
        await load(true);
      } catch (createError) {
        setCreateFeedback({
          tone: 'error',
          message: createError instanceof Error ? createError.message : 'No se pudo crear el ticket.',
        });
      } finally {
        setCreateBusy(false);
      }
    },
    [agents, createForm, load],
  );

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
        <div>
          <h2>Equipos autorizados como agentes</h2>
          <p className="activity-summary-line">
            El switch local ya no basta por sí solo. Solo los RustDesk ID autorizados aquí pueden
            publicar presencia de operador y recibir tickets.
          </p>
        </div>

        <form className="stack" onSubmit={(event) => void handleAuthorizeAgent(event)}>
          <div className="filter-grid">
            <div>
              <label htmlFor="authorized-agent-id">RustDesk ID del agente</label>
              <input
                id="authorized-agent-id"
                value={authorizedForm.agentId}
                onChange={(event) =>
                  setAuthorizedForm((current) => ({ ...current, agentId: event.target.value }))
                }
                placeholder="419797027"
                required
              />
            </div>
            <div>
              <label htmlFor="authorized-agent-name">Nombre visible</label>
              <input
                id="authorized-agent-name"
                value={authorizedForm.displayName}
                onChange={(event) =>
                  setAuthorizedForm((current) => ({
                    ...current,
                    displayName: event.target.value,
                  }))
                }
                placeholder="Edward soporte"
              />
            </div>
          </div>

          <div className="filter-actions">
            <button type="submit" className="btn primary" disabled={authorizeBusy}>
              {authorizeBusy ? 'Guardando...' : 'Autorizar agente'}
            </button>
            {authorizeFeedback ? (
              <p className={authorizeFeedback.tone === 'error' ? 'error-text' : 'success-text'}>
                {authorizeFeedback.message}
              </p>
            ) : (
              <p className="activity-summary-line">
                Si un cliente final activa el switch por error, no aparecerá como agente si su
                RustDesk ID no está autorizado aquí.
              </p>
            )}
          </div>
        </form>

        {authorizedAgents.length === 0 ? (
          <p>Aún no hay equipos autorizados como agentes.</p>
        ) : (
          <table>
            <thead>
              <tr>
                <th>Agente autorizado</th>
                <th>Estado actual</th>
                <th>Autorizado desde</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {authorizedAgents.map((authorizedAgent) => {
                const liveAgent =
                  agents.find((agent) => agent.agent_id === authorizedAgent.agent_id) ?? null;
                const label = authorizedAgent.display_name?.trim() || authorizedAgent.agent_id;
                return (
                  <tr key={authorizedAgent.agent_id}>
                    <td>
                      <strong>{label}</strong>
                      <div className="table-subtle">{authorizedAgent.agent_id}</div>
                    </td>
                    <td>
                      {liveAgent ? (
                        <span className={`status-pill ${statusClass(liveAgent.status)}`}>
                          {statusLabel(liveAgent.status)}
                        </span>
                      ) : (
                        <span className="status-pill status-neutral">Sin presencia</span>
                      )}
                    </td>
                    <td>{formatDateTime(authorizedAgent.created_at)}</td>
                    <td>
                      <button
                        type="button"
                        className="btn secondary"
                        disabled={authorizeBusy}
                        onClick={() => void handleRemoveAuthorizedAgent(authorizedAgent.agent_id)}
                      >
                        Quitar
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>

      <div className="panel">
        <div>
          <h2>Crear ticket</h2>
          <p className="activity-summary-line">
            Crea el ticket desde la web usando el RustDesk ID de la maquina. Ahora puedes registrar
            titulo, descripcion, dificultad y tiempo estimado antes de despacharlo.
          </p>
          <p className="activity-summary-line">
            Agentes disponibles ahora: <strong>{availableAgents.length}</strong>
          </p>
        </div>

        <form className="stack" onSubmit={(event) => void handleCreateTicket(event)}>
          <div className="filter-grid">
            <div>
              <label htmlFor="helpdesk-client-id">RustDesk ID / Machine ID</label>
              <input
                id="helpdesk-client-id"
                value={createForm.clientId}
                onChange={(event) =>
                  setCreateForm((current) => ({ ...current, clientId: event.target.value }))
                }
                placeholder="419797027"
                required
              />
            </div>
            <div>
              <label htmlFor="helpdesk-client-display-name">Nombre visible</label>
              <input
                id="helpdesk-client-display-name"
                value={createForm.clientDisplayName}
                onChange={(event) =>
                  setCreateForm((current) => ({
                    ...current,
                    clientDisplayName: event.target.value,
                  }))
                }
                placeholder="PC Contabilidad"
              />
            </div>
            <div>
              <label htmlFor="helpdesk-preferred-agent">Despachar a</label>
              <select
                id="helpdesk-preferred-agent"
                value={createForm.preferredAgentId}
                onChange={(event) =>
                  setCreateForm((current) => ({
                    ...current,
                    preferredAgentId: event.target.value,
                  }))
                }
              >
                <option value="auto">Primer agente disponible</option>
                {availableAgents.map((agent) => (
                  <option key={agent.agent_id} value={agent.agent_id}>
                    {agentName(agent)} ({agent.agent_id})
                  </option>
                ))}
              </select>
            </div>
            <div>
              <label htmlFor="helpdesk-title">Titulo</label>
              <input
                id="helpdesk-title"
                value={createForm.title}
                onChange={(event) =>
                  setCreateForm((current) => ({ ...current, title: event.target.value }))
                }
                placeholder="No puede abrir el sistema contable"
                required
              />
            </div>
            <div>
              <label htmlFor="helpdesk-difficulty">Dificultad</label>
              <select
                id="helpdesk-difficulty"
                value={createForm.difficulty}
                onChange={(event) =>
                  setCreateForm((current) => ({
                    ...current,
                    difficulty: event.target.value,
                  }))
                }
              >
                <option value="low">Baja</option>
                <option value="medium">Media</option>
                <option value="high">Alta</option>
              </select>
            </div>
            <div>
              <label htmlFor="helpdesk-estimated">Tiempo aprox. (min)</label>
              <input
                id="helpdesk-estimated"
                type="number"
                min="1"
                step="1"
                value={createForm.estimatedMinutes}
                onChange={(event) =>
                  setCreateForm((current) => ({
                    ...current,
                    estimatedMinutes: event.target.value,
                  }))
                }
                placeholder="30"
                required
              />
            </div>
          </div>

          <div>
            <label htmlFor="helpdesk-description">Descripcion</label>
            <textarea
              id="helpdesk-description"
              value={createForm.description}
              onChange={(event) =>
                setCreateForm((current) => ({
                  ...current,
                  description: event.target.value,
                }))
              }
              placeholder="Describe claramente lo que el cliente necesita y cualquier error visible."
              rows={4}
              required
            />
          </div>

          <div className="filter-actions">
            <button type="submit" className="btn primary" disabled={createBusy}>
              {createBusy ? 'Creando...' : 'Crear y despachar'}
            </button>
            {createFeedback ? (
              <p className={createFeedback.tone === 'error' ? 'error-text' : 'success-text'}>
                {createFeedback.message}
              </p>
            ) : (
              <p className="activity-summary-line">
                El agente asignado intentará iniciar la conexión remota automáticamente desde su app.
              </p>
            )}
          </div>
        </form>
      </div>

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
                    <div className="table-identity">
                      {hasAvatarUrl(agent) && !failedAvatarIds[agent.agent_id] ? (
                        <img
                          src={agent.avatar_url}
                          alt={`Avatar de ${agentName(agent)}`}
                          className="participant-avatar"
                          loading="lazy"
                          onError={() => {
                            setFailedAvatarIds((prev) => ({
                              ...prev,
                              [agent.agent_id]: true,
                            }));
                          }}
                        />
                      ) : (
                        <div className="participant-avatar-fallback" aria-hidden="true">
                          {agentInitial(agent)}
                        </div>
                      )}
                      <div>
                        <strong>{agentName(agent)}</strong>
                        <div className="table-subtle">{agent.agent_id}</div>
                        {!authorizedAgentIds.has(agent.agent_id) && (
                          <div className="table-subtle">Sin autorización de operador</div>
                        )}
                      </div>
                    </div>
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
                    {ticket.title ? <div className="table-subtle">{ticket.title}</div> : null}
                    {!ticket.title && ticket.summary ? (
                      <div className="table-subtle">{ticket.summary}</div>
                    ) : null}
                  </td>
                  <td>
                    <strong>{ticket.client_display_name || ticket.client_id}</strong>
                    <div className="table-subtle">{ticket.client_id}</div>
                    {ticket.difficulty ? (
                      <div className="table-subtle">
                        {difficultyLabel(ticket.difficulty)}
                        {ticket.estimated_minutes ? ` · ${ticket.estimated_minutes} min` : ''}
                      </div>
                    ) : null}
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

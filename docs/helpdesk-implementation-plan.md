# Plan de Implementacion Helpdesk

## Fase 1. Contrato funcional minimo
- Consolidar los documentos de requisitos y estados como referencia del proyecto.
- Adoptar nomenclatura fija en codigo y API:
  - `agent`
  - `ticket`
  - `assignment`
  - `remote_session`
  - `heartbeat`
- Mantener fuera de alcance `multi-tenant`.

## Fase 2. Modelo de datos
- Crear tablas para `agents`, `helpdesk_tickets`, `ticket_assignments`, `agent_heartbeats`.
- Crear tabla de auditoria ligera para eventos de dominio.
- Anadir indices por:
  - estado de agente
  - estado de ticket
  - `created_at`
  - `assigned_agent_id`

## Fase 3. Backend operativo
- Endpoint para solicitar ayuda desde cliente final.
- Endpoint para activar/desactivar agente.
- Endpoint heartbeat de agente.
- Endpoint para consultar o consumir asignaciones activas.
- Endpoint para iniciar y cerrar sesion remota.
- Worker de asignacion al primer agente `available`.

## Fase 4. Logica de asignacion
- Seleccionar el primer agente `available`.
- Mover agente a `opening`.
- Mover ticket a `assigned/opening`.
- Lanzar countdown de 10 segundos.
- Si todo va bien, abrir sesion y pasar a `busy/in_progress`.
- Si falla, devolver ticket a `queued`.

## Fase 5. Integracion fork RustDesk

### Cliente final
- Boton visible `Solicitar helpdesk`.
- Identificador estable del dispositivo o peer.
- Envio de solicitud al backend.

### Cliente operador
- Toggle `Disponible / Ausente`.
- Heartbeat mientras esta activo.
- Aviso de asignacion con cuenta atras de 10 segundos.
- Apertura automatica de la sesion remota.

## Fase 6. Dashboard minimo
- Lista de tickets:
  - nuevos
  - en cola
  - en apertura
  - en curso
  - resueltos
- Lista de agentes:
  - offline
  - available
  - opening
  - busy
  - away
- Vista basica de auditoria por ticket.

## Fase 7. Reglas de robustez
- Evitar doble asignacion concurrente.
- Expirar heartbeats.
- Reencolar tickets si un agente desaparece antes de iniciar sesion.
- Registrar fallos de apertura.

## Fase 8. Auditoria ligera
- Guardar eventos minimos de negocio.
- Hacerlos visibles en dashboard o via API.
- Mantenerlos desacoplados del flujo critico.

## Orden recomendado de implementacion
1. Esquema SQLite y repositorios.
2. Estados y transiciones en backend.
3. Endpoints helpdesk.
4. Worker de asignacion.
5. Integracion cliente operador.
6. Integracion cliente final.
7. Dashboard operativo minimo.
8. Auditoria ligera.

## Criterios de aceptacion MVP
- Un cliente puede solicitar ayuda.
- Si hay un agente `available`, recibe una asignacion.
- El agente ve aviso de 10 segundos.
- La sesion se abre automaticamente.
- El agente pasa a `busy`.
- Al cerrar, ticket y agente vuelven al estado correcto.
- Si no hay agentes, el ticket queda en `queued`.

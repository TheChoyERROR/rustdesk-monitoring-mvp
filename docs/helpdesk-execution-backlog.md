# Backlog de Ejecucion Helpdesk

## Objetivo
Convertir el estado actual del MVP en un helpdesk operativo basado en el fork de RustDesk, manteniendo el backend Rust + SQLite y separando claramente:
- backend central;
- fork cliente final;
- fork operador;
- dashboard operativo;
- hardening y despliegue.

## Fase 1. Backend: ciclo de vida operativo
Estado: `en curso`

### Bloque 1.1 Transiciones explicitas de ticket/agente
- Crear endpoint para pasar una asignacion de `opening` a `in_progress`.
- Crear endpoint para cerrar una atencion y mover ticket a `resolved`.
- Permitir devolver el agente a `available` o `away` al cerrar.
- Registrar auditoria minima en cada cambio de estado.

Criterio de salida:
- un ticket asignado puede abrirse formalmente;
- un ticket en curso puede cerrarse formalmente;
- el agente no queda bloqueado tras cerrar.

### Bloque 1.2 Reconciliacion automatica
- Expirar tickets en `opening` cuando venza `opening_deadline_at`.
- Reencolar tickets si el agente desaparece antes de abrir la sesion.
- Marcar agentes como `offline` por heartbeat vencido.
- Resolver de forma consistente el estado de tickets afectados por agentes caidos.

Criterio de salida:
- no quedan tickets indefinidamente en `opening`;
- un agente caido no sigue apareciendo como disponible.

### Bloque 1.3 API de lectura operativa
- Exponer detalle de ticket por `ticket_id`.
- Exponer auditoria minima por ticket.
- Exponer resumen operativo de cola/agentes para dashboard y fork.

Criterio de salida:
- dashboard y clientes pueden consultar el estado sin leer la base de datos.

## Fase 2. Fork operador RustDesk
Estado: `pendiente`

### Bloque 2.1 Modo operador
- Pantalla o modo visible de operador.
- Toggle `Disponible / Ausente`.
- Heartbeat periodico al backend.
- Polling o SSE de asignacion activa.

### Bloque 2.2 Apertura operativa
- Modal o banner con countdown de 10 segundos.
- Apertura automatica de sesion remota sobre el peer asignado.
- Notificacion al backend cuando la sesion arranca y cuando termina.

Criterio de salida:
- un operador activo recibe una asignacion y entra en sesion sin pasos manuales externos.

## Fase 3. Fork cliente final RustDesk
Estado: `pendiente`

### Bloque 3.1 Branding helpdesk
- Texto visible tipo `Helpdesk activado`.
- Identidad corporativa y mensajes legales.
- Empaquetado diferenciado del RustDesk generalista.

### Bloque 3.2 Solicitud de ayuda
- Boton visible `Solicitar helpdesk`.
- Creacion de ticket contra backend con identificador estable del equipo.
- Estado visible: en cola, asignado, conectado, finalizado.

Criterio de salida:
- un usuario puede pedir ayuda desde el cliente corporativo sin usar el dashboard.

## Fase 4. Conexion automatica ticket <-> sesion remota
Estado: `pendiente`

### Bloque 4.1 Mapeo tecnico
- Definir de forma estable como se obtiene el `peer_id` o identificador remoto del cliente.
- Definir payload minimo de asignacion para que el operador abra la sesion correcta.
- Sincronizar inicio/fin real de sesion con el backend.

### Bloque 4.2 Recuperacion y errores
- Timeout de apertura.
- Peer no disponible.
- Reintento o reencolado.

Criterio de salida:
- una asignacion backend termina en una conexion remota real o en una salida controlada.

## Fase 5. Dashboard operativo
Estado: `pendiente`

### Bloque 5.1 Vista de tickets
- Tickets `queued`, `opening`, `in_progress`, `resolved`, `failed`, `cancelled`.
- Filtros por cliente, agente, estado y fecha.

### Bloque 5.2 Vista de agentes
- Agentes `offline`, `available`, `opening`, `busy`, `away`.
- Ultimo heartbeat, ticket actual y carga visible.

### Bloque 5.3 Acciones supervisor
- Reencolar ticket.
- Cerrar ticket.
- Marcar agente fuera de servicio.

Criterio de salida:
- el supervisor puede operar el helpdesk sin mirar logs ni SQLite.

## Fase 6. Hardening
Estado: `pendiente`

- Tests de concurrencia en asignacion.
- Tests de expiracion de heartbeats y countdown.
- Metricas helpdesk.
- Retencion y consulta de auditoria.
- Preparacion para migrar de SQLite si la carga lo exige.

## Orden de implementacion
1. Backend: transiciones y reconciliacion.
2. Fork operador.
3. Fork cliente final.
4. Conexion automatica ticket/sesion.
5. Dashboard operativo.
6. Hardening.

## Siguiente entrega
- cerrar backend `opening -> in_progress -> resolved`;
- expirar `opening` vencidos;
- marcar agentes `offline` por heartbeat vencido.

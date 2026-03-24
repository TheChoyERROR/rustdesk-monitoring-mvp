# Modelo de Estados Helpdesk

## Objetivo
Definir los estados minimos del sistema para implementar el flujo helpdesk sin ambiguedad entre backend, dashboard y fork de RustDesk.

## Entidades principales
- `Agent`
- `HelpdeskTicket`
- `RemoteSession`

## Estados de agente

### `offline`
El operador no esta conectado al sistema o no envia heartbeat valido.

### `available`
El operador esta activo y puede recibir una nueva asignacion.

### `opening`
El operador ya fue seleccionado para una incidencia y esta dentro de la ventana previa de 10 segundos.

### `busy`
El operador ya esta atendiendo una sesion activa.

### `away`
El operador esta conectado pero se ha marcado manualmente como no disponible.

## Transiciones de agente
- `offline -> available`: login/activacion correcta.
- `available -> away`: operador se pausa manualmente.
- `away -> available`: operador vuelve a activarse.
- `available -> opening`: backend asigna ticket y dispara countdown.
- `opening -> busy`: la sesion se abre correctamente.
- `opening -> available`: countdown cancelado, timeout de apertura o fallo de conexion.
- `busy -> available`: sesion terminada y operador sigue activo.
- `busy -> away`: sesion terminada y operador decide quedar pausado.
- `any -> offline`: perdida de heartbeat, cierre de app o logout.

## Estados de ticket

### `new`
Solicitud recien creada, aun no evaluada por el asignador.

### `queued`
Solicitud pendiente de agente disponible.

### `assigned`
Ticket asociado a un agente concreto, pendiente de iniciar countdown.

### `opening`
Countdown de 10 segundos en curso antes de abrir la sesion remota.

### `in_progress`
Sesion remota abierta y ticket siendo atendido.

### `resolved`
Incidencia finalizada correctamente.

### `cancelled`
Solicitud cancelada por usuario, operador o sistema.

### `failed`
No se pudo completar la apertura o la asignacion y no hubo recuperacion automatica.

## Transiciones de ticket
- `new -> queued`: ticket normalizado y en espera de agente.
- `queued -> assigned`: backend encuentra agente `available`.
- `assigned -> opening`: se envia aviso de 10 segundos al agente.
- `opening -> in_progress`: sesion abierta correctamente.
- `opening -> queued`: agente ya no disponible, countdown cancelado o error recuperable.
- `in_progress -> resolved`: soporte terminado con exito.
- `new|queued|assigned|opening -> cancelled`: cancelacion anticipada.
- `opening|in_progress -> failed`: error no recuperable.

## Estados de sesion remota

### `pending`
La sesion aun no se ha abierto, pero existe intencion de apertura.

### `connecting`
Se esta intentando abrir la conexion remota.

### `active`
Conexion remota establecida.

### `ended`
Conexion cerrada de forma normal.

### `failed`
No se pudo abrir o mantener la conexion.

## Relacion entre estados
- Un `Agent` en `busy` implica al menos un `HelpdeskTicket` en `in_progress`.
- Un `HelpdeskTicket` en `opening` implica un `Agent` en `opening`.
- Un `HelpdeskTicket` en `in_progress` implica una `RemoteSession` en `active`.
- Un `Agent` no puede tener mas de un ticket en `opening` o `in_progress` simultaneamente.

## Eventos de dominio minimos
- `help_request_created`
- `agent_became_available`
- `agent_became_away`
- `agent_went_offline`
- `ticket_assigned`
- `opening_countdown_started`
- `opening_countdown_cancelled`
- `remote_session_started`
- `remote_session_failed`
- `remote_session_ended`
- `ticket_resolved`
- `ticket_cancelled`

## Auditoria minima recomendada
Prioridad baja, pero contemplada desde el modelo:
- alta de ticket
- asignacion a agente
- inicio del countdown
- cancelacion del countdown
- inicio de sesion
- fin de sesion
- cancelacion o fallo

## Invariantes
- Un agente no puede estar `available` y `busy` a la vez.
- Un ticket no puede estar asignado a mas de un agente.
- Un agente `offline` no puede conservar tickets en `opening`.
- Si un agente pierde heartbeat en `opening`, el ticket vuelve a `queued`.
- Si un agente pierde heartbeat en `busy`, la sesion se marca para recuperacion o cierre controlado.

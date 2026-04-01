# Addendum: flujo helpdesk de 3 actores y ticket enriquecido

Fecha: 2026-03-31

## Objetivo

Registrar la ampliacion funcional pedida despues de la demo:

- pasar de un flujo operativo de 2 actores implementados hoy a un modelo funcional de 3 actores;
- permitir que el cliente final cree tickets desde su propio equipo;
- definir el uso de un boton residente en segundo plano en Windows;
- enriquecer el ticket con informacion operativa adicional.

Este documento no reemplaza los requisitos originales. Los complementa.

## Cambio de modelo funcional

## Estado actual implementado

Hoy el sistema opera principalmente con 2 actores activos:

1. `Supervisor`
2. `Agente helpdesk`

El cliente final existe en el flujo como maquina objetivo del ticket, pero no como actor operativo con interfaz propia dentro del sistema.

## Nuevo modelo solicitado

Se solicita pasar a un flujo funcional de 3 actores:

1. `Supervisor`
2. `Agente helpdesk`
3. `Cliente final / equipo atendido`

## Definicion de actores

### 1. Supervisor

Responsabilidades:

- supervisar la operacion desde dashboard;
- ver tickets, agentes y estados;
- reasignar o intervenir cuando haga falta;
- auditar el ciclo de atencion.

### 2. Agente helpdesk

Responsabilidades:

- marcarse como disponible o ausente;
- recibir asignaciones;
- abrir la conexion remota;
- atender y cerrar tickets.

### 3. Cliente final / equipo atendido

Responsabilidades:

- tener instalado el cliente corporativo basado en RustDesk;
- poder solicitar ayuda desde su propio equipo;
- enviar junto con la solicitud el identificador estable de la maquina;
- ver que el helpdesk corporativo esta activo y visible.

## Requisito clave: el cliente final tambien debe tener RustDesk instalado

Si el ticket nace desde el equipo atendido y debe terminar en una conexion remota real, entonces el cliente final debe tener instalado el cliente corporativo de RustDesk.

Motivo:

- el ticket debe viajar con el `RustDesk ID` real del equipo;
- ese identificador sera el destino de la futura conexion del agente;
- sin ese cliente instalado, no existe una forma estable de asociar el ticket con la maquina remota que debe atenderse.

## Nuevo flujo funcional objetivo

1. El cliente final tiene instalado el cliente corporativo de RustDesk.
2. En su equipo existe un acceso visible en segundo plano, tipo icono de bandeja de sistema de Windows.
3. El cliente pulsa ese boton y abre un formulario de solicitud.
4. El sistema toma automaticamente su `RustDesk ID`.
5. El cliente completa los campos del ticket.
6. El ticket se crea en el backend central.
7. El supervisor puede verlo en el dashboard.
8. Si hay agentes disponibles, el sistema lo puede despachar automaticamente o dejarlo en cola segun la politica definida.
9. El agente recibe la asignacion y abre la sesion remota contra ese `RustDesk ID`.
10. Al finalizar, el ticket queda auditado y cerrado con trazabilidad.

## Requisito de UX: boton en segundo plano de Windows

Se solicita un punto de entrada visible para el cliente final que no dependa de abrir toda la app manualmente.

### Comportamiento esperado

- la app cliente corre en segundo plano en Windows;
- aparece un icono de bandeja de sistema;
- desde ese icono se puede abrir una accion tipo `Solicitar ayuda` o `Crear ticket`;
- al pulsarlo, aparece un formulario simple y corporativo.

### Requisitos del boton

- visible y legitimo, no oculto ni ambiguo;
- nombre corporativo claro;
- accesible desde tray/menu contextual;
- debe funcionar aunque la ventana principal no este abierta.

## Campos nuevos requeridos para el ticket

El ticket ya no debe limitarse al identificador de maquina y un resumen minimo. Debe enriquecerse con informacion operativa util.

### Campos propuestos

#### Obligatorios

- `client_rustdesk_id`
- `title`
- `description`
- `difficulty`
- `estimated_time`

#### Recomendados

- `requested_by`
- `client_display_name`
- `device_id`
- `source = client_tray`
- `created_at`

## Definicion funcional de campos

### `client_rustdesk_id`

El identificador RustDesk del equipo atendido.

Uso:

- clave tecnica para abrir la conexion remota;
- correlacion entre ticket y maquina.

### `title`

Titulo breve del problema.

Ejemplos:

- `No abre el ERP`
- `Impresora sin conexion`
- `PC muy lenta`

### `description`

Descripcion libre del problema.

Uso:

- dar contexto al agente;
- mejorar clasificacion y auditoria.

### `difficulty`

Dificultad estimada de la incidencia.

Opciones sugeridas:

- `low`
- `medium`
- `high`
- `critical`

Tambien se podria usar una escala `1-5`, pero la opcion por etiquetas es mas comprensible para operacion inicial.

### `estimated_time`

Tiempo aproximado estimado para resolver o revisar la tarea.

Formato recomendado:

- minutos enteros, por ejemplo `15`, `30`, `60`, `120`

Motivo:

- ayuda a priorizar;
- ayuda a repartir carga;
- sirve para futuras metricas y SLA.

## Impacto tecnico esperado

## Backend

Habra que extender el contrato de creacion de tickets para aceptar al menos:

- `title`
- `description`
- `difficulty`
- `estimated_time`
- `source`

Y conservar:

- `client_id` o `client_rustdesk_id`
- `client_display_name`
- `requested_by`
- `device_id`

## Dashboard

El dashboard debera mostrar esos campos en:

- cola de tickets;
- detalle del ticket;
- filtros y priorizacion futura.

## Fork cliente final

Habra que crear el modo cliente final de helpdesk, con:

- branding visible;
- icono de bandeja;
- formulario de ticket;
- captura automatica del `RustDesk ID`;
- feedback de estado del ticket.

## Fork agente

No cambia el actor, pero si cambia el origen del ticket:

- ahora el agente recibira tickets creados por clientes finales;
- el payload de asignacion debe seguir llevando el `client_rustdesk_id`;
- el agente debera ver tambien `title`, `description`, `difficulty` y `estimated_time`.

## Requisitos de producto

Para que el flujo sea coherente con lo pedido, el sistema debe presentarse claramente como herramienta corporativa de helpdesk.

El cliente final debe percibir:

- que el software esta instalado con consentimiento corporativo;
- que puede pedir ayuda desde su equipo;
- que la solicitud sera atendida por soporte;
- que existe trazabilidad operativa.

## Fases recomendadas

### Fase 1. Modelo y API

- extender modelo de ticket;
- extender API de creacion;
- guardar y exponer nuevos campos.

### Fase 2. Dashboard

- mostrar nuevos campos;
- permitir ordenar y filtrar por dificultad y tiempo estimado.

### Fase 3. Cliente final Windows

- icono de bandeja;
- boton `Solicitar ayuda`;
- formulario de ticket;
- captura automatica de `RustDesk ID`.

### Fase 4. Integracion operacional

- conectar el ticket creado por cliente con la asignacion automatica;
- entregar al agente el `RustDesk ID` correcto;
- correlacionar ticket, agente y sesion remota.

## Decision recomendada

La peticion es coherente y encaja con la vision original del proyecto.

Pero implica reconocer explicitamente que el sistema ya no es solo:

- dashboard supervisor;
- consola de agente.

Pasa a ser un helpdesk de 3 actores:

- supervisor;
- agente;
- cliente final.

Ese cambio debe quedar reflejado en:

- requisitos;
- backlog;
- modelo de ticket;
- UX del cliente final;
- documentacion operativa.

## Conclusión

Lo pedido introduce un salto funcional importante y legitimo:

- el cliente final deja de ser solo una maquina destino;
- pasa a convertirse en actor funcional del sistema;
- el ticket deja de ser un registro minimo y pasa a ser una incidencia mas rica y priorizable;
- la app cliente necesita un punto de entrada visible desde Windows, preferiblemente en la bandeja de sistema.

La implementacion actual no cubre aun esta ampliacion completa, pero el backend y la base helpdesk existente ya permiten usarla como siguiente gran etapa del producto.

# Analisis de brecha del proyecto

Fecha: 2026-03-17

## 1. Objetivo del proyecto

Construir una solucion de soporte remoto basada en un fork corporativo de RustDesk, con una capa centralizada que permita:

- operar dos tipos de clientes: equipo final y agente de soporte
- registrar disponibilidad de agentes
- solicitar atencion remota desde el equipo final
- asignar automaticamente solicitudes a agentes disponibles
- iniciar y cerrar atenciones con trazabilidad operativa
- conservar capacidades de auditoria, presencia y monitoreo ya implementadas

## 2. Estado actual del software

El repositorio ya dispone de una base funcional para monitoreo operativo de sesiones remotas:

- backend en Rust
- persistencia SQLite
- ingestion de eventos de sesion
- webhook con outbox, reintentos y HMAC
- presencia por sesion
- dashboard web para supervision
- autenticacion local para dashboard
- exportacion CSV e historial de sesiones
- CLI para pruebas de eventos, presencia y grabacion

## 3. Alcance cubierto actualmente

Actualmente el proyecto cubre bien estas necesidades:

- auditar inicio y cierre de sesiones
- registrar actividad y presencia de participantes
- visualizar sesiones activas en dashboard
- consultar detalle y timeline de una sesion
- exponer integraciones externas por webhook
- soportar una base tecnica para el fork corporativo

## 4. Brecha funcional detectada

El proyecto todavia no implementa el flujo principal de helpdesk distribuido. Las brechas mas relevantes son las siguientes.

### 4.1 Solicitud de soporte

No existe un flujo formal para que un equipo final genere una solicitud de ayuda.

Falta:

- boton o accion explicita de "solicitar ayuda"
- identificacion del equipo que pide soporte
- estado de solicitud abierta, asignada, en curso, cerrada o cancelada
- prioridad, timestamps y contexto minimo de la incidencia

### 4.2 Gestion de agentes

No existe un modelo operativo de agentes de soporte.

Falta:

- registro de agentes
- estados de agente: disponible, ocupado, ausente, desconectado
- activacion y desactivacion manual de disponibilidad
- control para evitar que un agente reciba mas de una atencion simultanea

### 4.3 Asignacion automatica

No existe una central de despacho que conecte solicitudes con agentes.

Falta:

- cola de solicitudes pendientes
- algoritmo de asignacion
- reglas de prioridad
- historial de asignaciones y reasignaciones
- manejo de timeout cuando un agente no acepta o no responde

### 4.4 Flujo operativo de atencion

No existe una capa de orquestacion del ciclo de atencion.

Falta:

- aceptar o rechazar una atencion
- marcar inicio real de intervencion
- marcar finalizacion
- registrar motivo de cierre o abandono
- enlazar solicitud, agente y sesion remota resultante

### 4.5 Interfaz de cliente final

El fork actual no evidencia una experiencia centrada en helpdesk para el equipo atendido.

Falta:

- branding y mensaje explicito de herramienta corporativa de soporte
- pantalla o estado visible de "helpdesk activo"
- accion simple para pedir asistencia
- feedback de estado: pendiente, asignado, conectado, finalizado

### 4.6 Interfaz de agente

No existe una consola orientada al agente de soporte.

Falta:

- vista de disponibilidad
- cola o bandeja de solicitudes
- notificacion de nueva asignacion
- estado de trabajo actual
- control manual de disponibilidad

### 4.7 Modelo de datos de helpdesk

La base SQLite actual soporta eventos, presencia y sesiones web del dashboard, pero no entidades de helpdesk.

Falta crear al menos:

- `agents`
- `agent_status_history`
- `support_requests`
- `support_assignments`
- `support_sessions`
- `support_events`

### 4.8 API de helpdesk

No existe API dedicada al nuevo dominio funcional.

Faltan endpoints para:

- crear solicitud de soporte
- consultar estado de solicitud
- registrar disponibilidad del agente
- asignar o reasignar solicitud
- iniciar y cerrar atencion
- listar cola operativa y agentes activos

### 4.9 Dashboard operativo

El dashboard actual esta orientado a supervision y auditoria, no a coordinacion de helpdesk.

Falta:

- vista de cola en tiempo real
- vista de agentes y su estado
- metricas de SLA basicas
- metricas de carga por agente
- vista de asignaciones activas

### 4.10 Escalabilidad operativa

La base actual sirve como MVP tecnico, pero el caso de uso objetivo implica carga sostenida y mas concurrencia.

Riesgos actuales:

- SQLite puede quedarse corto para una operacion con muchas solicitudes concurrentes
- no hay colas distribuidas ni mecanismos de locking a nivel de despacho
- no hay politicas de reintento o recuperacion para asignaciones fallidas

## 5. Componentes reutilizables del MVP actual

La base implementada sigue siendo util y debe reaprovecharse:

- ingestion de eventos para auditoria de sesiones
- presencia para reflejar actividad en tiempo real
- dashboard existente como base de autenticacion y visualizacion
- SQLite como almacenamiento inicial de bajo costo
- webhook para integracion con sistemas externos
- fork corporativo como binario principal del cliente remoto

## 6. Backlog priorizado

### Fase 1. Nucleo funcional de helpdesk

- definir modelo de datos de helpdesk
- crear API de solicitudes y disponibilidad
- implementar estados de agente y solicitud
- enlazar solicitud con sesion remota
- crear vista minima de cola y agentes

### Fase 2. Asignacion automatica

- implementar motor de asignacion
- bloquear doble asignacion de agente
- registrar historial de asignacion
- agregar reglas simples de prioridad y fairness

### Fase 3. Integracion con el fork

- agregar UI de cliente final para pedir ayuda
- agregar UI de agente para marcar disponibilidad
- conectar el fork con la API central
- asegurar compatibilidad de binarios y parametros corporativos

### Fase 4. Observabilidad y operacion

- metricas de SLA
- auditoria de solicitudes
- panel de sesiones vinculadas a tickets
- alertas basicas de backlog y agentes desconectados

### Fase 5. Escalado tecnico

- revisar migracion de SQLite a Postgres
- separar worker de asignacion si aumenta la carga
- endurecer autenticacion y permisos por rol

## 7. Supuestos operativos pendientes de definir

Quedan decisiones funcionales por confirmar porque afectan el diseno:

- si la asignacion sera totalmente automatica o semiautomatica
- si el agente debe aceptar manualmente una solicitud
- si una solicitud puede pasar por varios agentes
- si el cliente final debe ver nombre del agente asignado
- si el sistema debe soportar prioridades, regiones o idiomas
- si habra horario de operacion y estados programados de agentes

## 8. Recomendacion inmediata

La siguiente etapa recomendable no es ampliar auditoria, sino introducir el dominio de helpdesk como capa formal del sistema:

1. definir entidades y estados
2. crear endpoints de solicitudes y agentes
3. habilitar una vista operativa simple en dashboard
4. integrar despues el fork con esos flujos

## 9. Conclusión

El proyecto tiene una base tecnica valida y reutilizable, pero todavia no cumple el flujo operacional completo de helpdesk remoto. La brecha principal no esta en monitoreo ni auditoria, sino en orquestacion: solicitud, disponibilidad, asignacion, atencion y cierre.

El avance actual reduce mucho el trabajo tecnico inicial, pero todavia falta implementar el nucleo funcional que convierte el fork de RustDesk en una solucion de helpdesk centralizada.

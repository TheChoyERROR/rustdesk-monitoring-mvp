# Post-demo: prioridades reales a abordar

Fecha: 2026-03-27

## Objetivo

Separar lo que realmente conviene atacar despues de la demo de lo que ya tiene base implementada. La idea es concentrar esfuerzo en robustez operativa, persistencia, observabilidad y escalabilidad real.

## Resumen ejecutivo

Despues de la demo, el proyecto no necesita "reinventar el helpdesk". Lo que necesita es endurecerlo para operacion real.

Las prioridades mas sensatas son:

1. persistencia real y despliegue confiable;
2. reparto de carga y politica de asignacion;
3. metricas operativas y SLA;
4. correlacion completa entre ticket, agente y sesion remota;
5. webhook productivo y runbook operativo.

## Prioridad alta

### ISSUE 13. Persistencia real en Render con disco montado

Es el issue mas importante de corto plazo.

Motivo:

- hoy SQLite en Render Free no garantiza persistencia;
- esto afecta historico de tickets, sesiones y continuidad operativa;
- sin persistencia real, la demo puede verse bien pero la operacion diaria queda fragil.

Trabajo recomendado:

- montar persistent disk en `/app/data` en un plan compatible;
- validar que `outbox.db` sobreviva a restart y redeploy;
- documentar backup y restore.

### ISSUE 2. Fairness / reparto de carga entre agentes

Es el siguiente hueco funcional fuerte.

Motivo:

- hoy la asignacion existe, pero la regla de reparto sigue siendo simple;
- en operacion real conviene poder explicar por que un ticket fue a un agente y no a otro.

Trabajo recomendado:

- definir estrategia clara: round-robin, least-loaded o ponderada;
- exponer carga por agente;
- auditar la razon de asignacion.

### ISSUE 11. Metricas operativas y SLA basicos

Hace falta para pasar de MVP funcional a operacion medible.

Trabajo recomendado:

- tiempo hasta asignacion;
- tiempo hasta inicio de atencion;
- duracion de atencion;
- tickets resueltos por agente;
- tickets fallidos, vencidos o abandonados.

### ISSUE 15. Webhook productivo endurecido

El emisor ya esta bien, pero el receptor operativo aun debe cerrarse mejor.

Trabajo recomendado:

- usar receptor estable;
- mantener HMAC;
- monitorear entregas/fallos;
- revisar alertas y troubleshooting.

## Prioridad media

### ISSUE 6. Auditoria completa del ciclo de atencion

La auditoria ya existe, pero falta cerrar la correlacion completa:

- ticket;
- agente;
- sesion remota real;
- causa de cierre.

Trabajo recomendado:

- enriquecer timeline por ticket;
- exponer mejor la correlacion con la sesion real;
- facilitar exportacion y consulta.

### ISSUE 9. Cola operativa en tiempo real

La cola ya existe, pero hoy usa refresco periodico y no una actualizacion en vivo mas refinada.

Trabajo recomendado:

- pasar a SSE o push donde aporte valor;
- mejorar orden, filtros y escalado;
- hacerla mas rapida para coordinacion diaria.

### ISSUE 14. Runbook operativo de incidentes

Muy recomendable para evitar dependencia de contexto oral.

Trabajo recomendado:

- que revisar si no entran eventos;
- que revisar si no aparecen agentes;
- que revisar si falla webhook;
- que hacer ante reinicios, redeploy o corrupcion del archivo SQLite.

### ISSUE 18. Estrategia de mantenimiento del fork contra upstream

Es importante, pero no bloquea la operacion inmediata.

Trabajo recomendado:

- politica de merge o rebase;
- inventario de cambios propios;
- zonas sensibles del fork;
- checklist antes de actualizar upstream.

### ISSUE 19. Roles minimos adicionales

Tiene sentido cuando el dashboard deje de ser de un solo supervisor.

Trabajo recomendado:

- diferenciar supervisor de operador web;
- limitar acciones sensibles;
- proteger reasignaciones y vistas de auditoria.

## Prioridad baja

### ISSUE 12. Refinamiento del timeline visual

Es valido y conviene mejorarlo, pero no es el mayor riesgo del proyecto.

Trabajo recomendado:

- diferenciar eventos puntuales de intervalos;
- no expandir de mas tramos incompletos;
- evitar lecturas engañosas.

## Issues que no son post-demo greenfield

Estos temas si pueden seguir mejorandose, pero no deberian entrar como grandes bloques nuevos porque ya tienen base implementada:

- `ISSUE 1` motor de despacho desacoplado
- `ISSUE 5` maquina de estados formal
- `ISSUE 7` heartbeats y presencia operativa
- `ISSUE 10` vista operativa de agentes
- `ISSUE 16` pipeline reproducible del instalador
- `ISSUE 17` trazabilidad del binario
- `ISSUE 20` endurecimiento de sesion web

En estos casos, lo razonable despues de la demo es hablar de:

- refinamiento;
- endurecimiento;
- formalizacion;
- observabilidad;
- mejor escalado.

No de implementacion desde cero.

## Recomendacion final

Orden practico sugerido para el siguiente tramo:

1. persistencia real en Render;
2. fairness de asignacion;
3. metricas operativas y SLA;
4. auditoria ticket-agente-sesion;
5. webhook productivo;
6. runbook operativo;
7. refinamiento de UX y timeline.

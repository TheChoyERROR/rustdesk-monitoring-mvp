# Issues ya cubiertos o parcialmente cubiertos

Fecha: 2026-03-27

## Objetivo

Dejar claro que varios issues planteados despues de la demo no parten de cero. En estos puntos ya existe implementacion real en backend, dashboard, app desktop o pipeline de build. En algunos casos no estan "cerrados al 100%", pero ya hay una base funcional clara y comprobable.

## Resumen ejecutivo

Los issues `1`, `5`, `7`, `10`, `16`, `17` y `20` no deben tratarse como bloques totalmente pendientes. La lectura correcta es:

- ya existe implementacion base o avanzada;
- el trabajo restante es de endurecimiento, refinamiento o formalizacion;
- no corresponde presentarlos como si el proyecto no tuviera nada hecho en esas areas.

## ISSUE 1. Motor de despacho desacoplado

### Estado actual

Este punto ya esta parcialmente cubierto.

La asignacion principal vive en backend y no en el frontend:

- `create_helpdesk_ticket` crea tickets y puede disparar asignacion inmediata.
- `assign_helpdesk_ticket` permite despacho manual desde web.
- `reconcile_helpdesk_queue_tx` resuelve la asignacion automatica cuando hay agentes disponibles.
- `reconcile_helpdesk_runtime` recupera tickets en timeout o agentes caidos.

Referencias principales:

- `src/storage.rs`
- funciones: `create_helpdesk_ticket`, `assign_helpdesk_ticket`, `reconcile_helpdesk_queue_tx`, `reconcile_helpdesk_runtime`

### Lo que falta

- separar aun mas la logica como "dispatcher" formal si se quiere escalar;
- definir politica de fairness;
- mejorar trazabilidad explicita de por que se eligio un agente sobre otro.

### Conclusión

No es correcto decir que el despacho "falta". Lo correcto es decir que ya existe una capa central de asignacion en backend, pero todavia no esta madura para operacion amplia.

## ISSUE 5. Maquina de estados formal para tickets

### Estado actual

Este punto ya esta parcialmente cubierto y documentado.

Ya existe un modelo de estados de helpdesk en:

- `docs/helpdesk-state-model.md`

Y ademas el backend impone transiciones reales en:

- `start_helpdesk_ticket`
- `resolve_helpdesk_ticket`
- `requeue_helpdesk_ticket`
- `cancel_helpdesk_ticket`
- `reconcile_helpdesk_runtime`

Estados ya presentes en codigo y documentacion:

- `new`
- `queued`
- `assigned`
- `opening`
- `in_progress`
- `resolved`
- `cancelled`
- `failed`

### Lo que falta

- decidir si se quieren estados mas finos como `accepted`, `connecting`, `closed` o `expired`;
- formalizar si esos estados son de ticket o de sesion remota;
- mantener sincronizada la UX web con esa taxonomia final.

### Conclusión

No es un issue greenfield. La maquina de estados ya existe; lo pendiente es refinarla, no inventarla desde cero.

## ISSUE 7. Heartbeats y presencia operativa de agentes

### Estado actual

Este punto esta mayormente cubierto.

Ya existe:

- heartbeat periodico desde la app desktop;
- estados `available`, `busy`, `away`, `offline`;
- transicion automatica a `offline` cuando no hay heartbeat valido;
- resumen y vista web del estado actual de agentes.

Referencias principales:

- `src/storage.rs`
- `rustdesk-fork/flutter/lib/models/helpdesk_model.dart`
- `web-dashboard/src/pages/HelpdeskPage.tsx`

### Lo que falta

- historial mas formal de cambios de disponibilidad si se quiere mas auditoria;
- mas metricas por agente para operacion y SLA.

### Conclusión

No tiene sentido presentarlo como pendiente principal. La base de presencia operativa ya esta implementada.

## ISSUE 10. Vista operativa de agentes

### Estado actual

Este punto ya esta bastante implementado.

El dashboard ya muestra:

- listado de agentes;
- estado actual;
- ticket actual;
- ultimo heartbeat;
- momento de actualizacion;
- resumen agregado de agentes disponibles, ocupados, ausentes y offline.

Ademas existe detalle de ticket con acciones operativas:

- despachar;
- reencolar;
- cancelar.

Referencias principales:

- `web-dashboard/src/pages/HelpdeskPage.tsx`
- `web-dashboard/src/pages/HelpdeskTicketDetailPage.tsx`

### Lo que falta

- tiempo en estado actual;
- deteccion visual de agentes atascados mas refinada;
- mas acciones operativas desde una sola vista.

### Conclusión

No es correcto decir que "falta la vista de agentes". Lo correcto es decir que ya existe una vista operativa util, aunque aun no es la version final de produccion.

## ISSUE 16. Pipeline reproducible del instalador Windows

### Estado actual

Este punto esta parcialmente cubierto con bastante avance.

Ya existen:

- scripts de bootstrap de maquina nueva;
- instalacion asistida de dependencias;
- chequeo del entorno Windows;
- build del instalador;
- manifiesto del paquete;
- version, hash y artefactos de salida;
- documentacion del flujo de build.

Referencias principales:

- `scripts/bootstrap-windows-dev-machine.ps1`
- `scripts/check-rustdesk-windows-build-env.ps1`
- `scripts/build-rustdesk-windows-installer.ps1`
- `docs/windows-installer.md`
- `docs/windows-new-machine-setup.md`

### Lo que falta

- mas limpieza del estado del fork para garantizar reproducibilidad absoluta;
- checklist de release mas formal;
- idealmente CI o una maquina de build controlada.

### Conclusión

No esta resuelto al nivel industrial, pero tampoco es artesanal como al inicio. Ya hay un pipeline reproducible razonable.

## ISSUE 17. Trazabilidad formal del binario corporativo

### Estado actual

Este punto ya esta parcialmente cubierto.

El pipeline del instalador ya guarda:

- version del paquete;
- SHA256 del ejecutable fuente;
- version del ejecutable;
- rutas de artefactos;
- manifiesto por release.

Referencias principales:

- `scripts/build-rustdesk-windows-installer.ps1`
- `docs/windows-installer.md`

### Lo que falta

- ligar de forma mas estricta `release -> commit -> binario`;
- exponer esa informacion de forma mas visible para soporte;
- normalizar versionado semantico interno del fork.

### Conclusión

La trazabilidad no esta ausente; ya existe un nivel util de trazabilidad tecnica. Lo que falta es endurecerla y volverla mas formal.

## ISSUE 20. Endurecimiento de sesion web

### Estado actual

Este punto ya fue abordado recientemente.

Ya existe:

- cookie firmada;
- renovacion de sesion con actividad;
- keep-alive periodico en frontend;
- TTL configurable para produccion;
- logout e invalidacion de cookie.

Referencias principales:

- `src/auth.rs`
- `src/server.rs`
- `web-dashboard/src/auth.tsx`
- `render.yaml`

### Lo que falta

- auditoria mas completa de logins si se considera necesaria;
- endurecimiento adicional por roles o politicas mas finas de sesion.

### Conclusión

No es un issue prioritario de "corregir ya". La parte critica de estabilidad de sesion ya se trabajo.

## Cierre

Los siete issues anteriores deben tratarse como:

- cubiertos en base funcional;
- parcialmente resueltos;
- candidatos a refinamiento posterior.

No deben presentarse como ausencia de arquitectura o como carencias absolutas del producto actual, porque eso no refleja el estado real del codigo.

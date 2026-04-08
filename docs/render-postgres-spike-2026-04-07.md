# Render Postgres Spike (2026-04-07)

Este spike deja preparado un primer camino controlado hacia Render Postgres sin reemplazar aun el runtime principal basado en SQLite.

## Alcance

- soporte `postgres` habilitado en `sqlx`
- smoke test real contra Render Postgres
- bootstrap de schema `helpdesk` en Postgres
- import inicial de datos `helpdesk` desde SQLite local
- storage minimo de `helpdesk` sobre Postgres
- smoke CRUD real de `helpdesk` sobre Render Postgres
- integracion opcional al servidor mediante rutas protegidas
- runtime real de helpdesk sobre Postgres usando las rutas normales del agente y supervisor

## Archivos

- [Cargo.toml](/C:/Users/Choy/Desktop/rustdesk-monitoring-mvp/Cargo.toml)
- [src/postgres.rs](/C:/Users/Choy/Desktop/rustdesk-monitoring-mvp/src/postgres.rs)
- [src/postgres_helpdesk.rs](/C:/Users/Choy/Desktop/rustdesk-monitoring-mvp/src/postgres_helpdesk.rs)
- [src/bin/render-postgres-smoke.rs](/C:/Users/Choy/Desktop/rustdesk-monitoring-mvp/src/bin/render-postgres-smoke.rs)
- [src/bin/render-postgres-helpdesk-bootstrap.rs](/C:/Users/Choy/Desktop/rustdesk-monitoring-mvp/src/bin/render-postgres-helpdesk-bootstrap.rs)
- [src/bin/render-postgres-helpdesk-crud-smoke.rs](/C:/Users/Choy/Desktop/rustdesk-monitoring-mvp/src/bin/render-postgres-helpdesk-crud-smoke.rs)
- [src/bin/render-postgres-helpdesk-agent-runtime-smoke.rs](/C:/Users/Choy/Desktop/rustdesk-monitoring-mvp/src/bin/render-postgres-helpdesk-agent-runtime-smoke.rs)
- [src/server.rs](/C:/Users/Choy/Desktop/rustdesk-monitoring-mvp/src/server.rs)

## Validado

Se valido contra la base Free creada en Render:

- conexion OK
- `SELECT 1` OK
- creacion de tabla de prueba OK
- insercion de fila de prueba OK
- bootstrap del schema de helpdesk OK
- CRUD real de `helpdesk` OK
- handlers protegidos sobre Postgres OK
- runtime real de agente OK sobre Postgres

Salida observada:

```text
Render Postgres OK: select_one=1, smoke_rows=2
Render Postgres helpdesk bootstrap OK: authorized_agents=0, agents=0, tickets=0, assignments=0, heartbeats=0, audit_events=0
Render Postgres helpdesk CRUD OK: authorized_agents=1, tickets=1, fetched_ticket=<uuid>, audit_events=2
Render Postgres helpdesk runtime OK: authorized=true, opening_status=Opening, assignment_ticket=<uuid>, in_progress=InProgress, report_present=true, resolved=Resolved, released_agent=Available, fetched_agent=Available, fetched_ticket=Resolved, initial_presence=Available
```

## Activar rutas experimentales en el servidor

Configura una de estas variables:

```text
HELPDESK_POSTGRES_DATABASE_URL=<database-url>
```

o, como fallback:

```text
DATABASE_URL=<database-url>
```

Si quieres que las rutas reales de helpdesk usen Postgres, agrega tambien:

```text
HELPDESK_BACKEND=postgres
```

Con eso el servidor habilita rutas protegidas por dashboard auth:

- `GET/POST /api/v1/postgres/helpdesk/agent-authorizations`
- `DELETE /api/v1/postgres/helpdesk/agent-authorizations/:agent_id`
- `GET /api/v1/postgres/helpdesk/agents`
- `GET /api/v1/postgres/helpdesk/summary`
- `GET/POST /api/v1/postgres/helpdesk/tickets`
- `GET /api/v1/postgres/helpdesk/tickets/:ticket_id`
- `GET /api/v1/postgres/helpdesk/tickets/:ticket_id/audit`

Si Postgres no esta configurado, esas rutas responden `503`.

## Como probarlo desde la web

Una vez desplegado el backend con `HELPDESK_POSTGRES_DATABASE_URL` o `DATABASE_URL` apuntando a
Render Postgres:

1. entra al dashboard web
2. usa el toggle del header `Helpdesk: SQLite / Postgres`
3. cambia a `Postgres`
4. entra a `Helpdesk`

En este modo experimental ya deberias poder:

- ver resumen de helpdesk desde Postgres
- listar agentes autorizados desde Postgres
- autorizar y quitar agentes autorizados en Postgres
- crear tickets basicos en Postgres
- listar tickets y ver su detalle/auditoria

Si ademas configuras `HELPDESK_BACKEND=postgres`, las rutas normales de helpdesk (`/api/v1/helpdesk/...`) pasan a usar Postgres para:

- autorizacion del agente
- presencia del agente
- polling de asignacion
- inicio de asignacion
- tickets, auditoria y resumen
- asignacion manual
- actualizacion de campos operativos
- reportes de soporte
- resolve / requeue / cancel
- reconciliacion de timeouts y agentes stale

Todavia no se mueve a Postgres:

- `session_events`
- `session_presence`
- `outbox_events`
- webhook
- dashboard summary/timeline de monitoreo

## Como usar

### 1. Smoke test de conexion

```powershell
$env:DATABASE_URL="<external-database-url>"
cargo run --quiet --bin render-postgres-smoke
```

### 2. Bootstrap de helpdesk desde SQLite

```powershell
$env:DATABASE_URL="<external-database-url>"
cargo run --quiet --bin render-postgres-helpdesk-bootstrap -- --sqlite-path .\data\outbox.db
```

### 3. Smoke CRUD de helpdesk

```powershell
$env:DATABASE_URL="<external-database-url>"
cargo run --quiet --bin render-postgres-helpdesk-crud-smoke
```

### 4. Smoke del runtime real del agente

```powershell
$env:DATABASE_URL="<external-database-url>"
cargo run --quiet --bin render-postgres-helpdesk-agent-runtime-smoke
```

Notas:

- para pruebas locales usa la `External Database URL`
- cuando esto corra dentro de Render, convendra usar la `Internal Database URL`
- el bootstrap actual solo cubre tablas de `helpdesk`
- para el cutover del helpdesk en Render usa:
  - `HELPDESK_POSTGRES_DATABASE_URL=<internal-database-url>`
  - `HELPDESK_BACKEND=postgres`
- este corte no requiere recompilar el `.exe`; la app sigue usando las mismas rutas HTTP

## Que NO hace aun

- no cambia el backend principal a Postgres
- no migra `session_events`, `session_presence` ni `outbox_events`
- no reemplaza `SqlitePool` en `monitoring-server`
- no migra aun el monitoreo/webhook a Postgres

## Siguiente fase recomendada

1. probar `HELPDESK_BACKEND=postgres` en Render con agentes reales
2. decidir si el dashboard debe eliminar el toggle experimental una vez validado el corte
3. planear por separado la migracion de monitoreo/webhook o mantenerlos en SQLite

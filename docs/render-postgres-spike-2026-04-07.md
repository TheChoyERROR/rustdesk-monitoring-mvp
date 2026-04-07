# Render Postgres Spike (2026-04-07)

Este spike deja preparado un primer camino controlado hacia Render Postgres sin reemplazar aun el runtime principal basado en SQLite.

## Alcance

- soporte `postgres` habilitado en `sqlx`
- smoke test real contra Render Postgres
- bootstrap de schema `helpdesk` en Postgres
- import inicial de datos `helpdesk` desde SQLite local

## Archivos

- [Cargo.toml](/C:/Users/Choy/Desktop/rustdesk-monitoring-mvp/Cargo.toml)
- [src/postgres.rs](/C:/Users/Choy/Desktop/rustdesk-monitoring-mvp/src/postgres.rs)
- [src/bin/render-postgres-smoke.rs](/C:/Users/Choy/Desktop/rustdesk-monitoring-mvp/src/bin/render-postgres-smoke.rs)
- [src/bin/render-postgres-helpdesk-bootstrap.rs](/C:/Users/Choy/Desktop/rustdesk-monitoring-mvp/src/bin/render-postgres-helpdesk-bootstrap.rs)

## Validado

Se valido contra la base Free creada en Render:

- conexion OK
- `SELECT 1` OK
- creacion de tabla de prueba OK
- insercion de fila de prueba OK
- bootstrap del schema de helpdesk OK

Salida observada:

```text
Render Postgres OK: select_one=1, smoke_rows=2
Render Postgres helpdesk bootstrap OK: authorized_agents=0, agents=0, tickets=0, assignments=0, heartbeats=0, audit_events=0
```

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

Notas:

- para pruebas locales usa la `External Database URL`
- cuando esto corra dentro de Render, convendra usar la `Internal Database URL`
- el bootstrap actual solo cubre tablas de `helpdesk`

## Que NO hace aun

- no cambia el backend principal a Postgres
- no migra `session_events`, `session_presence` ni `outbox_events`
- no reemplaza `SqlitePool` en `monitoring-server`

## Siguiente fase recomendada

1. crear storage Postgres solo para `helpdesk`
2. leer/escribir tickets/agentes desde Postgres en un modo aislado
3. dejar sesiones y webhook aun en SQLite hasta validar bien el corte

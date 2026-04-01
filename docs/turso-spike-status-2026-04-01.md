# Estado del spike Turso

Fecha: 2026-04-01

## Objetivo

Validar si la base actual del proyecto puede empezar a convivir con Turso sin tocar todavia la logica completa de storage del backend.

## Resultado

El spike ya evoluciono a un puente inicial usable para helpdesk.

Si podemos:

- conectar a Turso con `libsql`;
- aplicar el schema base del proyecto;
- listar tablas remotas ya creadas;
- sembrar supervisor/agente autorizado;
- crear y leer tickets reales de helpdesk;
- restaurar el estado de helpdesk desde Turso al arrancar el backend;
- sincronizar periodicamente el estado local de helpdesk hacia Turso;
- repetir la prueba desde WSL con un comando estable.

Todavia no podemos:

- usar Turso como storage primario para `session_events`, `outbox_events` y presencia de sesiones;
- decir que toda la base del proyecto ya migro a Turso;
- considerar la migracion cerrada.

## Lo que ya se probo

### Infraestructura

- base `rustdesk-monitoring-mvp` creada en Turso;
- URL `libsql://...` valida;
- autenticacion CLI valida;
- shell SQL responde `SELECT 1;`.

### Codigo

Se agrego una integracion inicial en Rust para:

- conectar a Turso remoto;
- aplicar el schema del proyecto;
- listar tablas detectadas;
- correr un smoke real de helpdesk;
- restaurar/sincronizar snapshots de helpdesk entre SQLite y Turso.

Archivos principales:

- `src/schema.rs`
- `src/turso.rs`
- `src/bin/turso-bootstrap.rs`
- `src/bin/turso-helpdesk-smoke.rs`
- `scripts/run-turso-bootstrap.sh`
- `scripts/run-turso-helpdesk-smoke.sh`

### Validacion real

La prueba completa se ejecuto con exito en WSL/Linux usando:

```bash
source ~/.cargo/env
cd /mnt/c/Users/Choy/Desktop/rustdesk-monitoring-mvp
export TURSO_DATABASE_URL="libsql://..."
export TURSO_AUTH_TOKEN="<token>"
./scripts/run-turso-bootstrap.sh
```

Y la validacion de dominio ya corre con:

```bash
./scripts/run-turso-helpdesk-smoke.sh --agent-id ... --client-id ... --title ...
```

Nota tecnica:

- el proyecto compila en Windows con `cargo check`;
- la ejecucion estable del bootstrap remoto quedo validada en WSL/Linux;
- para seguir con Turso, conviene usar WSL como entorno operativo de esta migracion.

## Tablas detectadas en Turso

El bootstrap remoto dejo visibles estas tablas:

- `dashboard_sessions`
- `dashboard_users`
- `helpdesk_agent_heartbeats`
- `helpdesk_agents`
- `helpdesk_audit_events`
- `helpdesk_authorized_agents`
- `helpdesk_ticket_assignments`
- `helpdesk_tickets`
- `outbox_events`
- `session_events`
- `session_presence`
- `sqlite_sequence`

## Operaciones de dominio ya validadas

Sobre la base remota de Turso ya quedo comprobado que podemos:

- insertar o actualizar el supervisor del dashboard;
- autorizar un agente de helpdesk;
- crear un ticket real en `helpdesk_tickets`;
- escribir su evento inicial en `helpdesk_audit_events`;
- leer tickets y resumen operativo.

## Estado actual del backend

El backend ya puede arrancar con un puente de persistencia de helpdesk si detecta:

- `TURSO_DATABASE_URL`
- `TURSO_AUTH_TOKEN`

Con esas variables:

- inicializa schema remoto en Turso;
- si Turso ya tiene datos, restaura helpdesk hacia la SQLite local;
- si Turso esta vacio pero SQLite local tiene datos, siembra Turso;
- mantiene un worker de sincronizacion periodica de helpdesk hacia Turso.

## Limite actual

El backend productivo sigue acoplado a `sqlx + SqlitePool + ruta de archivo local`.

Eso significa que:

- el servidor actual sigue esperando una base SQLite local;
- Turso no entra todavia como reemplazo directo;
- falta una segunda fase de adaptacion de storage.

## Decision tecnica correcta desde aqui

Ya no estamos en "solo spike de conexion". El siguiente paso correcto es probar este puente en Render con:

- `TURSO_DATABASE_URL`
- `TURSO_AUTH_TOKEN`

Y validar que:

- tickets creados sobreviven a redeploy;
- agentes autorizados sobreviven a redeploy;
- historial de helpdesk basico vuelve a levantar al reiniciar.

Despues de eso, recien conviene evaluar si seguimos con una migracion completa del runtime.

1. conservar SQLite actual como referencia;
2. dejar Turso bootstrapable y verificable;
3. definir la adaptacion del storage;
4. recien despues probar que el runtime real use Turso.

## Siguiente fase recomendada

### Fase 2A. Migracion prudente

- abstraer apertura de base / storage backend;
- decidir si el backend pasa a `libsql` remoto o a otra estrategia;
- validar operaciones criticas:
  - outbox
  - tickets
  - agentes
  - sesiones
  - auditoria

### Fase 2B. Migracion de datos

- importar una SQLite de prueba;
- comprobar consistencia;
- validar rollback.

## Conclusion

Turso ya quedo listo del lado de infraestructura y ya existe una prueba real de conexion + schema contra la base remota.

Lo que falta ya no es crear la base, sino adaptar el backend para que deje de depender exclusivamente de SQLite por archivo local.

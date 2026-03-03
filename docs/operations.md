# Operacion del Sistema (Backend + Dashboard + RustDesk Corporativo)

## Objetivo operativo
Este sistema permite que supervisores auditen sesiones de soporte remoto en tiempo real e historico.

Flujo base:
1. El fork corporativo de RustDesk envia eventos a `POST /api/v1/session-events`.
2. El backend guarda eventos en SQLite, actualiza presencia y procesa webhook/outbox.
3. El dashboard web consume APIs protegidas y SSE para supervision en vivo.

## Roles
- Supervisor (jefe): usa dashboard web con usuario/clave local.
- Trabajador: usa RustDesk corporativo preconfigurado. No requiere portal web en MVP.

## Variables de entorno del backend
- `DASHBOARD_SUPERVISOR_USERNAME`: usuario supervisor inicial. Default: `supervisor`.
- `DASHBOARD_SUPERVISOR_PASSWORD`: clave supervisor inicial. Default inseguro: `ChangeMeNow123!`.
- `DASHBOARD_SESSION_TTL_MINUTES`: expiracion de sesion web. Default: `480`.
- `DASHBOARD_COOKIE_SECURE`: `true/false`, habilitar cookie segura en HTTPS. Default: `false`.

## Configuracion de presencia (server-config.toml)
- `presence.stale_after_seconds`: tiempo maximo sin actividad para considerar una presencia como activa.
- `presence.cleanup_interval_seconds`: frecuencia del worker que expira presencias estancadas.
- Defaults recomendados para MVP: `stale_after_seconds = 120`, `cleanup_interval_seconds = 30`.

## Endpoints dashboard (protegidos)
- `GET /api/v1/auth/me`
- `GET /api/v1/dashboard/summary`
- `GET /api/v1/events`
- `GET /api/v1/sessions/:session_id/timeline`
- `GET /api/v1/reports/sessions.csv`

Endpoints auth abiertos:
- `POST /api/v1/auth/login`
- `POST /api/v1/auth/logout`

## Arranque en entorno interno
1. Levantar backend:
- Linux: `bash scripts/run-monitoring-server.sh`
- Windows: `powershell -ExecutionPolicy Bypass -File .\scripts\run-monitoring-server.ps1`
- Los scripts recompilan backend release en cada arranque para evitar binarios viejos.
- Si necesitas saltar compilacion:
  - Linux: `SKIP_BUILD=1 bash scripts/run-monitoring-server.sh`
  - Windows: `powershell -ExecutionPolicy Bypass -File .\scripts\run-monitoring-server.ps1 -SkipBuild`

2. Levantar dashboard (modo desarrollo):
- Linux: `bash scripts/run-dashboard-dev.sh`
- Windows: `powershell -ExecutionPolicy Bypass -File .\scripts\run-dashboard-dev.ps1`

3. Acceder al panel:
- URL: `http://127.0.0.1:5173`
- Login con supervisor configurado.

## Despliegue recomendado
1. Publicar backend y dashboard detras de reverse proxy HTTPS (Nginx/Caddy/IIS).
2. Mantener un solo dominio para evitar CORS y simplificar cookies.
3. Forzar `DASHBOARD_COOKIE_SECURE=true` en produccion.
4. Exponer solo puertos necesarios en red interna.

## Politica de monitoreo para trabajadores
Requerir aviso formal de monitoreo por TI/RRHH antes del despliegue general.

Texto base sugerido:
"Este equipo usa una version corporativa de RustDesk con registro de eventos operativos para auditoria de soporte y seguridad."

## Backups y retencion
- Base SQLite: `data/outbox.db`.
- Backup diario minimo recomendado:
  - Copia rotativa por fecha (`outbox-YYYYMMDD.db`).
  - Retencion minima: 7 dias para eventos fallidos auditables.
- Verificacion semanal de restauracion.

## Checklist de salud
1. `GET /health` responde `200`.
2. `GET /metrics` incrementa `events_received_total` al generar eventos.
3. Dashboard muestra eventos en menos de 3 segundos tras `session_started`.
4. CSV export devuelve filas para el rango filtrado.

## Incidentes comunes
1. Login 401 repetido:
- Verificar `DASHBOARD_SUPERVISOR_PASSWORD` y reiniciar backend.

2. Dashboard sin datos:
- Revisar que workers usen RustDesk corporativo con URL de monitoreo correcta.
- Probar ingest manual con `rustdesk-cli`.

3. Webhook pendiente/fallido alto:
- Revisar endpoint externo, timeout y conectividad.
- Verificar `webhook.hmac.secret` y validacion en receptor.

4. Sesiones "fantasma" activas:
- Revisar valores de `presence.stale_after_seconds` y `presence.cleanup_interval_seconds`.
- Confirmar en logs de arranque `presence cleanup configuration` y el binario actualizado.
- Validar que lleguen eventos de actividad (`participant_activity`) desde cliente fork.

## Plan de escalado futuro
1. Migrar de SQLite a Postgres para mayor concurrencia.
2. Integrar SSO y roles adicionales.
3. Agregar politicas de retencion por tipo de evento.

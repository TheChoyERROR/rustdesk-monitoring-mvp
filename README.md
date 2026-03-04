# RustDesk Monitoring MVP (Backend + Dashboard)

MVP de monitoreo y auditoria para un fork corporativo de RustDesk.

Incluye:
- Ingestion de eventos de sesion (`SessionEventV1`) con idempotencia por `event_id`.
- Outbox SQLite para webhook con HMAC, retries exponenciales y circuit breaker.
- Presencia colaborativa por sesion + SSE en tiempo real.
- Dashboard web (`React + Vite + TypeScript`) con login local para supervisor.
- Vista de detalle con avatares y timeline de actividad por usuario.
- Reporte CSV y timeline historico de sesiones.

## Estructura del repo
- `src/`: backend Rust (`monitoring-server`) + CLI (`rustdesk-cli`).
- `web-dashboard/`: frontend supervisor.
- `scripts/`: scripts de arranque Linux/Windows.
- `docs/technical-spec.md`: especificacion tecnica.
- `docs/operations.md`: guia de operacion y despliegue.
- `docs/run-binaries.md`: guia para ejecutar binarios y validar runtime.
- `docs/windows-installer.md`: guia para generar instalador Windows corporativo.
- `docs/windows-exe-playbook.md`: requisitos y flujo completo para instalar o crear `.exe` en Windows.
- `docs/railway-deploy.md`: deploy en Railway (API + dashboard en un servicio).
- `docs/render-free-deploy.md`: deploy rapido en Render Free para demo.
- `docs/professional-report-2026-03-04.txt`: reporte profesional consolidado del avance.

## Arquitectura funcional
1. RustDesk corporativo envia eventos a `POST /api/v1/session-events`.
2. Backend guarda eventos en `session_events`, actualiza presencia y cola outbox.
3. Worker entrega webhook con firma HMAC y retries.
4. Dashboard consulta API protegida por cookie HTTP-only.
5. Vista detalle usa SSE (`/presence/stream`) para cambios en tiempo real.

## Requisitos

Backend:
- Rust (cargo/rustc).
- Toolchain C/C++ para dependencias nativas.

Dashboard:
- Node.js 20+.
- npm 10+.

## Variables de entorno (dashboard auth)
- `DASHBOARD_SUPERVISOR_USERNAME` (default: `supervisor`)
- `DASHBOARD_SUPERVISOR_PASSWORD` (default inseguro: `ChangeMeNow123!`)
- `DASHBOARD_SESSION_TTL_MINUTES` (default: `480`)
- `DASHBOARD_COOKIE_SECURE` (default: `false`; en produccion usar `true` con HTTPS)

## Configuracion del server
Usa `server-config.example.toml` como base:
- `webhook.enabled`
- `webhook.url`
- `webhook.method`
- `webhook.timeout_ms`
- `webhook.retry.max_attempts`
- `webhook.retry.backoff_ms`
- `webhook.hmac.enabled`
- `webhook.hmac.secret`
- `presence.stale_after_seconds` (TTL de presencia activa antes de expirar)
- `presence.cleanup_interval_seconds` (frecuencia de limpieza de presencia)

## Levantar en Linux

1. Levantar backend:
```bash
bash scripts/run-monitoring-server.sh
```

Nota: el script recompila `monitoring-server` en modo release en cada arranque para evitar binarios desactualizados.  
Si quieres saltar compilacion: `SKIP_BUILD=1 bash scripts/run-monitoring-server.sh`

2. Levantar dashboard (dev):
```bash
bash scripts/run-dashboard-dev.sh
```

3. Abrir:
- Dashboard: `http://127.0.0.1:5173`
- Health: `http://127.0.0.1:8080/health`
- Metrics: `http://127.0.0.1:8080/metrics`

## Levantar en Windows (PowerShell)

1. Levantar backend:
```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\run-monitoring-server.ps1
```

Nota: el script recompila `monitoring-server.exe` en release en cada arranque para evitar binarios desactualizados.  
Si quieres saltar compilacion: `powershell -ExecutionPolicy Bypass -File .\scripts\run-monitoring-server.ps1 -SkipBuild`

2. Levantar dashboard (dev):
```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\run-dashboard-dev.ps1
```

3. Abrir:
- Dashboard: `http://127.0.0.1:5173`
- Health: `http://127.0.0.1:8080/health`
- Metrics: `http://127.0.0.1:8080/metrics`

## Endpoints

Publicos:
- `POST /api/v1/session-events`
- `GET /api/v1/sessions/presence`
- `GET /api/v1/sessions/:session_id/presence`
- `GET /api/v1/sessions/:session_id/presence/stream`
- `GET /health`
- `GET /metrics`

Auth:
- `POST /api/v1/auth/login`
- `POST /api/v1/auth/logout`
- `GET /api/v1/auth/me`

Dashboard protegido:
- `GET /api/v1/dashboard/summary?from=&to=`
- `GET /api/v1/events?session_id=&user_id=&event_type=&from=&to=&page=&page_size=`
- `GET /api/v1/sessions/:session_id/timeline?page=&page_size=`
- `GET /api/v1/reports/sessions.csv?from=&to=&user_id=`

## Uso del jefe (flujo real)
1. Abrir dashboard y autenticar.
2. Ver resumen diario (eventos, sesiones, estado webhook).
3. Filtrar eventos por trabajador, sesion, fecha y tipo.
4. Abrir detalle de sesion para timeline + presencia en vivo.
5. Exportar CSV para evidencia auditable.

## Uso de trabajadores (flujo real)
1. Instalar RustDesk corporativo (fork, no cliente oficial).
2. Ejecutar soporte remoto normal.
3. El cliente envia eventos automaticamente al backend.
4. TI debe comunicar aviso obligatorio de politica de monitoreo.

## Prueba rapida con CLI

Iniciar sesion:
```bash
cargo run --bin rustdesk-cli -- \
  --server-url http://127.0.0.1:8080 \
  --user-id supervisor \
  session start --session-id worker-001
```

Actividad/presencia:
```bash
cargo run --bin rustdesk-cli -- \
  --server-url http://127.0.0.1:8080 \
  presence join --session-id worker-001 --participant-id empleado1 --display-name "Empleado 1"
```

Actividad/presencia con avatar (visible en detalle de sesion):
```bash
cargo run --bin rustdesk-cli -- \
  --server-url http://127.0.0.1:8080 \
  presence join --session-id worker-001 --participant-id empleado1 \
  --display-name "Empleado 1" \
  --avatar-url "https://i.pravatar.cc/96?u=empleado1"
```

Cerrar sesion:
```bash
cargo run --bin rustdesk-cli -- \
  --server-url http://127.0.0.1:8080 \
  session end --session-id worker-001
```

## Integracion con fork RustDesk
Para preparar un fork real en Ubuntu:
```bash
bash scripts/install-rustdesk-ubuntu-deps.sh
bash scripts/check-rustdesk-fork.sh
```

Referencia:
- `docs/rustdesk-fork-setup-ubuntu.md`
- `docs/windows-installer.md`

## Notas de seguridad MVP
- Cambiar password supervisor antes de demo.
- En produccion: HTTPS + `DASHBOARD_COOKIE_SECURE=true`.
- Mantener repo privado para codigo corporativo.

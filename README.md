# RustDesk Monitoring MVP (CLI + Webhook + Presence)

Implementacion MVP basada en el plan de monitoreo y auditoria:
- Control de grabacion por CLI (entrante/saliente, auto/manual/off).
- Ingestion de eventos de sesion en server.
- Outbox SQLite con entrega webhook, HMAC y retries exponenciales.
- Presencia remota colaborativa (participantes, control activo, actividad).
- Idempotencia por `event_id`.
- Metricas y logs estructurados.

## Componentes

- `monitoring-server`
  - Endpoint interno: `POST /api/v1/session-events`
  - Endpoints de presencia:
    - `GET /api/v1/sessions/:session_id/presence`
    - `GET /api/v1/sessions/:session_id/presence/stream` (SSE en tiempo real)
    - `GET /api/v1/sessions/presence`
  - Endpoints operativos: `GET /health`, `GET /metrics`
  - Cola outbox en SQLite
  - Worker de webhook con retry + circuit breaker
- `rustdesk-cli`
  - Flags de grabacion:
    - `--recording-mode=off|auto|manual`
    - `--recording-incoming=on|off`
    - `--recording-outgoing=on|off`
    - `--recording-storage-path=<ruta>`
  - Comandos runtime:
    - `session start --session-id <id>`
    - `session end --session-id <id>`
    - `recording start --session-id <id>`
    - `recording stop --session-id <id>`
    - `presence join --session-id <id> [--participant-id <id>] [--display-name <name>] [--avatar-url <url>]`
    - `presence leave --session-id <id> [--participant-id <id>]`
    - `presence control --session-id <id> --participant-id <id>`
    - `presence activity --session-id <id> [--participant-id <id>] [--signal <texto>]`
    - `presence show --session-id <id>`
    - `presence sessions`

## Contrato de evento

`SessionEventV1` contiene:
- `event_id` (uuid)
- `event_type` (`session_started|session_ended|recording_started|recording_stopped|participant_joined|participant_left|control_changed|participant_activity`)
- `session_id`
- `user_id`
- `direction` (`incoming|outgoing`)
- `timestamp` (UTC)
- `host_info` (opcional)
- `meta` (objeto JSON opcional)

## Configuracion del server

Usa `server-config.example.toml` como base.

Opciones principales:
- `webhook.enabled`
- `webhook.url`
- `webhook.method`
- `webhook.timeout_ms`
- `webhook.retry.max_attempts`
- `webhook.retry.backoff_ms`
- `webhook.hmac.enabled`
- `webhook.hmac.secret`

## Ejecucion

1. Instalar Rust (`rustup`) para obtener `cargo` y `rustc`.
2. Instalar compilador C (`cc/gcc/clang`) para dependencias nativas (`ring`, `sqlite`).
3. Compilar:

```bash
cargo build
```

4. Levantar server:

```bash
cargo run --bin monitoring-server -- \
  --config ./server-config.example.toml \
  --database-path ./data/outbox.db \
  --bind 0.0.0.0:8080
```

5. Simular sesion desde CLI:

```bash
cargo run --bin rustdesk-cli -- \
  --server-url http://127.0.0.1:8080 \
  --user-id admin01 \
  --recording-mode auto \
  session start --session-id ses-001
```

```bash
cargo run --bin rustdesk-cli -- session end --session-id ses-001
```

6. Probar presencia colaborativa:

```bash
cargo run --bin rustdesk-cli -- \
  --server-url http://127.0.0.1:8080 \
  presence join --session-id ses-001 --participant-id bob --display-name Bob
```

```bash
cargo run --bin rustdesk-cli -- \
  --server-url http://127.0.0.1:8080 \
  presence control --session-id ses-001 --participant-id bob
```

```bash
cargo run --bin rustdesk-cli -- \
  --server-url http://127.0.0.1:8080 \
  presence show --session-id ses-001
```

7. Probar stream en tiempo real (SSE):

```bash
curl -N http://127.0.0.1:8080/api/v1/sessions/ses-001/presence/stream
```

En otra terminal, dispara cambios:

```bash
cargo run --bin rustdesk-cli -- \
  --server-url http://127.0.0.1:8080 \
  presence activity --session-id ses-001 --participant-id bob --signal mouse_move
```

## Notas

- Si `webhook.enabled=true`, se exige `webhook.url`.
- Si `webhook.hmac.enabled=true`, se exige `webhook.hmac.secret`.
- El server retorna:
  - `202 Accepted` evento encolado.
  - `409 Conflict` `event_id` duplicado.
  - `4xx` payload invalido.

## Fork RustDesk (Ubuntu)

Para compilar el fork real de RustDesk en Linux y conectar eventos al `monitoring-server`:

```bash
bash scripts/install-rustdesk-ubuntu-deps.sh
bash scripts/check-rustdesk-fork.sh
```

Guia detallada:
- `docs/rustdesk-fork-setup-ubuntu.md`

## Antes de presentar: que faltaba

- Publicar este proyecto en un repo remoto (GitHub/GitLab) para que tu jefe lo pueda clonar.
- Publicar tambien tu fork de RustDesk con los cambios de `feature/monitoring-events`.
- Tener una URL/IP accesible del `monitoring-server` desde la red donde hara la prueba.
- Definir receptor webhook de prueba (si quieren validar entrega/HMAC en vivo).
- Para prueba completa de presencia, idealmente usar el fork en ambos extremos de la sesion.

## Publicar este proyecto para clonado

```bash
cd /home/choy/Escritorio/Reto
git init
git add .
git commit -m "MVP monitoreo RustDesk: CLI + Webhook + Presence"
git branch -M main
git remote add origin <URL_REPO_RETO>
git push -u origin main
```

Haz lo mismo con tu fork de RustDesk y sube la rama:

```bash
cd /home/choy/Escritorio/rustdesk
git push -u origin feature/monitoring-events
```

## Prueba para jefe en Linux

1. Clonar y levantar server:

```bash
git clone <URL_REPO_RETO>
cd Reto
bash scripts/run-monitoring-server.sh
```

2. Iniciar RustDesk fork apuntando al server:

```bash
export RUSTDESK_MONITORING_URL="http://IP_DEL_SERVER:8080"
/ruta/a/rustdesk-fork/target/debug/rustdesk
```

3. Verificar recepcion de eventos:

```bash
curl -s http://IP_DEL_SERVER:8080/metrics
curl -s http://IP_DEL_SERVER:8080/api/v1/sessions/presence
```

## Prueba para jefe en Windows

Requisitos minimos:
- Git
- Rustup (cargo/rustc)
- Visual Studio Build Tools (C++ workload), si va a compilar.

1. Clonar y levantar server desde PowerShell:

```powershell
git clone <URL_REPO_RETO>
cd Reto
powershell -ExecutionPolicy Bypass -File .\scripts\run-monitoring-server.ps1
```

2. Ejecutar RustDesk fork con variable de entorno:

```powershell
$env:RUSTDESK_MONITORING_URL = "http://IP_DEL_SERVER:8080"
Start-Process "C:\ruta\tu-rustdesk-fork\rustdesk.exe"
```

3. Hacer una sesion real y validar:

```powershell
curl.exe http://IP_DEL_SERVER:8080/metrics
curl.exe http://IP_DEL_SERVER:8080/api/v1/sessions/presence
```

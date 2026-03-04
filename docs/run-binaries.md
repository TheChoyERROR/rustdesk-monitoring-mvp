# Guia de ejecucion de binarios (Linux y Windows)

Este documento explica como correr los componentes del MVP en local o red interna:
- Backend `monitoring-server`
- Frontend `web-dashboard`
- CLI `rustdesk-cli`
- Binario del fork `rustdesk`

## 1) Backend `monitoring-server`

### Linux (recomendado con script)
```bash
cd /home/choy/Escritorio/Reto
bash scripts/run-monitoring-server.sh
```

Notas:
- El script recompila `target/release/monitoring-server` en cada arranque.
- Para saltar compilacion: `SKIP_BUILD=1 bash scripts/run-monitoring-server.sh`

### Linux (binario directo)
```bash
cd /home/choy/Escritorio/Reto
cargo build --release --bin monitoring-server
./target/release/monitoring-server \
  --config ./server-config.example.toml \
  --database-path ./data/outbox.db \
  --bind 0.0.0.0:8080
```

### Windows PowerShell (recomendado con script)
```powershell
cd C:\ruta\rustdesk-monitoring-mvp
powershell -ExecutionPolicy Bypass -File .\scripts\run-monitoring-server.ps1
```

Notas:
- El script recompila `target/release/monitoring-server.exe` en cada arranque.
- Para saltar compilacion: `-SkipBuild`

### Windows PowerShell (binario directo)
```powershell
cd C:\ruta\rustdesk-monitoring-mvp
cargo build --release --bin monitoring-server
.\target\release\monitoring-server.exe `
  --config .\server-config.example.toml `
  --database-path .\data\outbox.db `
  --bind 0.0.0.0:8080
```

## 2) Frontend `web-dashboard`

### Linux / Windows (modo desarrollo)
```bash
cd /home/choy/Escritorio/Reto/web-dashboard
npm install
npm run dev -- --host 0.0.0.0 --port 5173
```

Alternativa con scripts del repo:
- Linux: `bash scripts/run-dashboard-dev.sh`
- Windows: `powershell -ExecutionPolicy Bypass -File .\scripts\run-dashboard-dev.ps1`

### Build de produccion (validacion local)
```bash
cd /home/choy/Escritorio/Reto/web-dashboard
npm run build
npm run preview -- --host 0.0.0.0 --port 5173
```

## 3) CLI `rustdesk-cli` (pruebas de eventos)

### Iniciar sesion de prueba
```bash
cd /home/choy/Escritorio/Reto
cargo run --bin rustdesk-cli -- \
  --server-url http://127.0.0.1:8080 \
  --user-id supervisor \
  session start --session-id worker-001
```

### Actividad/presencia
```bash
cd /home/choy/Escritorio/Reto
cargo run --bin rustdesk-cli -- \
  --server-url http://127.0.0.1:8080 \
  presence join --session-id worker-001 \
  --participant-id empleado1 \
  --display-name "Empleado 1"
```

### Cierre de sesion
```bash
cd /home/choy/Escritorio/Reto
cargo run --bin rustdesk-cli -- \
  --server-url http://127.0.0.1:8080 \
  session end --session-id worker-001
```

## 4) Binario del fork `rustdesk` (cliente corporativo)

Este paso corre en el repo del fork (ejemplo: `/home/choy/Escritorio/rustdesk`).

### Linux bash
```bash
cd /home/choy/Escritorio/rustdesk
RUSTDESK_MONITORING_URL="http://192.168.0.103:8080" ./target/release/rustdesk
```

### Linux fish
```fish
cd /home/choy/Escritorio/rustdesk
set -gx RUSTDESK_MONITORING_URL "http://192.168.0.103:8080"
./target/release/rustdesk
```

### Windows PowerShell
```powershell
cd C:\ruta\rustdesk
$env:RUSTDESK_MONITORING_URL="http://192.168.0.103:8080"
.\target\release\rustdesk.exe
```

## 5) Verificacion minima despues de arrancar

Backend:
```bash
curl -s http://127.0.0.1:8080/health
curl -s http://127.0.0.1:8080/metrics
curl -s http://127.0.0.1:8080/api/v1/sessions/presence
```

Dashboard:
- URL: `http://127.0.0.1:5173`
- Login default: `supervisor / ChangeMeNow123!` (cambiar en entornos reales)

## 6) Troubleshooting rapido

1. Sesiones fantasma no expiran:
- Asegura que arrancaste backend actualizado.
- Usa el script sin `SKIP_BUILD`.
- Revisa logs de arranque: `presence cleanup configuration`.

2. Webhook fallido alto:
- Si `webhook.url` esta en `https://example.com/...`, es solo placeholder.
- Configura URL real del receptor y valida red/timeout.

3. Puerto ocupado:
- Cambia `--bind` (por ejemplo `0.0.0.0:8081`) o libera el proceso anterior.

4. Error de dependencias en fork RustDesk:
- Revisar guia: `docs/rustdesk-fork-setup-ubuntu.md`

## 7) Siguiente paso (manana)

Siguiente hito recomendado: generar instalador corporativo del fork de RustDesk
(Windows primero, Linux despues), con `RUSTDESK_MONITORING_URL` preconfigurada.

Guia y script listos:
- `docs/windows-installer.md`
- `scripts/build-rustdesk-windows-installer.ps1`

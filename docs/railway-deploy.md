# Deploy en Railway (backend + dashboard en un solo servicio)

Esta guia despliega una sola app en Railway usando el `Dockerfile` del repo.
El backend sirve la API y tambien los archivos del dashboard (`web-dashboard/dist`),
asi todo queda en el mismo dominio y las cookies de login funcionan sin CORS extra.

## 1) Requisitos previos

- Repo subido a GitHub.
- Cuenta de Railway conectada a GitHub.

## 2) Crear proyecto en Railway

1. `New Project` -> `Deploy from GitHub Repo`.
2. Selecciona este repo.
3. Railway detectara el `Dockerfile` en la raiz y construira la imagen.

## 3) Configurar volumen persistente (SQLite)

1. En el servicio, abre `Settings` -> `Volumes`.
2. Agrega un volumen y montalo en:
   - `/app/data`

Sin volumen, SQLite se pierde en redeploy/restart.

## 4) Variables de entorno recomendadas

En `Variables` del servicio:

- `DASHBOARD_SUPERVISOR_USERNAME=supervisor`
- `DASHBOARD_SUPERVISOR_PASSWORD=<una-clave-fuerte>`
- `DASHBOARD_COOKIE_SECURE=true`
- `RUST_LOG=info`

## 5) Config runtime usada

El contenedor arranca con:

- Config: `/app/server-config.railway.toml`
- DB: `/app/data/outbox.db`
- Bind: `0.0.0.0:$PORT`

`server-config.railway.toml` viene con webhook deshabilitado por defecto
para evitar reintentos contra URL placeholder.

## 6) Verificacion post-deploy

Con la URL publica de Railway (`https://<tu-app>.up.railway.app`):

- `GET /health` -> `{"status":"ok"}`
- `GET /metrics` -> metricas Prometheus
- Abrir `/` -> dashboard login

## 7) Habilitar webhook (opcional)

Edita `server-config.railway.toml` y cambia:

- `webhook.enabled=true`
- `webhook.url=<https://tu-endpoint>`
- `webhook.hmac.enabled=true`
- `webhook.hmac.secret=<secret-fuerte>`

Luego commit + push para redeploy.

## 8) Troubleshooting rapido

1. Login no funciona:
- Verifica `DASHBOARD_SUPERVISOR_PASSWORD` en variables y redeploy.

2. Se pierden datos:
- Confirma volumen montado en `/app/data`.

3. UI carga pero API falla:
- Revisa logs del servicio.
- Valida `GET /health`.

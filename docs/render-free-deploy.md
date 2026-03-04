# Deploy en Render Free (demo rapida)

Este flujo despliega backend + dashboard en un solo servicio web usando Docker.

## Limitaciones importantes en Free

- El servicio puede entrar en sleep por inactividad (cold start al volver).
- No hay disco persistente en free para este caso: SQLite sera efimero.
  - Datos pueden perderse al reiniciar/redeploy.

Para demo funciona; para operacion estable conviene plan pago con disco persistente
o migrar a Postgres.

## 1) Requisitos

- Repo en GitHub con:
  - `Dockerfile`
  - `render.yaml`
- Cuenta Render conectada a GitHub.

## 2) Crear el servicio

1. En Render: `New` -> `Blueprint`.
2. Selecciona tu repo.
3. Render detectara `render.yaml`.
4. Confirma deploy.

Alternativa manual:
- `New` -> `Web Service` -> `Deploy from existing repository`.
- Runtime: Docker.
- Plan: Free.
- Health check path: `/health`.

## 3) Variables de entorno

Configura en Render:

- `DASHBOARD_SUPERVISOR_USERNAME=supervisor`
- `DASHBOARD_SUPERVISOR_PASSWORD=<clave-fuerte>`
- `DASHBOARD_COOKIE_SECURE=true`
- `RUST_LOG=info`
- `WEBHOOK_ENABLED=false` (default en demo)
- `WEBHOOK_HMAC_ENABLED=false` (default en demo)

## 4) Validacion

Con la URL publica de Render (`https://<service>.onrender.com`):

- `GET /health` -> `{"status":"ok"}`
- `GET /metrics` -> metricas
- `GET /` -> dashboard login

## 5) Checklist de demo

1. Loguearte al dashboard.
2. Enviar eventos de prueba con `rustdesk-cli`.
3. Ver que resumen y sesiones cambian.
4. Aclarar en demo que storage en free es temporal.

## 6) Siguiente paso despues de validar

1. Pasar a plan con persistencia o VPS.
2. Habilitar webhook real (`webhook.url` + HMAC secret).
3. Cambiar password supervisor y rotar secretos.

## 7) Activar webhook para prueba de jefe

Si quieres mostrar flujo completo de entrega webhook en Render:

1. Crea endpoint temporal en `https://webhook.site/` y copia tu URL unica.
2. En Variables de Render configura:
   - `WEBHOOK_ENABLED=true`
   - `WEBHOOK_URL=<tu-url-webhook.site>`
   - `WEBHOOK_METHOD=POST`
   - `WEBHOOK_HMAC_ENABLED=true`
   - `WEBHOOK_HMAC_SECRET=<secret-fuerte>`
   - `WEBHOOK_MAX_ATTEMPTS=5`
   - `WEBHOOK_BACKOFF_MS=500`
3. Haz `Manual Deploy` -> `Clear build cache & deploy`.
4. Genera eventos y valida:
   - Dashboard: sube `webhook_entregados`
   - webhook.site: recibe payloads firmados (`x-signature`, `x-event-id`, `x-event-type`)

# Checklist de preparacion para Turso

Fecha: 2026-04-01

## Estado actual

Base creada:

- `rustdesk-monitoring-mvp`
- URL: `libsql://rustdesk-monitoring-mvp-thechoyerror.aws-us-west-2.turso.io`

## Importante

El token de Turso es un secreto. No debe subirse al repo ni quedar en archivos versionados.

En esta fase vamos a usarlo solo por variable de entorno o en la configuracion local de la CLI.

## Objetivo de esta fase

Dejar Turso listo del lado de infraestructura para que luego el backend pueda adaptarse con el menor riesgo posible.

En esta fase:

- si vamos a preparar Turso;
- no vamos a apuntar el backend productivo actual a Turso todavia;
- no vamos a borrar ni reemplazar la SQLite actual.

## Paso 1. Instalar y autenticar CLI

Documentacion oficial:

- `https://docs.turso.tech/cli/installation`
- `https://docs.turso.tech/cli/auth/login`

En Windows, para esta integracion conviene usar WSL.

Nota practica:

- el binario `turso-bootstrap` compila en Windows;
- pero la ejecucion estable del bootstrap remoto quedo validada en WSL/Linux;
- para esta fase, toma WSL como entorno de trabajo recomendado.

Comandos:

```bash
wsl
curl -sSfL https://get.tur.so/install.sh | bash
source ~/.bashrc
turso auth login
```

Si hace falta modo headless:

```bash
turso auth login --headless
```

## Paso 2. Verificar la base

Comandos:

```bash
turso db show rustdesk-monitoring-mvp
turso db show --url rustdesk-monitoring-mvp
turso db shell rustdesk-monitoring-mvp
```

Prueba minima dentro del shell:

```sql
SELECT 1;
```

No crear schema manual del proyecto todavia.

Si esto responde bien, ya tenemos:

- CLI OK;
- conectividad OK;
- shell SQL OK.

## Paso 3. Preparar variables para la fase de codigo

Los nombres recomendados para variables del backend son:

```text
TURSO_DATABASE_URL=libsql://rustdesk-monitoring-mvp-thechoyerror.aws-us-west-2.turso.io
TURSO_AUTH_TOKEN=<token>
```

Todavia no las uses en Render hasta que el backend soporte Turso.

## Paso 4. Bootstrap del schema desde el repo

Una vez instalada la CLI y validado `SELECT 1`, ya podemos ejecutar el spike desde WSL.

Comandos:

```bash
source ~/.cargo/env
cd /mnt/c/Users/Choy/Desktop/rustdesk-monitoring-mvp
export TURSO_DATABASE_URL="libsql://rustdesk-monitoring-mvp-thechoyerror.aws-us-west-2.turso.io"
export TURSO_AUTH_TOKEN="<token>"
./scripts/run-turso-bootstrap.sh
```

Resultado esperado:

- conecta a Turso;
- aplica el schema base;
- lista las tablas detectadas.

## Paso 4.1. Smoke real de helpdesk sobre Turso

Una vez que el bootstrap ya paso, el siguiente comando valida operaciones reales del dominio:

- siembra supervisor del dashboard;
- autoriza un agente;
- crea un ticket;
- lee tickets y resumen operativo desde Turso.

Comandos:

```bash
source ~/.cargo/env
cd /mnt/c/Users/Choy/Desktop/rustdesk-monitoring-mvp
export TURSO_DATABASE_URL="libsql://rustdesk-monitoring-mvp-thechoyerror.aws-us-west-2.turso.io"
export TURSO_AUTH_TOKEN="<token>"
./scripts/run-turso-helpdesk-smoke.sh \
  --agent-id "419797027" \
  --agent-name "Edward soporte" \
  --client-id "client-turso-smoke-001" \
  --client-name "Cliente Turso Smoke" \
  --title "Prueba Turso Smoke" \
  --description "Validacion de escritura helpdesk sobre Turso" \
  --difficulty "Medium" \
  --estimated-minutes 15 \
  --summary "Smoke test Turso"
```

Resultado esperado:

- ticket creado en estado `queued`;
- total de tickets mayor que cero;
- resumen operativo consistente.

## Paso 5. Opcional: crear branch de trabajo en Turso

Si la interfaz o CLI te deja usar branches, conviene crear una branch de pruebas.

Nombre sugerido:

```text
staging
```

Objetivo:

- probar migracion y schema sin tocar la base principal.

Si no lo ves claro o tu plan no lo expone facil, no bloquea el trabajo inicial.

## Paso 6. No importar datos todavia

Aunque Turso soporta migrar una SQLite existente con `db import`, todavia no conviene hacerlo hasta definir cual sera el flujo final del backend.

Documentacion oficial:

- `https://docs.turso.tech/cloud/migrate-to-turso`

Esto se deja para la siguiente fase porque primero hay que decidir:

- remoto puro;
- o una adaptacion de storage sobre `libsql`;
- o una estrategia de migracion mas gradual.

## Estado real al cierre de esta fase

Ya esta validado:

- base remota creada;
- CLI y shell operativos;
- conexion desde Rust hacia Turso;
- bootstrap remoto del schema del proyecto.
- escritura y lectura basica de helpdesk sobre Turso.

Todavia no esta validado:

- usar Turso como storage principal del `monitoring-server`;
- reemplazo completo de SQLite local;
- migracion de datos productivos.

## Lo que sigue del lado del codigo

Una vez Turso quede listo, el siguiente bloque tecnico en el proyecto es:

1. abstraer apertura de base / storage backend;
2. decidir si el runtime pasa a `libsql` remoto;
3. extender el spike actual hacia lecturas y escrituras reales del dominio;
4. luego recien decidir la migracion completa.

## Decision tecnica recomendada

La ruta mas prudente es:

1. preparar Turso;
2. dejar el bootstrap reproducible;
3. validar operaciones criticas:
   - outbox
   - tickets
   - agentes
   - transacciones
4. despues migrar.

## Senales de que ya estamos listos para la siguiente fase

- CLI instalada y autenticada;
- base visible por `turso db show`;
- URL confirmada;
- shell responde `SELECT 1;`;
- bootstrap remoto ejecutado con exito.

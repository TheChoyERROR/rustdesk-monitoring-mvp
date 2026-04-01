# Estrategia de build para fork RustDesk en Windows

## Recomendacion

Para compilar el fork de RustDesk y generar un instalador real de Windows, la ruta principal debe ser **build local en Windows**, no Docker.

## Por que no Docker como camino principal

Docker sirve bien para:

- backend del proyecto
- pruebas de servicios
- empaquetado reproducible de componentes server-side

Docker no simplifica este caso porque el objetivo es:

- compilar una app desktop Windows
- usar toolchain MSVC
- integrar Flutter y dependencias nativas
- generar un instalador Windows
- validar el comportamiento del binario final en Windows

Eso normalmente termina dependiendo de:

- Visual Studio Build Tools
- Windows SDK
- Flutter desktop para Windows
- vcpkg
- NSIS o MSI/WiX

Intentar resolver esto con Docker agrega complejidad extra:

- contenedores Windows mas pesados
- peor ergonomia para Flutter desktop
- mas friccion con Visual Studio Build Tools y SDK
- menos claridad para depurar problemas de build del fork
- no elimina la necesidad de probar el instalador en Windows real

## Cuando si puede ayudar Docker

Docker podria ayudar mas adelante para:

- levantar backend de monitoreo local
- estandarizar entorno del servidor
- pruebas de integracion del API

No es la herramienta correcta para compilar y empaquetar el cliente Windows del fork como primer paso.

## Decision recomendada

1. Preparar una PC Windows como entorno de build del fork.
2. Compilar el binario del fork localmente.
3. Generar instalador de prueba con el script de este repo.
4. Probar el instalador en una VM o maquina limpia.
5. Solo despues evaluar si conviene automatizar CI/CD o MSI mas formal.

## Cambios del fork detectados localmente

Del analisis del fork local, si estoy viendo cambios relacionados con monitoreo corporativo:

- modulo de eventos de monitoreo y envio a backend
- soporte de `RUSTDESK_MONITORING_URL`
- configuracion de `monitoring-server-url`
- avatar por usuario via `monitoring-avatar-url` o `monitoring-avatar-path`
- conversion de avatar local a `data:image/...`
- emision de eventos de sesion, participante, control y actividad
- dialogo/perfil de monitoreo en UI
- aviso de politica de monitoreo en UI

Eso indica que para empaquetar no conviene tratar el fork como binario generico: hay que preservar y rastrear exactamente ese ejecutable compilado.

## Script de chequeo

Usa este comando para ver si la maquina esta lista:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\check-rustdesk-windows-build-env.ps1
```

## Script de instalacion asistida

Para ver los comandos que va a ejecutar, sin instalar nada:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install-rustdesk-windows-build-deps.ps1
```

Para ejecutar la instalacion real:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install-rustdesk-windows-build-deps.ps1 -Execute
```

Despues de eso, vuelve a correr:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\check-rustdesk-windows-build-env.ps1
```

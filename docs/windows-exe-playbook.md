# Playbook Windows: Instalar o Crear `.exe` / Instalador

## Objetivo
Este documento define que necesita una PC Windows para:
1. Instalar el cliente corporativo ya empaquetado.
2. Compilar desde codigo y generar un instalador `.exe` del fork de RustDesk.

No reemplaza la guia de empaquetado corporativo:
- [windows-installer.md](/home/choy/Escritorio/Reto/docs/windows-installer.md)

## Escenario A: Solo instalar el cliente (usuario final)
Requisitos minimos:
- Windows 10/11 (64 bits).
- Permisos para ejecutar instaladores.
- Acceso al instalador generado (`rustdesk-<version>-install.exe` o paquete portable).

Pasos:
1. Ejecutar el instalador como administrador.
2. Completar instalacion.
3. Abrir RustDesk corporativo y validar conexion al backend de monitoreo.

Validacion:
- El dashboard debe reflejar nuevos eventos de sesion/presencia.
- En caso de bloqueo SmartScreen, usar "Mas informacion" -> "Ejecutar de todos modos" (si el archivo proviene de fuente confiable interna).

## Escenario B: Crear `.exe` desde una PC Windows (build local)
Este flujo aplica al repo del fork de RustDesk (separado del repo `Reto`).

### 1) Requisitos de herramientas
- Git.
- Python 3.10+ y `pip`.
- Rust (toolchain MSVC): `stable-x86_64-pc-windows-msvc`.
- Visual Studio 2022 Build Tools:
  - MSVC C++ x64/x86 build tools.
  - Windows 10/11 SDK.
  - CMake tools for C++.
- Flutter (si se usa pipeline Flutter).
- `vcpkg` para dependencias nativas.

### 2) Dependencias nativas con vcpkg
En PowerShell:

```powershell
git clone https://github.com/microsoft/vcpkg C:\vcpkg
C:\vcpkg\bootstrap-vcpkg.bat
setx VCPKG_ROOT C:\vcpkg
$env:VCPKG_ROOT = "C:\vcpkg"
& "$env:VCPKG_ROOT\vcpkg.exe" install libvpx:x64-windows-static libyuv:x64-windows-static opus:x64-windows-static aom:x64-windows-static
```

### 3) Clonar fork y rama
```powershell
git clone https://github.com/TheChoyERROR/rustdesk.git
cd rustdesk
git checkout feature/monitoring-events
```

### 4) Build base en Windows
```powershell
python build.py
```

Salidas esperadas:
- `target\release\RustDesk.exe`
- instalador en raiz (segun version/pipeline), por ejemplo `rustdesk-<version>-win7-install.exe`

## Escenario C: Empaquetado corporativo desde este repo (`Reto`)
Si ya tienes un `rustdesk.exe` compilado y solo quieres empaquetado corporativo:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\build-rustdesk-windows-installer.ps1 `
  -RustDeskRepoPath "C:\ruta\al\fork\rustdesk" `
  -MonitoringUrl "https://tu-backend-monitoring" `
  -OutputDir ".\artifacts\windows-installer"
```

Opciones utiles:
- `-BuildRustDesk`: ejecuta build del fork antes de empaquetar.
- `-SkipNsis`: omite el instalador NSIS y deja paquete portable/staging.

## Checklist de verificacion final
1. Instalar paquete en una PC limpia.
2. Abrir cliente y conectar a una sesion real.
3. Verificar en backend:
   - `GET /health`
   - `GET /metrics`
   - `GET /api/v1/sessions/presence`
4. Verificar en dashboard:
   - Sesion en Resumen.
   - Presencia/timeline en Detalle.
   - Si aplica, eventos webhook entregados.

## Errores comunes
- "No se puede compilar": faltan Build Tools de Visual Studio o SDK.
- "No encuentra librerias nativas": `VCPKG_ROOT` no configurado o puertos no instalados.
- "Instalador no se genera": build incompleto o ruta de `rustdesk.exe` incorrecta.
- "No aparecen eventos": cliente oficial en vez del fork corporativo, o URL de monitoreo no configurada.

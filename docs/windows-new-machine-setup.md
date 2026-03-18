# Windows New Machine Setup

Esta guia deja una maquina Windows lista para trabajar con:

- backend `monitoring-server`
- dashboard `web-dashboard`
- fork `rustdesk-fork`
- build de `rustdesk.exe`
- empaquetado `setup.exe`

## 1. Clonar repos

Desde una carpeta de trabajo:

```powershell
git clone https://github.com/TheChoyERROR/rustdesk-monitoring-mvp.git
cd rustdesk-monitoring-mvp
git clone --branch feature/monitoring-events https://github.com/TheChoyERROR/rustdesk.git rustdesk-fork
```

Validacion rapida:

```powershell
git rev-parse --short HEAD
git -C .\rustdesk-fork rev-parse --short HEAD
```

Valores esperados al momento de esta guia:

- repo principal: `8940207`
- fork: `a917fc630`

## 1.5 Bootstrap de una sola pasada

Si quieres una sola orden para preparar la maquina, usa:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\bootstrap-windows-dev-machine.ps1 -Execute -BuildRustDesk -BuildInstaller
```

Si todavia no tienes `tools\flutter-3.24.5`, ejecuta primero sin `-BuildRustDesk` y sin `-BuildInstaller`,
o pasa `-FlutterRoot "C:\ruta\flutter-3.24.5"`.

## 2. Dependencias de Windows para el fork

Recomendado ejecutar PowerShell como administrador:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install-rustdesk-windows-build-deps.ps1 -Execute
```

Esto instala/prepara:

- Visual Studio 2022 Build Tools
- CMake
- NSIS
- Rustup
- `vcpkg`
- paquetes nativos requeridos

Luego valida el entorno:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\check-rustdesk-windows-build-env.ps1
```

## 3. Flutter correcto

El fork fue ajustado para usar Flutter `3.24.5`.

Opcion recomendada:

- copiar `tools\flutter-3.24.5` desde la maquina anterior al mismo path dentro de este repo

Alternativa:

- instalar Flutter `3.24.5` en otra ruta
- definir `RUSTDESK_FLUTTER_ROOT` apuntando a ese SDK

Ejemplo:

```powershell
$env:RUSTDESK_FLUTTER_ROOT="C:\dev\flutter-3.24.5"
$env:FLUTTER_ROOT=$env:RUSTDESK_FLUTTER_ROOT
```

## 4. Backend y dashboard

Backend:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\run-monitoring-server.ps1
```

Dashboard en dev:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\run-dashboard-dev.ps1
```

## 5. Compilar `rustdesk.exe`

Desde `rustdesk-fork`:

```powershell
cd .\rustdesk-fork
python build.py --flutter --skip-portable-pack
```

Si solo quieres validar la parte Flutter y ya tienes Cargo listo:

```powershell
python build.py --flutter --skip-cargo --skip-portable-pack
```

Salida esperada:

- `rustdesk-fork\flutter\build\windows\x64\runner\Release\rustdesk.exe`

## 6. Crear instalador

Desde el repo principal:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\build-rustdesk-windows-test-installer.ps1 `
  -MonitoringUrl "https://rustdesk-monitoring-mvp.onrender.com" `
  -CompanyName "RustDesk Monitoring MVP"
```

O usando el script completo:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\build-rustdesk-windows-installer.ps1 `
  -RustDeskRepoPath .\rustdesk-fork `
  -MonitoringUrl "https://rustdesk-monitoring-mvp.onrender.com" `
  -CompanyName "RustDesk Monitoring MVP" `
  -BuildRustDesk
```

Salida esperada:

- `artifacts\windows-installer\...\*-setup.exe`
- `artifacts\windows-installer\...\*-portable.zip`

## 7. Flujo recomendado para issues

- cambios de backend/dashboard: repo principal
- cambios del cliente RustDesk: `rustdesk-fork`
- commitear y pushear ambos repos por separado
- no usar `git add .` si hay artefactos locales o cambios de build

## 8. Nota importante

El repo principal no versiona `tools\` ni empaqueta automaticamente el SDK de Flutter.
Si necesitas builds reproducibles en otra maquina, copia `tools\flutter-3.24.5`
desde una maquina ya preparada o instala manualmente esa misma version.

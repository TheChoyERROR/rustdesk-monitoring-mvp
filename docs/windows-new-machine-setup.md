# Windows New Machine Setup

Esta guia deja una maquina Windows lista para:

- backend `monitoring-server`
- dashboard `web-dashboard`
- fork `rustdesk-fork`
- build de `rustdesk.exe`
- empaquetado `setup.exe`

## 1. Camino corto: clone y bootstrap

Desde una carpeta de trabajo:

```powershell
git clone https://github.com/TheChoyERROR/rustdesk-monitoring-mvp.git
cd rustdesk-monitoring-mvp
powershell -ExecutionPolicy Bypass -File .\scripts\bootstrap-windows-dev-machine.ps1 -Execute -BuildInstaller
```

Ese bootstrap hace esto:

- clona `rustdesk-fork` si no existe;
- fija el fork en `a917fc630baf0ffa1eb0982b108571e8a9952be7`;
- inicializa sus submodulos;
- aplica el overlay versionado de `patches\rustdesk-fork\`;
- descarga Flutter `3.24.5` dentro de `tools\flutter-3.24.5` si falta;
- instala dependencias Windows;
- valida el entorno;
- compila backend;
- genera el instalador de prueba.

Salida esperada:

- `artifacts\windows-installer\...\*-setup.exe`
- `artifacts\windows-installer\...\*-portable.zip`

## 2. Si solo quieres preparar la maquina

```powershell
git clone https://github.com/TheChoyERROR/rustdesk-monitoring-mvp.git
cd rustdesk-monitoring-mvp
powershell -ExecutionPolicy Bypass -File .\scripts\bootstrap-windows-dev-machine.ps1 -Execute
```

Luego, cuando quieras crear el instalador:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\build-rustdesk-windows-test-installer.ps1 `
  -MonitoringUrl "https://rustdesk-monitoring-mvp.onrender.com" `
  -CompanyName "RustDesk Monitoring MVP"
```

## 3. Si quieres controlar cada paso

### 3.1 Dependencias Windows

Recomendado abrir PowerShell como administrador:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install-rustdesk-windows-build-deps.ps1 -Execute
```

Esto instala o prepara:

- Visual Studio 2022 Build Tools
- CMake
- NSIS
- Rustup
- `vcpkg`
- Flutter `3.24.5` dentro del repo
- paquetes nativos requeridos

Validacion:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\check-rustdesk-windows-build-env.ps1
```

### 3.2 Fork reproducible

Si no existe:

```powershell
git clone --branch feature/monitoring-events https://github.com/TheChoyERROR/rustdesk.git rustdesk-fork
git -C .\rustdesk-fork checkout a917fc630baf0ffa1eb0982b108571e8a9952be7
git -C .\rustdesk-fork submodule update --init --recursive
powershell -ExecutionPolicy Bypass -File .\scripts\apply-rustdesk-fork-patches.ps1 -Execute
```

Si el fork ya existe, asegurate de que este limpio antes de reaplicar:

```powershell
git -C .\rustdesk-fork status --short
```

### 3.3 Backend y dashboard

Backend:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\run-monitoring-server.ps1
```

Dashboard en dev:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\run-dashboard-dev.ps1
```

### 3.4 Compilar `rustdesk.exe`

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

### 3.5 Crear instalador

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

## 4. Como queda la reproducibilidad

El repo principal sigue sin versionar `tools\` ni `rustdesk-fork\`, pero ahora el flujo reproducible ya no depende de copiar carpetas a mano:

- el fork se clona desde una base fija;
- sus submodulos se inicializan en bootstrap y en el build;
- los cambios del fork viven como patch en `patches\rustdesk-fork\`;
- Flutter `3.24.5` se descarga automaticamente;
- el build del instalador reaplica los patches del fork antes de compilar.

## 5. Nota operativa

Si `rustdesk-fork` tiene cambios locales, el bootstrap no va a forzar `checkout` ni reaplicar patches por encima. En una maquina nueva lo correcto es partir de un clon limpio del repo principal y dejar que el bootstrap prepare el resto.

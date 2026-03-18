param(
  [switch]$Execute,
  [string]$ForkRepoUrl = "https://github.com/TheChoyERROR/rustdesk.git",
  [string]$ForkBranch = "feature/monitoring-events",
  [string]$ForkPath = "",
  [string]$FlutterRoot = "",
  [string]$MonitoringUrl = "https://rustdesk-monitoring-mvp.onrender.com",
  [string]$CompanyName = "RustDesk Monitoring MVP",
  [switch]$SkipCloneFork,
  [switch]$SkipInstallDeps,
  [switch]$SkipCheckEnv,
  [switch]$SkipDashboardDeps,
  [switch]$SkipBuildBackend,
  [switch]$BuildRustDesk,
  [switch]$BuildInstaller
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($ForkPath)) {
  $ForkPath = Join-Path $repoRoot "rustdesk-fork"
}

$installDepsScript = Join-Path $PSScriptRoot "install-rustdesk-windows-build-deps.ps1"
$checkEnvScript = Join-Path $PSScriptRoot "check-rustdesk-windows-build-env.ps1"
$testInstallerScript = Join-Path $PSScriptRoot "build-rustdesk-windows-test-installer.ps1"

function Invoke-Step {
  param(
    [Parameter(Mandatory = $true)][string]$Description,
    [Parameter(Mandatory = $true)][scriptblock]$Action
  )

  Write-Host ""
  Write-Host "==> $Description"

  if (-not $Execute) {
    Write-Host "dry-run"
    return
  }

  & $Action
}

function Ensure-ExternalCommandSucceeded {
  param([Parameter(Mandatory = $true)][string]$Description)

  if ($LASTEXITCODE -ne 0) {
    throw "Fallo: $Description"
  }
}

Write-Host "Bootstrap Windows para rustdesk-monitoring-mvp"
Write-Host "Repo root: $repoRoot"
Write-Host "Fork path: $ForkPath"
Write-Host "Modo: $(if ($Execute) { 'execute' } else { 'dry-run' })"

if (-not [string]::IsNullOrWhiteSpace($FlutterRoot)) {
  $resolvedFlutterRoot = (Resolve-Path $FlutterRoot).Path
  $env:RUSTDESK_FLUTTER_ROOT = $resolvedFlutterRoot
  $env:FLUTTER_ROOT = $resolvedFlutterRoot
  Write-Host "FlutterRoot forzado: $resolvedFlutterRoot"
} elseif (-not (Test-Path (Join-Path $repoRoot "tools\flutter-3.24.5"))) {
  Write-Warning "No existe tools\flutter-3.24.5 en este repo."
  Write-Warning "Para compilar el fork con la version correcta, copia ese SDK desde la maquina anterior o usa -FlutterRoot."
}

if (-not $SkipCloneFork -and -not (Test-Path $ForkPath)) {
  Invoke-Step -Description "Clonar rustdesk-fork" -Action {
    git clone --branch $ForkBranch $ForkRepoUrl $ForkPath
    Ensure-ExternalCommandSucceeded -Description "git clone rustdesk-fork"
  }
}

if (-not $SkipInstallDeps) {
  Invoke-Step -Description "Instalar dependencias Windows para backend y RustDesk" -Action {
    & powershell -NoProfile -ExecutionPolicy Bypass -File $installDepsScript -Execute
    Ensure-ExternalCommandSucceeded -Description "install-rustdesk-windows-build-deps.ps1"
  }
}

if (-not $SkipCheckEnv) {
  Invoke-Step -Description "Validar entorno de build" -Action {
    & powershell -NoProfile -ExecutionPolicy Bypass -File $checkEnvScript
    Ensure-ExternalCommandSucceeded -Description "check-rustdesk-windows-build-env.ps1"
  }
}

if (-not $SkipDashboardDeps) {
  $dashboardDir = Join-Path $repoRoot "web-dashboard"
  if (Test-Path (Join-Path $dashboardDir "package-lock.json")) {
    Invoke-Step -Description "Instalar dependencias del dashboard" -Action {
      Push-Location $dashboardDir
      try {
        npm ci
        Ensure-ExternalCommandSucceeded -Description "npm ci"
      } finally {
        Pop-Location
      }
    }
  }
}

if (-not $SkipBuildBackend) {
  Invoke-Step -Description "Compilar monitoring-server" -Action {
    Push-Location $repoRoot
    try {
      cargo build --release --bin monitoring-server
      Ensure-ExternalCommandSucceeded -Description "cargo build --release --bin monitoring-server"
    } finally {
      Pop-Location
    }
  }
}

if ($BuildRustDesk) {
  Invoke-Step -Description "Compilar rustdesk.exe del fork" -Action {
    if (-not (Test-Path $ForkPath)) {
      throw "No se encontro el fork en $ForkPath"
    }
    Push-Location $ForkPath
    try {
      python build.py --flutter --skip-portable-pack
      Ensure-ExternalCommandSucceeded -Description "python build.py --flutter --skip-portable-pack"
    } finally {
      Pop-Location
    }
  }
}

if ($BuildInstaller) {
  Invoke-Step -Description "Generar instalador de prueba" -Action {
    & powershell -NoProfile -ExecutionPolicy Bypass -File $testInstallerScript `
      -MonitoringUrl $MonitoringUrl `
      -CompanyName $CompanyName `
      -SkipBuild:(!$BuildRustDesk)
    Ensure-ExternalCommandSucceeded -Description "build-rustdesk-windows-test-installer.ps1"
  }
}

Write-Host ""
Write-Host "Bootstrap finalizado."
Write-Host "Siguiente comando recomendado para backend:"
Write-Host "powershell -ExecutionPolicy Bypass -File .\scripts\run-monitoring-server.ps1"

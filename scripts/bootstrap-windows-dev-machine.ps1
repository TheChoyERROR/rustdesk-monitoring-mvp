param(
  [switch]$Execute,
  [string]$ForkRepoUrl = "https://github.com/TheChoyERROR/rustdesk.git",
  [string]$ForkBranch = "feature/monitoring-events",
  [string]$ForkCommit = "",
  [string]$ForkPath = "",
  [string]$FlutterRoot = "",
  [string]$MonitoringUrl = "https://rustdesk-monitoring-mvp.onrender.com",
  [string]$CompanyName = "RustDesk Monitoring MVP",
  [switch]$SkipCloneFork,
  [switch]$SkipApplyForkPatches,
  [switch]$SkipInstallDeps,
  [switch]$SkipCheckEnv,
  [switch]$SkipDashboardDeps,
  [switch]$SkipBuildBackend,
  [switch]$BuildRustDesk,
  [switch]$BuildInstaller
)

$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "rustdesk-flutter.ps1")

$repoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($ForkPath)) {
  $ForkPath = Join-Path $repoRoot "rustdesk-fork"
}
if (Test-Path $ForkPath) {
  Add-RustDeskGitSafeDirectory -Path (Resolve-Path $ForkPath).Path
}

$installDepsScript = Join-Path $PSScriptRoot "install-rustdesk-windows-build-deps.ps1"
$checkEnvScript = Join-Path $PSScriptRoot "check-rustdesk-windows-build-env.ps1"
$testInstallerScript = Join-Path $PSScriptRoot "build-rustdesk-windows-test-installer.ps1"
$applyForkPatchesScript = Join-Path $PSScriptRoot "apply-rustdesk-fork-patches.ps1"
$forkPatchManifestPath = Join-Path $repoRoot "patches\rustdesk-fork\manifest.json"

if (Test-Path $forkPatchManifestPath) {
  try {
    $forkPatchManifest = Get-Content $forkPatchManifestPath -Raw | ConvertFrom-Json
    if ([string]::IsNullOrWhiteSpace($ForkCommit) -and -not [string]::IsNullOrWhiteSpace($forkPatchManifest.fork_base_commit)) {
      $ForkCommit = $forkPatchManifest.fork_base_commit
    }
    if ($ForkRepoUrl -eq "https://github.com/TheChoyERROR/rustdesk.git" -and -not [string]::IsNullOrWhiteSpace($forkPatchManifest.fork_repo_url)) {
      $ForkRepoUrl = $forkPatchManifest.fork_repo_url
    }
    if ($ForkBranch -eq "feature/monitoring-events" -and -not [string]::IsNullOrWhiteSpace($forkPatchManifest.fork_branch)) {
      $ForkBranch = $forkPatchManifest.fork_branch
    }
  } catch {
    Write-Warning "No se pudo leer $forkPatchManifestPath. Se usan los valores por defecto del script."
  }
}

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
Write-Host "Fork branch: $ForkBranch"
if (-not [string]::IsNullOrWhiteSpace($ForkCommit)) {
  Write-Host "Fork commit base: $ForkCommit"
}
Write-Host "Modo: $(if ($Execute) { 'execute' } else { 'dry-run' })"

if (-not [string]::IsNullOrWhiteSpace($FlutterRoot)) {
  $resolvedFlutterRoot = (Resolve-Path $FlutterRoot).Path
  $env:RUSTDESK_FLUTTER_ROOT = $resolvedFlutterRoot
  $env:FLUTTER_ROOT = $resolvedFlutterRoot
  Write-Host "FlutterRoot forzado: $resolvedFlutterRoot"
} elseif (-not (Test-Path (Join-Path $repoRoot "tools\flutter-3.24.5"))) {
  if ($SkipInstallDeps) {
    Write-Warning "No existe tools\flutter-3.24.5 y has pedido -SkipInstallDeps."
    Write-Warning "Ejecuta scripts\\install-rustdesk-flutter-sdk.ps1 o usa -FlutterRoot antes de compilar."
  } else {
    Write-Host "Flutter 3.24.5 se descargara automaticamente en tools\\flutter-3.24.5 durante el bootstrap."
  }
}

if (-not $SkipCloneFork -and -not (Test-Path $ForkPath)) {
  Invoke-Step -Description "Clonar rustdesk-fork" -Action {
    git clone --branch $ForkBranch $ForkRepoUrl $ForkPath
    Ensure-ExternalCommandSucceeded -Description "git clone rustdesk-fork"
  }
}

if (-not [string]::IsNullOrWhiteSpace($ForkCommit)) {
  Invoke-Step -Description "Fijar rustdesk-fork en la revision base reproducible" -Action {
    if (-not (Test-Path $ForkPath)) {
      throw "No se encontro el fork en $ForkPath"
    }

    $gitStatus = git -C $ForkPath status --porcelain
    Ensure-ExternalCommandSucceeded -Description "git status rustdesk-fork"
    if (-not [string]::IsNullOrWhiteSpace(($gitStatus | Out-String).Trim())) {
      throw "rustdesk-fork tiene cambios locales. Dejalo limpio antes de fijar la revision base."
    }

    $currentCommit = (git -C $ForkPath rev-parse HEAD).Trim()
    Ensure-ExternalCommandSucceeded -Description "git rev-parse rustdesk-fork"
    if ($currentCommit -eq $ForkCommit) {
      Write-Host "rustdesk-fork ya esta en $ForkCommit"
      return
    }

    git -C $ForkPath fetch origin $ForkBranch --tags
    Ensure-ExternalCommandSucceeded -Description "git fetch rustdesk-fork"
    git -C $ForkPath checkout $ForkCommit
    Ensure-ExternalCommandSucceeded -Description "git checkout rustdesk-fork base revision"
  }
}

Invoke-Step -Description "Inicializar submodulos del fork RustDesk" -Action {
  if (-not (Test-Path $ForkPath)) {
    throw "No se encontro el fork en $ForkPath"
  }
  Add-RustDeskGitSafeDirectory -Path (Resolve-Path $ForkPath).Path
  git -C $ForkPath submodule sync --recursive
  Ensure-ExternalCommandSucceeded -Description "git submodule sync rustdesk-fork"
  git -C $ForkPath submodule update --init --recursive
  Ensure-ExternalCommandSucceeded -Description "git submodule update rustdesk-fork"
}

if (-not $SkipApplyForkPatches) {
  Invoke-Step -Description "Aplicar el overlay versionado del fork RustDesk" -Action {
    if (-not (Test-Path $ForkPath)) {
      throw "No se encontro el fork en $ForkPath"
    }
    & powershell -NoProfile -ExecutionPolicy Bypass -File $applyForkPatchesScript -ForkPath $ForkPath -Execute
    Ensure-ExternalCommandSucceeded -Description "apply-rustdesk-fork-patches.ps1"
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

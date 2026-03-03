param(
  [int]$Port = 5173
)

$ErrorActionPreference = "Stop"
$RootDir = Split-Path -Parent $PSScriptRoot
$DashboardDir = Join-Path $RootDir "web-dashboard"

if (-not (Test-Path $DashboardDir)) {
  throw "No existe carpeta dashboard: $DashboardDir"
}

Push-Location $DashboardDir
try {
  if (-not (Test-Path (Join-Path $DashboardDir "node_modules"))) {
    Write-Host "Instalando dependencias del dashboard..."
    npm install
  }

  npm run dev -- --host 0.0.0.0 --port $Port
} finally {
  Pop-Location
}

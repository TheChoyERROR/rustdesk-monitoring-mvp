param(
  [string]$ConfigPath = "",
  [string]$DatabasePath = "",
  [string]$Bind = "0.0.0.0:8080",
  [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$RootDir = Split-Path -Parent $PSScriptRoot
if (-not $ConfigPath) {
  $ConfigPath = Join-Path $RootDir "server-config.example.toml"
}
if (-not $DatabasePath) {
  $DatabasePath = Join-Path $RootDir "data/outbox.db"
}

if (-not (Test-Path $ConfigPath)) {
  throw "No existe config: $ConfigPath"
}

$DbDir = Split-Path -Parent $DatabasePath
if ($DbDir -and -not (Test-Path $DbDir)) {
  New-Item -ItemType Directory -Force -Path $DbDir | Out-Null
}

$ExePath = Join-Path $RootDir "target/release/monitoring-server.exe"
if (-not $SkipBuild) {
  Write-Host "Compilando monitoring-server (release)..."
  Push-Location $RootDir
  try {
    cargo build --release --bin monitoring-server
  } finally {
    Pop-Location
  }
} elseif (-not (Test-Path $ExePath)) {
  Write-Host "Binario release no encontrado, compilando..."
  Push-Location $RootDir
  try {
    cargo build --release --bin monitoring-server
  } finally {
    Pop-Location
  }
}

& $ExePath `
  --config $ConfigPath `
  --database-path $DatabasePath `
  --bind $Bind

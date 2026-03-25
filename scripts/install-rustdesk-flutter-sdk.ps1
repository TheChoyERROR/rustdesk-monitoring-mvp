param(
  [string]$Version = "3.24.5",
  [string]$Channel = "stable",
  [string]$InstallRoot = "",
  [switch]$Execute,
  [switch]$Force
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($InstallRoot)) {
  $InstallRoot = Join-Path $repoRoot "tools"
}

$InstallRoot = [System.IO.Path]::GetFullPath($InstallRoot)
$targetDir = Join-Path $InstallRoot "flutter-$Version"
$downloadUrl = "https://storage.googleapis.com/flutter_infra_release/releases/$Channel/windows/flutter_windows_${Version}-${Channel}.zip"
$targetFlutterBat = Join-Path $targetDir "bin\flutter.bat"

Write-Host "Instalador Flutter para rustdesk-monitoring-mvp"
Write-Host "Version: $Version"
Write-Host "Canal: $Channel"
Write-Host "Destino: $targetDir"
Write-Host "URL: $downloadUrl"
Write-Host "Modo: $(if ($Execute) { 'execute' } else { 'dry-run' })"

if ((Test-Path $targetFlutterBat) -and -not $Force) {
  Write-Host "Flutter ya existe en: $targetDir"
  exit 0
}

if (-not $Execute) {
  exit 0
}

New-Item -ItemType Directory -Force -Path $InstallRoot | Out-Null
if ((Test-Path $targetDir) -and $Force) {
  Remove-Item -Path $targetDir -Recurse -Force
}

$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) "rustdesk-monitoring-mvp-flutter-$Version-$([guid]::NewGuid().ToString('N'))"
$zipPath = Join-Path $tempRoot "flutter_windows_${Version}-${Channel}.zip"
$extractRoot = Join-Path $tempRoot "extract"
$extractedFlutterDir = Join-Path $extractRoot "flutter"

try {
  New-Item -ItemType Directory -Force -Path $tempRoot | Out-Null
  New-Item -ItemType Directory -Force -Path $extractRoot | Out-Null

  Write-Host "Descargando Flutter..."
  Invoke-WebRequest -Uri $downloadUrl -OutFile $zipPath

  Write-Host "Extrayendo Flutter..."
  Expand-Archive -Path $zipPath -DestinationPath $extractRoot -Force

  if (-not (Test-Path $extractedFlutterDir)) {
    throw "No se encontro la carpeta 'flutter' dentro del zip descargado."
  }

  if (Test-Path $targetDir) {
    Remove-Item -Path $targetDir -Recurse -Force
  }

  Move-Item -Path $extractedFlutterDir -Destination $targetDir
  Write-Host "Flutter instalado en: $targetDir"
} finally {
  if (Test-Path $tempRoot) {
    Remove-Item -Path $tempRoot -Recurse -Force
  }
}

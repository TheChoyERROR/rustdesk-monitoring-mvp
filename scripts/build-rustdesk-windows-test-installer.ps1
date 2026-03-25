param(
  [string]$MonitoringUrl = "http://127.0.0.1:8080",
  [string]$CompanyName = "MyCompany",
  [string]$OutputDir = "",
  [switch]$SkipBuild,
  [switch]$SkipApplyForkPatches,
  [switch]$SkipZip,
  [switch]$SkipNsis
)

$ErrorActionPreference = "Stop"

$rootDir = Split-Path -Parent $PSScriptRoot
$forkPath = Join-Path $rootDir "rustdesk-fork"
$builderScript = Join-Path $PSScriptRoot "build-rustdesk-windows-installer.ps1"

if (-not (Test-Path $forkPath)) {
  throw "No se encontro el fork local en: $forkPath"
}

$params = @{
  RustDeskRepoPath = $forkPath
  MonitoringUrl = $MonitoringUrl
  CompanyName = $CompanyName
}

if (-not [string]::IsNullOrWhiteSpace($OutputDir)) {
  $params.OutputDir = $OutputDir
}

if (-not $SkipBuild) {
  $params.BuildRustDesk = $true
}

if ($SkipApplyForkPatches) {
  $params.SkipApplyForkPatches = $true
}

if ($SkipZip) {
  $params.SkipZip = $true
}

if ($SkipNsis) {
  $params.SkipNsis = $true
}

& $builderScript @params

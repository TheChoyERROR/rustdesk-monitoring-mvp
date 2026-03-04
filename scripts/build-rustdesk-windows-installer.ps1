param(
  [string]$RustDeskRepo = "$HOME\Desktop\rustdesk",
  [string]$MonitoringUrl = "",
  [string]$Version = "",
  [string]$OutputDir = "",
  [string]$AppName = "RustDesk Monitoring Corporate",
  [string]$CompanyName = "YourCompany",
  [switch]$BuildRustDesk,
  [switch]$SkipZip,
  [switch]$SkipNsis
)

$ErrorActionPreference = "Stop"

function Resolve-ExistingPath {
  param(
    [Parameter(Mandatory = $true)][string]$Path,
    [Parameter(Mandatory = $true)][string]$Label
  )

  if (-not (Test-Path $Path)) {
    throw "$Label no existe: $Path"
  }

  return (Resolve-Path $Path).Path
}

function Get-CargoVersion {
  param([Parameter(Mandatory = $true)][string]$CargoTomlPath)

  if (-not (Test-Path $CargoTomlPath)) {
    return ""
  }

  $match = Select-String -Path $CargoTomlPath -Pattern '^[\s]*version[\s]*=[\s]*"([^"]+)"' | Select-Object -First 1
  if (-not $match) {
    return ""
  }

  return $match.Matches[0].Groups[1].Value.Trim()
}

function To-NsisPath {
  param([Parameter(Mandatory = $true)][string]$Path)
  return $Path.Replace('\\', '\\\\')
}

if ([string]::IsNullOrWhiteSpace($MonitoringUrl)) {
  throw "Debes especificar -MonitoringUrl, por ejemplo http://192.168.0.103:8080"
}

if ($MonitoringUrl -notmatch '^https?://') {
  throw "MonitoringUrl invalida. Debe iniciar con http:// o https://"
}

$rustDeskRepoPath = Resolve-ExistingPath -Path $RustDeskRepo -Label "RustDesk repo"

if ([string]::IsNullOrWhiteSpace($OutputDir)) {
  $OutputDir = Join-Path (Get-Location) "artifacts/windows-installer"
}

New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
$outputRoot = (Resolve-Path $OutputDir).Path

if ($BuildRustDesk) {
  $pythonCmd = Get-Command python -ErrorAction SilentlyContinue
  if (-not $pythonCmd) {
    $pythonCmd = Get-Command py -ErrorAction SilentlyContinue
  }
  if (-not $pythonCmd) {
    throw "No se encontro python/py en PATH para ejecutar build.py"
  }

  Write-Host "Compilando fork RustDesk (flutter windows release, sin pack portable)..."
  Push-Location $rustDeskRepoPath
  try {
    & $pythonCmd.Source .\build.py --flutter --skip-portable-pack
  } finally {
    Pop-Location
  }
}

$candidateExePaths = @(
  (Join-Path $rustDeskRepoPath "flutter\build\windows\x64\runner\Release\rustdesk.exe"),
  (Join-Path $rustDeskRepoPath "target\release\rustdesk.exe"),
  (Join-Path $rustDeskRepoPath "target\release\RustDesk.exe")
)

$rustDeskExePath = $candidateExePaths | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $rustDeskExePath) {
  throw "No se encontro rustdesk.exe compilado. Ejecuta primero el build del fork o usa -BuildRustDesk."
}

$rustDeskExePath = (Resolve-Path $rustDeskExePath).Path
$sourceDir = Split-Path -Parent $rustDeskExePath
$exeName = Split-Path -Leaf $rustDeskExePath

if ([string]::IsNullOrWhiteSpace($Version)) {
  $Version = Get-CargoVersion -CargoTomlPath (Join-Path $rustDeskRepoPath "Cargo.toml")
}
if ([string]::IsNullOrWhiteSpace($Version)) {
  $Version = Get-Date -Format "yyyy.MM.dd.HHmm"
}

$packageName = "rustdesk-monitoring-corporate-$Version"
$packageRoot = Join-Path $outputRoot $packageName
$stageDir = Join-Path $packageRoot "stage"
$appDir = Join-Path $stageDir "app"

if (Test-Path $packageRoot) {
  Remove-Item -Path $packageRoot -Recurse -Force
}

New-Item -ItemType Directory -Force -Path $appDir | Out-Null
Copy-Item -Path (Join-Path $sourceDir "*") -Destination $appDir -Recurse -Force

$launcherCmdPath = Join-Path $stageDir "launch-rustdesk.cmd"
$launcherPs1Path = Join-Path $stageDir "launch-rustdesk.ps1"
$policyPath = Join-Path $stageDir "MONITORING-POLICY.txt"
$readmePath = Join-Path $stageDir "README-INSTALL.txt"

@"
@echo off
setlocal
set "RUSTDESK_MONITORING_URL=$MonitoringUrl"
start "" "%~dp0app\$exeName" %*
endlocal
"@ | Set-Content -Path $launcherCmdPath -Encoding ASCII

@"
param(
  [Parameter(ValueFromRemainingArguments = `$true)]
  [string[]]`$AppArgs
)

`$env:RUSTDESK_MONITORING_URL = "$MonitoringUrl"
Start-Process -FilePath (Join-Path `$PSScriptRoot "app\\$exeName") -ArgumentList `$AppArgs
"@ | Set-Content -Path $launcherPs1Path -Encoding UTF8

@"
Este equipo usa una version corporativa de RustDesk con registro de eventos operativos
para auditoria de soporte y seguridad.

Servidor de monitoreo configurado:
$MonitoringUrl
"@ | Set-Content -Path $policyPath -Encoding UTF8

@"
$AppName
Version: $Version

Contenido instalado:
- app\\$exeName
- launch-rustdesk.cmd
- launch-rustdesk.ps1
- MONITORING-POLICY.txt

Uso recomendado:
1) Abrir el acceso directo "$AppName" del menu Inicio, o
2) Ejecutar launch-rustdesk.cmd

El launcher configura RUSTDESK_MONITORING_URL en:
$MonitoringUrl
"@ | Set-Content -Path $readmePath -Encoding UTF8

if (-not $SkipZip) {
  $zipPath = Join-Path $packageRoot "$packageName-portable.zip"
  if (Test-Path $zipPath) {
    Remove-Item -Path $zipPath -Force
  }
  Compress-Archive -Path (Join-Path $stageDir "*") -DestinationPath $zipPath -Force
  Write-Host "ZIP corporativo generado: $zipPath"
}

if (-not $SkipNsis) {
  $nsis = Get-Command makensis.exe -ErrorAction SilentlyContinue
  if (-not $nsis) {
    $nsis = Get-Command makensis -ErrorAction SilentlyContinue
  }

  if (-not $nsis) {
    Write-Warning "No se encontro makensis. Se omite setup.exe (NSIS)."
    Write-Warning "Instala NSIS y vuelve a correr para generar instalador."
  } else {
    $setupExeName = "$packageName-setup.exe"
    $setupExePath = Join-Path $packageRoot $setupExeName
    $uninstallKey = "RustDeskMonitoringCorporate"

    $nsiPath = Join-Path $packageRoot "installer.nsi"
    @"
Unicode True
RequestExecutionLevel admin
SetCompressor /SOLID lzma

!define APP_NAME "$AppName"
!define APP_VERSION "$Version"
!define COMPANY_NAME "$CompanyName"
!define EXE_NAME "$exeName"
!define STAGE_DIR "$(To-NsisPath $stageDir)"
!define OUTPUT_EXE "$(To-NsisPath $setupExePath)"
!define UNINSTALL_KEY "$uninstallKey"

Name "`$\{APP_NAME}`"
OutFile "`$\{OUTPUT_EXE}`"
InstallDir "`$PROGRAMFILES64\\RustDeskMonitoringCorporate"
InstallDirRegKey HKLM "Software\\`$\{UNINSTALL_KEY}`" "InstallLocation"

Page directory
Page instfiles
UninstPage uninstConfirm
UninstPage instfiles

Section "Install"
  SetShellVarContext all
  SetOutPath "`$INSTDIR\\app"
  File /r "`$\{STAGE_DIR}\\app\\*.*"

  SetOutPath "`$INSTDIR"
  File "`$\{STAGE_DIR}\\launch-rustdesk.cmd"
  File "`$\{STAGE_DIR}\\launch-rustdesk.ps1"
  File "`$\{STAGE_DIR}\\MONITORING-POLICY.txt"
  File "`$\{STAGE_DIR}\\README-INSTALL.txt"

  CreateDirectory "`$SMPROGRAMS\\`$\{APP_NAME}"
  CreateShortCut "`$SMPROGRAMS\\`$\{APP_NAME}\\`$\{APP_NAME}.lnk" "`$INSTDIR\\launch-rustdesk.cmd"
  CreateShortCut "`$DESKTOP\\`$\{APP_NAME}.lnk" "`$INSTDIR\\launch-rustdesk.cmd"

  WriteUninstaller "`$INSTDIR\\Uninstall.exe"

  WriteRegStr HKLM "Software\\`$\{UNINSTALL_KEY}" "DisplayName" "`$\{APP_NAME}"
  WriteRegStr HKLM "Software\\`$\{UNINSTALL_KEY}" "DisplayVersion" "`$\{APP_VERSION}"
  WriteRegStr HKLM "Software\\`$\{UNINSTALL_KEY}" "Publisher" "`$\{COMPANY_NAME}"
  WriteRegStr HKLM "Software\\`$\{UNINSTALL_KEY}" "InstallLocation" "`$INSTDIR"
  WriteRegStr HKLM "Software\\`$\{UNINSTALL_KEY}" "UninstallString" "`$\"`$INSTDIR\\Uninstall.exe`$\""

  WriteRegStr HKLM "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\`$\{UNINSTALL_KEY}" "DisplayName" "`$\{APP_NAME}"
  WriteRegStr HKLM "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\`$\{UNINSTALL_KEY}" "DisplayVersion" "`$\{APP_VERSION}"
  WriteRegStr HKLM "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\`$\{UNINSTALL_KEY}" "Publisher" "`$\{COMPANY_NAME}"
  WriteRegStr HKLM "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\`$\{UNINSTALL_KEY}" "InstallLocation" "`$INSTDIR"
  WriteRegStr HKLM "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\`$\{UNINSTALL_KEY}" "UninstallString" "`$\"`$INSTDIR\\Uninstall.exe`$\""
SectionEnd

Section "Uninstall"
  SetShellVarContext all
  Delete "`$DESKTOP\\`$\{APP_NAME}.lnk"
  Delete "`$SMPROGRAMS\\`$\{APP_NAME}\\`$\{APP_NAME}.lnk"
  RMDir "`$SMPROGRAMS\\`$\{APP_NAME}"

  Delete "`$INSTDIR\\launch-rustdesk.cmd"
  Delete "`$INSTDIR\\launch-rustdesk.ps1"
  Delete "`$INSTDIR\\MONITORING-POLICY.txt"
  Delete "`$INSTDIR\\README-INSTALL.txt"
  Delete "`$INSTDIR\\Uninstall.exe"
  RMDir /r "`$INSTDIR\\app"
  RMDir "`$INSTDIR"

  DeleteRegKey HKLM "Software\\`$\{UNINSTALL_KEY}"
  DeleteRegKey HKLM "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\`$\{UNINSTALL_KEY}"
SectionEnd
"@ | Set-Content -Path $nsiPath -Encoding ASCII

    & $nsis.Source /V2 $nsiPath
    Write-Host "Instalador NSIS generado: $setupExePath"
  }
}

Write-Host ""
Write-Host "Build de paquete corporativo finalizado."
Write-Host "Carpeta de salida: $packageRoot"
Write-Host "Binario usado: $rustDeskExePath"
Write-Host "Monitoring URL: $MonitoringUrl"

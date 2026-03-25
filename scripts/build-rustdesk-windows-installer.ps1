param(
  [string]$RustDeskRepo = "$HOME\Desktop\rustdesk",
  [string]$RustDeskRepoPath = "",
  [string]$RustDeskExePath = "",
  [string]$MonitoringUrl = "",
  [string]$Version = "",
  [string]$OutputDir = "",
  [string]$AppName = "RustDesk Monitoring Corporate",
  [string]$CompanyName = "YourCompany",
  [string]$InstallDirName = "RustDeskMonitoringCorporate",
  [string]$UninstallKey = "RustDeskMonitoringCorporate",
  [switch]$BuildRustDesk,
  [switch]$SkipApplyForkPatches,
  [switch]$SkipZip,
  [switch]$SkipNsis
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir "rustdesk-flutter.ps1")
$applyForkPatchesScript = Join-Path $scriptDir "apply-rustdesk-fork-patches.ps1"

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

function Get-FileSha256 {
  param([Parameter(Mandatory = $true)][string]$Path)

  return (Get-FileHash -Path $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-FileVersionValue {
  param([Parameter(Mandatory = $true)][string]$ExePath)

  try {
    $versionInfo = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($ExePath)
    if (-not [string]::IsNullOrWhiteSpace($versionInfo.ProductVersion)) {
      return $versionInfo.ProductVersion.Trim()
    }
    if (-not [string]::IsNullOrWhiteSpace($versionInfo.FileVersion)) {
      return $versionInfo.FileVersion.Trim()
    }
  } catch {
  }

  return ""
}

function Get-NsisPath {
  $nsis = Get-Command makensis.exe -ErrorAction SilentlyContinue
  if (-not $nsis) {
    $nsis = Get-Command makensis -ErrorAction SilentlyContinue
  }

  if ($nsis) {
    return $nsis.Source
  }

  foreach ($candidate in @(
      "C:\Program Files (x86)\NSIS\makensis.exe",
      "C:\Program Files\NSIS\makensis.exe"
    )) {
    if (Test-Path $candidate) {
      return $candidate
    }
  }

  return $null
}

if (-not [string]::IsNullOrWhiteSpace($RustDeskRepoPath)) {
  $RustDeskRepo = $RustDeskRepoPath
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

  $repoRoot = Split-Path -Parent $PSScriptRoot
  $flutterRoot = Use-RustDeskFlutter -RepoRoot $repoRoot
  if (-not [string]::IsNullOrWhiteSpace($flutterRoot)) {
    $flutterVersion = Get-RustDeskFlutterVersion -FlutterRoot $flutterRoot
    Write-Host "Usando Flutter: $flutterRoot$(if ($flutterVersion) { " (version $flutterVersion)" })"
  }
  Add-RustDeskGitSafeDirectory -Path $rustDeskRepoPath

  Write-Host "Inicializando submodulos del fork RustDesk..."
  & git -C $rustDeskRepoPath submodule sync --recursive
  if ($LASTEXITCODE -ne 0) {
    throw "Fallo git submodule sync"
  }
  & git -C $rustDeskRepoPath submodule update --init --recursive
  if ($LASTEXITCODE -ne 0) {
    throw "Fallo git submodule update"
  }

  if (-not $SkipApplyForkPatches -and (Test-Path $applyForkPatchesScript)) {
    $gitStatus = & git -C $rustDeskRepoPath status --porcelain
    if ($LASTEXITCODE -ne 0) {
      throw "Fallo git status antes de aplicar patches"
    }
    $hasLocalChanges = -not [string]::IsNullOrWhiteSpace(($gitStatus | Out-String).Trim())

    if ($hasLocalChanges) {
      Write-Warning "rustdesk-fork ya tiene cambios locales; se omite la autoaplicacion de patches. Usa un checkout limpio si quieres reprovisionarlo desde cero."
    } else {
      Write-Host "Aplicando patches versionados del fork RustDesk..."
      & powershell -NoProfile -ExecutionPolicy Bypass -File $applyForkPatchesScript -ForkPath $rustDeskRepoPath -Execute
      if ($LASTEXITCODE -ne 0) {
        throw "Fallo apply-rustdesk-fork-patches.ps1"
      }
    }
  }

  Write-Host "Compilando fork RustDesk (flutter windows release, sin pack portable)..."
  Push-Location $rustDeskRepoPath
  try {
    & $pythonCmd.Source .\build.py --flutter --skip-portable-pack
  } finally {
    Pop-Location
  }
}

$candidateExePaths = @()
if (-not [string]::IsNullOrWhiteSpace($RustDeskExePath)) {
  $candidateExePaths += $RustDeskExePath
}
$candidateExePaths += @(
  (Join-Path $rustDeskRepoPath "flutter\build\windows\x64\runner\Release\rustdesk.exe"),
  (Join-Path $rustDeskRepoPath "flutter\build\windows\x64\runner\Release\RustDesk.exe"),
  (Join-Path $rustDeskRepoPath "target\release\rustdesk.exe"),
  (Join-Path $rustDeskRepoPath "target\release\RustDesk.exe")
)

$rustDeskExePath = $candidateExePaths | Where-Object { -not [string]::IsNullOrWhiteSpace($_) -and (Test-Path $_) } | Select-Object -First 1
if (-not $rustDeskExePath) {
  throw "No se encontro rustdesk.exe compilado. Ejecuta primero el build del fork, usa -BuildRustDesk o especifica -RustDeskExePath."
}

$rustDeskExePath = (Resolve-Path $rustDeskExePath).Path
$sourceDir = Split-Path -Parent $rustDeskExePath
$exeName = Split-Path -Leaf $rustDeskExePath
$exeHash = Get-FileSha256 -Path $rustDeskExePath
$exeVersion = Get-FileVersionValue -ExePath $rustDeskExePath

if ([string]::IsNullOrWhiteSpace($Version)) {
  $Version = Get-CargoVersion -CargoTomlPath (Join-Path $rustDeskRepoPath "Cargo.toml")
}
if ([string]::IsNullOrWhiteSpace($Version)) {
  $Version = $exeVersion
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
$launcherEnvPath = Join-Path $stageDir "monitoring-launcher.env"
$policyPath = Join-Path $stageDir "MONITORING-POLICY.txt"
$readmePath = Join-Path $stageDir "README-INSTALL.txt"
$manifestPath = Join-Path $packageRoot "package-manifest.json"

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
RUSTDESK_MONITORING_URL=$MonitoringUrl
APP_NAME=$AppName
PACKAGE_VERSION=$Version
SOURCE_EXE_SHA256=$exeHash
"@ | Set-Content -Path $launcherEnvPath -Encoding ASCII

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
- monitoring-launcher.env
- MONITORING-POLICY.txt

Uso recomendado:
1) Abrir el acceso directo "$AppName" del menu Inicio, o
2) Ejecutar launch-rustdesk.cmd

El launcher configura RUSTDESK_MONITORING_URL en:
$MonitoringUrl
"@ | Set-Content -Path $readmePath -Encoding UTF8

$manifest = [ordered]@{
  package_name = $packageName
  app_name = $AppName
  company_name = $CompanyName
  package_version = $Version
  monitoring_url = $MonitoringUrl
  generated_at = (Get-Date).ToString("o")
  source_repo_path = $rustDeskRepoPath
  source_exe_path = $rustDeskExePath
  source_exe_name = $exeName
  source_exe_sha256 = $exeHash
  source_exe_file_version = $exeVersion
  install_dir_name = $InstallDirName
  uninstall_key = $UninstallKey
  artifacts = [ordered]@{
    stage_dir = $stageDir
    launcher_cmd = $launcherCmdPath
    launcher_ps1 = $launcherPs1Path
    launcher_env = $launcherEnvPath
    policy = $policyPath
    readme = $readmePath
  }
}
$manifest | ConvertTo-Json -Depth 6 | Set-Content -Path $manifestPath -Encoding UTF8

if (-not $SkipZip) {
  $zipPath = Join-Path $packageRoot "$packageName-portable.zip"
  if (Test-Path $zipPath) {
    Remove-Item -Path $zipPath -Force
  }
  Compress-Archive -Path (Join-Path $stageDir "*") -DestinationPath $zipPath -Force
  Write-Host "ZIP corporativo generado: $zipPath"
}

if (-not $SkipNsis) {
  $nsis = Get-NsisPath

  if (-not $nsis) {
    Write-Warning "No se encontro makensis. Se omite setup.exe (NSIS)."
    Write-Warning "Instala NSIS y vuelve a correr para generar instalador."
  } else {
    $setupExeName = "$packageName-setup.exe"
    $setupExePath = Join-Path $packageRoot $setupExeName

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
!define INSTALL_DIR_NAME "$InstallDirName"
!define UNINSTALL_KEY "$UninstallKey"

Name "`${APP_NAME}"
OutFile "`${OUTPUT_EXE}"
InstallDir "`$PROGRAMFILES64\\`${INSTALL_DIR_NAME}"
InstallDirRegKey HKLM "Software\\`${UNINSTALL_KEY}" "InstallLocation"

Page directory
Page instfiles
UninstPage uninstConfirm
UninstPage instfiles

Function CloseRunningRustDesk
  DetailPrint "Cerrando instancias activas de `${EXE_NAME} si existen..."
  ClearErrors
  ExecWait '`"`$SYSDIR\\taskkill.exe`" /F /T /IM `${EXE_NAME}' `$0
  Sleep 1000
FunctionEnd

Function un.CloseRunningRustDesk
  DetailPrint "Cerrando instancias activas de `${EXE_NAME} si existen..."
  ClearErrors
  ExecWait '`"`$SYSDIR\\taskkill.exe`" /F /T /IM `${EXE_NAME}' `$0
  Sleep 1000
FunctionEnd

Section "Install"
  SetShellVarContext all
  Call CloseRunningRustDesk
  SetOutPath "`$INSTDIR\\app"
  File /r "`${STAGE_DIR}\\app\\*.*"

  SetOutPath "`$INSTDIR"
  File "`${STAGE_DIR}\\launch-rustdesk.cmd"
  File "`${STAGE_DIR}\\launch-rustdesk.ps1"
  File "`${STAGE_DIR}\\monitoring-launcher.env"
  File "`${STAGE_DIR}\\MONITORING-POLICY.txt"
  File "`${STAGE_DIR}\\README-INSTALL.txt"

  CreateDirectory "`$SMPROGRAMS\\`${APP_NAME}"
  CreateShortCut "`$SMPROGRAMS\\`${APP_NAME}\\`${APP_NAME}.lnk" "`$INSTDIR\\launch-rustdesk.cmd"
  CreateShortCut "`$DESKTOP\\`${APP_NAME}.lnk" "`$INSTDIR\\launch-rustdesk.cmd"

  WriteUninstaller "`$INSTDIR\\Uninstall.exe"

  WriteRegStr HKLM "Software\\`${UNINSTALL_KEY}" "DisplayName" "`${APP_NAME}"
  WriteRegStr HKLM "Software\\`${UNINSTALL_KEY}" "DisplayVersion" "`${APP_VERSION}"
  WriteRegStr HKLM "Software\\`${UNINSTALL_KEY}" "Publisher" "`${COMPANY_NAME}"
  WriteRegStr HKLM "Software\\`${UNINSTALL_KEY}" "InstallLocation" "`$INSTDIR"
  WriteRegStr HKLM "Software\\`${UNINSTALL_KEY}" "UninstallString" "`$\"`$INSTDIR\\Uninstall.exe`$\""

  WriteRegStr HKLM "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\`${UNINSTALL_KEY}" "DisplayName" "`${APP_NAME}"
  WriteRegStr HKLM "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\`${UNINSTALL_KEY}" "DisplayVersion" "`${APP_VERSION}"
  WriteRegStr HKLM "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\`${UNINSTALL_KEY}" "Publisher" "`${COMPANY_NAME}"
  WriteRegStr HKLM "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\`${UNINSTALL_KEY}" "InstallLocation" "`$INSTDIR"
  WriteRegStr HKLM "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\`${UNINSTALL_KEY}" "UninstallString" "`$\"`$INSTDIR\\Uninstall.exe`$\""
SectionEnd

Section "Uninstall"
  SetShellVarContext all
  Call un.CloseRunningRustDesk
  Delete "`$DESKTOP\\`${APP_NAME}.lnk"
  Delete "`$SMPROGRAMS\\`${APP_NAME}\\`${APP_NAME}.lnk"
  RMDir "`$SMPROGRAMS\\`${APP_NAME}"

  Delete "`$INSTDIR\\launch-rustdesk.cmd"
  Delete "`$INSTDIR\\launch-rustdesk.ps1"
  Delete "`$INSTDIR\\monitoring-launcher.env"
  Delete "`$INSTDIR\\MONITORING-POLICY.txt"
  Delete "`$INSTDIR\\README-INSTALL.txt"
  Delete "`$INSTDIR\\Uninstall.exe"
  RMDir /r "`$INSTDIR\\app"
  RMDir "`$INSTDIR"

  DeleteRegKey HKLM "Software\\`${UNINSTALL_KEY}"
  DeleteRegKey HKLM "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\`${UNINSTALL_KEY}"
SectionEnd
"@ | Set-Content -Path $nsiPath -Encoding ASCII

    & $nsis /V2 $nsiPath
    if ($LASTEXITCODE -ne 0) {
      throw "Fallo makensis al generar setup.exe"
    }
    Write-Host "Instalador NSIS generado: $setupExePath"
  }
}

Write-Host ""
Write-Host "Build de paquete corporativo finalizado."
Write-Host "Carpeta de salida: $packageRoot"
Write-Host "Binario usado: $rustDeskExePath"
Write-Host "SHA256 binario: $exeHash"
Write-Host "Monitoring URL: $MonitoringUrl"

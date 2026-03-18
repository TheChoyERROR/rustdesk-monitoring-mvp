param(
  [switch]$InstallAll,
  [switch]$InstallBuildTools,
  [switch]$InstallFlutter,
  [switch]$InstallCMake,
  [switch]$InstallNsis,
  [switch]$InstallRustup,
  [switch]$BootstrapVcpkg,
  [switch]$InstallVcpkgPackages,
  [string]$VcpkgRoot = "C:\vcpkg",
  [switch]$Execute
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir "rustdesk-flutter.ps1")

function Test-Admin {
  $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
  $principal = New-Object Security.Principal.WindowsPrincipal($identity)
  return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Get-ToolPath {
  param([Parameter(Mandatory = $true)][string]$Name)
  $cmd = Get-Command $Name -ErrorAction SilentlyContinue
  if ($cmd) {
    return $cmd.Source
  }
  return $null
}

function Invoke-Step {
  param(
    [Parameter(Mandatory = $true)][string]$Description,
    [Parameter(Mandatory = $true)][string]$Command,
    [switch]$RequiresAdmin,
    [switch]$ContinueOnError
  )

  Write-Host ""
  Write-Host "==> $Description"
  Write-Host $Command

  if (-not $Execute) {
    return
  }

  if ($RequiresAdmin -and -not (Test-Admin)) {
    throw "Este paso requiere PowerShell ejecutado como administrador: $Description"
  }

  & powershell -NoProfile -ExecutionPolicy Bypass -Command $Command
  if ($LASTEXITCODE -ne 0) {
    if ($ContinueOnError) {
      Write-Warning "Fallo el paso pero se continua: $Description"
      return
    }
    throw "Fallo el paso: $Description"
  }
}

function Ensure-Winget {
  $winget = Get-ToolPath -Name "winget"
  if (-not $winget) {
    throw "No se encontro winget en PATH."
  }
  return $winget
}

if (-not ($InstallAll -or $InstallBuildTools -or $InstallFlutter -or $InstallCMake -or $InstallNsis -or $InstallRustup -or $BootstrapVcpkg -or $InstallVcpkgPackages)) {
  $InstallAll = $true
}

if ($InstallAll) {
  $InstallBuildTools = $true
  $InstallFlutter = $true
  $InstallCMake = $true
  $InstallNsis = $true
  $InstallRustup = $true
  $BootstrapVcpkg = $true
  $InstallVcpkgPackages = $true
}

$vsConfigPath = Join-Path $scriptDir "rustdesk-buildtools.vsconfig"
$repoRoot = Split-Path -Parent $scriptDir
$vendoredFlutterRoot = Get-RustDeskFlutterRoot -RepoRoot $repoRoot
$vendoredFlutterVersion = Get-RustDeskFlutterVersion -FlutterRoot $vendoredFlutterRoot
$winget = Ensure-Winget

Write-Host "Modo: $(if ($Execute) { 'execute' } else { 'dry-run' })"
Write-Host "Winget: $winget"
Write-Host "VS config: $vsConfigPath"
Write-Host "Vcpkg root: $VcpkgRoot"

if ($InstallBuildTools) {
  if (-not (Test-Path $vsConfigPath)) {
    throw "No se encontro rustdesk-buildtools.vsconfig en $vsConfigPath"
  }

  $cmd = "winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override '--wait --passive --config ""$vsConfigPath""' --accept-package-agreements --accept-source-agreements"
  Invoke-Step -Description "Instalar Visual Studio 2022 Build Tools con MSVC/CMake/Windows SDK" -Command $cmd -RequiresAdmin
}

if ($InstallFlutter) {
  if (-not [string]::IsNullOrWhiteSpace($vendoredFlutterRoot) -and (Test-Path (Join-Path $vendoredFlutterRoot "bin\flutter.bat"))) {
    $flutterBat = Join-Path $vendoredFlutterRoot "bin\flutter.bat"
    Write-Host "Flutter recomendado para este repo: $vendoredFlutterRoot$(if ($vendoredFlutterVersion) { " (version $vendoredFlutterVersion)" })"
    $cmd = "& `"$flutterBat`" --suppress-analytics precache --windows"
    Invoke-Step -Description "Preparar Flutter vendorizado para Windows" -Command $cmd
  } else {
    Write-Warning "No se encontro el SDK vendorizado esperado en tools\\flutter-3.24.5."
    Write-Warning "El fallback con winget puede instalar una version mas nueva que no coincida con este repo."
    $cmd = "winget install --id Flutter.Flutter -e --accept-package-agreements --accept-source-agreements"
    Invoke-Step -Description "Instalar Flutter global (fallback; revisar version)" -Command $cmd -RequiresAdmin -ContinueOnError
    Write-Host "Si necesitas una version exacta, coloca el SDK correcto en tools\\flutter-3.24.5."
  }
}

if ($InstallCMake) {
  $cmd = "winget install --id Kitware.CMake -e --accept-package-agreements --accept-source-agreements"
  Invoke-Step -Description "Instalar CMake" -Command $cmd -RequiresAdmin
}

if ($InstallNsis) {
  $cmd = "winget install --id NSIS.NSIS -e --accept-package-agreements --accept-source-agreements"
  Invoke-Step -Description "Instalar NSIS (makensis)" -Command $cmd -RequiresAdmin
}

if ($InstallRustup) {
  $cmd = "winget install --id Rustlang.Rustup -e --accept-package-agreements --accept-source-agreements"
  Invoke-Step -Description "Instalar Rustup" -Command $cmd -RequiresAdmin -ContinueOnError
}

if ($BootstrapVcpkg) {
  $git = Get-ToolPath -Name "git"
  if (-not $git) {
    throw "No se encontro git en PATH para bootstrap de vcpkg."
  }

  $cloneCmd = if (Test-Path $VcpkgRoot) {
    "Write-Host 'vcpkg ya existe en $VcpkgRoot'"
  } else {
    "git clone https://github.com/microsoft/vcpkg `"$VcpkgRoot`""
  }
  Invoke-Step -Description "Clonar vcpkg" -Command $cloneCmd

  $bootstrapBat = Join-Path $VcpkgRoot "bootstrap-vcpkg.bat"
  $bootstrapCmd = "& `"$bootstrapBat`""
  Invoke-Step -Description "Bootstrap de vcpkg" -Command $bootstrapCmd

  $envCmd = "[Environment]::SetEnvironmentVariable('VCPKG_ROOT', '$VcpkgRoot', 'User')"
  Invoke-Step -Description "Configurar variable de entorno VCPKG_ROOT" -Command $envCmd
}

if ($InstallVcpkgPackages) {
  $vcpkgExe = Join-Path $VcpkgRoot "vcpkg.exe"
  $cmd = "& `"$vcpkgExe`" install libvpx:x64-windows-static libyuv:x64-windows-static opus:x64-windows-static aom:x64-windows-static"
  Invoke-Step -Description "Instalar dependencias nativas con vcpkg" -Command $cmd
}

Write-Host ""
Write-Host "Siguiente paso recomendado:"
Write-Host "powershell -ExecutionPolicy Bypass -File .\scripts\check-rustdesk-windows-build-env.ps1"

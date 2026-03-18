param(
  [string]$RustDeskRepoPath = "",
  [switch]$AsJson
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir "rustdesk-flutter.ps1")

function Find-CommandPath {
  param([Parameter(Mandatory = $true)][string]$Name)

  $cmd = Get-Command $Name -ErrorAction SilentlyContinue
  if ($cmd) {
    return $cmd.Source
  }
  return $null
}

function Test-VisualStudioBuildTools {
  $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
  if (-not (Test-Path $vswhere)) {
    return $null
  }

  try {
    $installationPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($installationPath)) {
      return $installationPath.Trim()
    }
  } catch {
  }

  return $null
}

function Test-VcpkgPackage {
  param(
    [Parameter(Mandatory = $true)][string]$VcpkgRoot,
    [Parameter(Mandatory = $true)][string]$PackageName
  )

  $installedDir = Join-Path $VcpkgRoot "installed"
  if (-not (Test-Path $installedDir)) {
    return $false
  }

  $match = Get-ChildItem -Path $installedDir -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -like "*windows*" } |
    ForEach-Object {
      Get-ChildItem -Path $_.FullName -Recurse -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match [regex]::Escape($PackageName) }
    } |
    Select-Object -First 1

  return $null -ne $match
}

function Add-Result {
  param(
    [Parameter(Mandatory = $true)]$List,
    [Parameter(Mandatory = $true)][string]$Name,
    [Parameter(Mandatory = $true)][bool]$Ok,
    [string]$Details = "",
    [string]$Recommendation = ""
  )

  $List.Add([pscustomobject]@{
      name = $Name
      ok = $Ok
      details = $Details
      recommendation = $Recommendation
    })
}

$repoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($RustDeskRepoPath)) {
  $RustDeskRepoPath = Join-Path $repoRoot "rustdesk-fork"
}

$results = New-Object 'System.Collections.Generic.List[object]'
$repoExists = Test-Path $RustDeskRepoPath
Add-Result -List $results -Name "rustdesk_fork_path" -Ok:$repoExists -Details $RustDeskRepoPath -Recommendation "Asegura que el fork local exista en esa ruta."

$pythonPath = Find-CommandPath -Name "python"
Add-Result -List $results -Name "python" -Ok:($null -ne $pythonPath) -Details $pythonPath -Recommendation "Instala Python 3.10+ y dejalo en PATH."

$cargoPath = Find-CommandPath -Name "cargo"
Add-Result -List $results -Name "cargo" -Ok:($null -ne $cargoPath) -Details $cargoPath -Recommendation "Instala Rust con toolchain MSVC."

$rustcPath = Find-CommandPath -Name "rustc"
Add-Result -List $results -Name "rustc" -Ok:($null -ne $rustcPath) -Details $rustcPath -Recommendation "Instala Rust con toolchain MSVC."

$flutterRoot = Get-RustDeskFlutterRoot -RepoRoot $repoRoot
$flutterPath = Get-RustDeskFlutterExecutable -RepoRoot $repoRoot
$flutterVersion = Get-RustDeskFlutterVersion -FlutterRoot $flutterRoot
$flutterDetails = $flutterPath
if (-not [string]::IsNullOrWhiteSpace($flutterVersion)) {
  $flutterDetails = "$flutterPath (version $flutterVersion)"
}
Add-Result -List $results -Name "flutter_sdk" -Ok:($null -ne $flutterPath) -Details $flutterDetails -Recommendation "Usa el SDK vendorizado en tools\\flutter-3.24.5 o define RUSTDESK_FLUTTER_ROOT."

$pathFlutterPath = Find-CommandPath -Name "flutter"
if ($pathFlutterPath) {
  $pathFlutterRoot = Split-Path -Parent (Split-Path -Parent $pathFlutterPath)
  $pathFlutterVersion = Get-RustDeskFlutterVersion -FlutterRoot $pathFlutterRoot
  $pathFlutterDetails = $pathFlutterPath
  if (-not [string]::IsNullOrWhiteSpace($pathFlutterVersion)) {
    $pathFlutterDetails = "$pathFlutterPath (version $pathFlutterVersion)"
  }
  Add-Result -List $results -Name "flutter_path_command" -Ok:$true -Details $pathFlutterDetails -Recommendation "Si no coincide con el SDK vendorizado, los scripts del repo ya priorizan el SDK correcto."
}

$cmakePath = Find-CommandPath -Name "cmake"
Add-Result -List $results -Name "cmake" -Ok:($null -ne $cmakePath) -Details $cmakePath -Recommendation "Instala CMake o habilitalo desde Visual Studio Build Tools."

$nsisPath = Find-CommandPath -Name "makensis"
if (-not $nsisPath) {
  $nsisPath = Find-CommandPath -Name "makensis.exe"
}
Add-Result -List $results -Name "nsis_makensis" -Ok:($null -ne $nsisPath) -Details $nsisPath -Recommendation "Instala NSIS para poder generar setup.exe."

$vsBuildTools = Test-VisualStudioBuildTools
Add-Result -List $results -Name "visual_studio_build_tools" -Ok:($null -ne $vsBuildTools) -Details $vsBuildTools -Recommendation "Instala Visual Studio 2022 Build Tools con MSVC x64/x86, Windows SDK y CMake tools."

$vcpkgRoot = $env:VCPKG_ROOT
$hasVcpkgRoot = -not [string]::IsNullOrWhiteSpace($vcpkgRoot) -and (Test-Path $vcpkgRoot)
Add-Result -List $results -Name "vcpkg_root" -Ok:$hasVcpkgRoot -Details $vcpkgRoot -Recommendation "Configura VCPKG_ROOT y bootstrap vcpkg."

if ($hasVcpkgRoot) {
  foreach ($pkg in @("libvpx", "libyuv", "opus", "aom")) {
    $ok = Test-VcpkgPackage -VcpkgRoot $vcpkgRoot -PackageName $pkg
    Add-Result -List $results -Name "vcpkg_$pkg" -Ok:$ok -Details $vcpkgRoot -Recommendation "Instala $pkg:x64-windows-static con vcpkg."
  }
}

if ($repoExists) {
  $cargoToml = Join-Path $RustDeskRepoPath "Cargo.toml"
  Add-Result -List $results -Name "fork_cargo_toml" -Ok:(Test-Path $cargoToml) -Details $cargoToml -Recommendation "Verifica que el fork sea un checkout valido."

  $monitoringFile = Join-Path $RustDeskRepoPath "src\monitoring_event.rs"
  Add-Result -List $results -Name "fork_monitoring_module" -Ok:(Test-Path $monitoringFile) -Details $monitoringFile -Recommendation "Confirma que el fork local contiene tus cambios de monitoreo/avatar."

  $candidateExePaths = @(
    (Join-Path $RustDeskRepoPath "flutter\build\windows\x64\runner\Release\rustdesk.exe"),
    (Join-Path $RustDeskRepoPath "flutter\build\windows\x64\runner\Release\RustDesk.exe"),
    (Join-Path $RustDeskRepoPath "target\release\rustdesk.exe"),
    (Join-Path $RustDeskRepoPath "target\release\RustDesk.exe")
  )
  $existingExe = $candidateExePaths | Where-Object { Test-Path $_ } | Select-Object -First 1
  Add-Result -List $results -Name "built_rustdesk_exe" -Ok:($null -ne $existingExe) -Details $existingExe -Recommendation "Compila el fork o pasa -RustDeskExePath al empaquetador."
}

$allOk = ($results | Where-Object { -not $_.ok }).Count -eq 0

if ($AsJson) {
  [pscustomobject]@{
    rustdesk_repo_path = $RustDeskRepoPath
    all_ok = $allOk
    checks = $results
  } | ConvertTo-Json -Depth 5
  exit 0
}

Write-Host ""
Write-Host "Diagnostico de entorno para build/installer de RustDesk"
Write-Host "Repo: $RustDeskRepoPath"
Write-Host ""

foreach ($item in $results) {
  $status = if ($item.ok) { "OK " } else { "MISS" }
  Write-Host "[$status] $($item.name)"
  if (-not [string]::IsNullOrWhiteSpace($item.details)) {
    Write-Host "  Detalle: $($item.details)"
  }
  if (-not $item.ok -and -not [string]::IsNullOrWhiteSpace($item.recommendation)) {
    Write-Host "  Siguiente paso: $($item.recommendation)"
  }
}

Write-Host ""
if ($allOk) {
  Write-Host "Estado final: entorno listo para compilar y empaquetar."
  exit 0
}

Write-Host "Estado final: faltan dependencias antes de compilar o crear el instalador."
exit 1

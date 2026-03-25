function Get-RustDeskFlutterRoot {
  param([string]$RepoRoot = "")

  if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    $RepoRoot = Split-Path -Parent $PSScriptRoot
  }

  $candidateRoots = New-Object 'System.Collections.Generic.List[string]'

  foreach ($candidate in @(
      $env:RUSTDESK_FLUTTER_ROOT,
      $env:FLUTTER_ROOT,
      (Join-Path $RepoRoot "tools\flutter-3.24.5"),
      (Join-Path $RepoRoot "tools\flutter")
    )) {
    if (-not [string]::IsNullOrWhiteSpace($candidate)) {
      $candidateRoots.Add($candidate)
    }
  }

  foreach ($root in ($candidateRoots | Select-Object -Unique)) {
    $flutterBat = Join-Path $root "bin\flutter.bat"
    if (Test-Path $flutterBat) {
      return (Resolve-Path $root).Path
    }
  }

  $flutterCmd = Get-Command flutter -ErrorAction SilentlyContinue
  if ($flutterCmd) {
    return Split-Path -Parent (Split-Path -Parent $flutterCmd.Source)
  }

  return $null
}

function Get-RustDeskFlutterExecutable {
  param([string]$RepoRoot = "")

  $flutterRoot = Get-RustDeskFlutterRoot -RepoRoot $RepoRoot
  if (-not [string]::IsNullOrWhiteSpace($flutterRoot)) {
    $flutterBat = Join-Path $flutterRoot "bin\flutter.bat"
    if (Test-Path $flutterBat) {
      return $flutterBat
    }
  }

  $flutterCmd = Get-Command flutter -ErrorAction SilentlyContinue
  if ($flutterCmd) {
    return $flutterCmd.Source
  }

  return $null
}

function Get-RustDeskFlutterVersion {
  param([string]$FlutterRoot)

  if ([string]::IsNullOrWhiteSpace($FlutterRoot)) {
    return $null
  }

  $versionJson = Join-Path $FlutterRoot "bin\cache\flutter.version.json"
  if (Test-Path $versionJson) {
    try {
      $json = Get-Content $versionJson -Raw | ConvertFrom-Json
      if (-not [string]::IsNullOrWhiteSpace($json.flutterVersion)) {
        return $json.flutterVersion
      }
      if (-not [string]::IsNullOrWhiteSpace($json.frameworkVersion)) {
        return $json.frameworkVersion
      }
    } catch {
    }
  }

  $versionFile = Join-Path $FlutterRoot "version"
  if (Test-Path $versionFile) {
    $version = (Get-Content $versionFile -TotalCount 1).Trim()
    if (-not [string]::IsNullOrWhiteSpace($version)) {
      return $version
    }
  }

  return $null
}

function Get-RustDeskGitCommandDir {
  param([string]$RepoRoot = "")

  if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    $RepoRoot = Split-Path -Parent $PSScriptRoot
  }

  foreach ($candidate in @(
      (Join-Path $RepoRoot "tools\flutter-3.24.5\bin\mingit\cmd"),
      (Join-Path $RepoRoot "tools\flutter\bin\mingit\cmd"),
      "C:\Program Files\Git\cmd"
    )) {
    if (-not [string]::IsNullOrWhiteSpace($candidate) -and (Test-Path (Join-Path $candidate "git.exe"))) {
      return $candidate
    }
  }

  $gitCmd = Get-Command git -ErrorAction SilentlyContinue
  if ($gitCmd) {
    return Split-Path -Parent $gitCmd.Source
  }

  return $null
}

function Get-RustDeskWindowsShimDir {
  param([string]$RepoRoot = "")

  if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    $RepoRoot = Split-Path -Parent $PSScriptRoot
  }

  $shimDir = Join-Path $RepoRoot "scripts\windows-shims"
  if (Test-Path (Join-Path $shimDir "ver.bat")) {
    return (Resolve-Path $shimDir).Path
  }

  return $null
}

function Add-RustDeskGitSafeDirectory {
  param([Parameter(Mandatory = $true)][string]$Path)

  if ([string]::IsNullOrWhiteSpace($Path)) {
    return
  }

  $normalizedPath = $Path.Replace('\', '/')
  $currentCount = 0
  if (-not [string]::IsNullOrWhiteSpace($env:GIT_CONFIG_COUNT)) {
    [void][int]::TryParse($env:GIT_CONFIG_COUNT, [ref]$currentCount)
  }

  for ($i = 0; $i -lt $currentCount; $i++) {
    $existingKey = [Environment]::GetEnvironmentVariable("GIT_CONFIG_KEY_$i")
    $existingValue = [Environment]::GetEnvironmentVariable("GIT_CONFIG_VALUE_$i")
    if ($existingKey -eq "safe.directory" -and $existingValue -eq $normalizedPath) {
      return
    }
  }

  [Environment]::SetEnvironmentVariable("GIT_CONFIG_KEY_$currentCount", "safe.directory")
  [Environment]::SetEnvironmentVariable("GIT_CONFIG_VALUE_$currentCount", $normalizedPath)
  [Environment]::SetEnvironmentVariable("GIT_CONFIG_COUNT", ($currentCount + 1).ToString())
}

function Use-RustDeskFlutter {
  param([string]$RepoRoot = "")

  $flutterRoot = Get-RustDeskFlutterRoot -RepoRoot $RepoRoot
  if ([string]::IsNullOrWhiteSpace($flutterRoot)) {
    return $null
  }

  $flutterBin = Join-Path $flutterRoot "bin"
  if (-not (Test-Path (Join-Path $flutterBin "flutter.bat"))) {
    return $null
  }

  $env:RUSTDESK_FLUTTER_ROOT = $flutterRoot
  $env:FLUTTER_ROOT = $flutterRoot
  $env:FLUTTER_SUPPRESS_ANALYTICS = "true"
  $env:CI = "true"
  if (Test-Path "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools") {
    $env:RUSTDESK_VS_INSTALL_PATH = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools"
  }
  $localCmakeShim = Join-Path $RepoRoot "scripts\windows-shims\cmake.bat"
  if (Test-Path $localCmakeShim) {
    $env:RUSTDESK_CMAKE_PATH = (Resolve-Path $localCmakeShim).Path
  } elseif (Test-Path "C:\Program Files\CMake\bin\cmake.exe") {
    $env:RUSTDESK_CMAKE_PATH = "C:\Program Files\CMake\bin\cmake.exe"
  }

  $pathEntries = $env:Path -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
  $shimDir = Get-RustDeskWindowsShimDir -RepoRoot $RepoRoot
  if (-not [string]::IsNullOrWhiteSpace($shimDir) -and $pathEntries -notcontains $shimDir) {
    $env:Path = "$shimDir;$env:Path"
    $pathEntries = $env:Path -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
  }
  if ($pathEntries -notcontains $flutterBin) {
    $env:Path = "$flutterBin;$env:Path"
    $pathEntries = $env:Path -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
  }

  $gitCmdDir = Get-RustDeskGitCommandDir -RepoRoot $RepoRoot
  if (-not [string]::IsNullOrWhiteSpace($gitCmdDir) -and $pathEntries -notcontains $gitCmdDir) {
    $env:Path = "$gitCmdDir;$env:Path"
  }

  Add-RustDeskGitSafeDirectory -Path $flutterRoot

  return $flutterRoot
}

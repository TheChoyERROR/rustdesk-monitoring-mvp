param(
  [string]$ForkPath = "",
  [string]$PatchDir = "",
  [switch]$Execute,
  [switch]$Force
)

$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "rustdesk-flutter.ps1")

$repoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($ForkPath)) {
  $ForkPath = Join-Path $repoRoot "rustdesk-fork"
}
if ([string]::IsNullOrWhiteSpace($PatchDir)) {
  $PatchDir = Join-Path $repoRoot "patches\rustdesk-fork"
}

$ForkPath = (Resolve-Path $ForkPath).Path
$PatchDir = (Resolve-Path $PatchDir).Path
$manifestPath = Join-Path $PatchDir "manifest.json"

if (-not (Test-Path (Join-Path $ForkPath ".git"))) {
  throw "No se encontro un repo git valido en: $ForkPath"
}
if (-not (Test-Path $manifestPath)) {
  throw "No se encontro el manifest de patches en: $manifestPath"
}

Add-RustDeskGitSafeDirectory -Path $ForkPath

$manifest = Get-Content $manifestPath -Raw | ConvertFrom-Json
$patchNames = @($manifest.patches)
if ($patchNames.Count -eq 0) {
  throw "El manifest no contiene patches para aplicar."
}

function Invoke-Git {
  param(
    [Parameter(Mandatory = $true)][string[]]$Arguments,
    [switch]$IgnoreExitCode
  )

  & git @Arguments
  $exitCode = $LASTEXITCODE
  if (-not $IgnoreExitCode -and $exitCode -ne 0) {
    throw "Fallo git $($Arguments -join ' ')"
  }
  return $exitCode
}

function Test-GitApply {
  param(
    [Parameter(Mandatory = $true)][string]$RepoPath,
    [Parameter(Mandatory = $true)][string]$PatchPath,
    [switch]$Reverse
  )

  $args = @("-C", $RepoPath, "apply", "--check", "--whitespace=nowarn")
  if ($Reverse) {
    $args += "-R"
  }
  $args += $PatchPath

  $previousErrorActionPreference = $ErrorActionPreference
  $nativePreferenceVar = Get-Variable -Name PSNativeCommandUseErrorActionPreference -ErrorAction SilentlyContinue
  $previousNativePreference = $null
  if ($nativePreferenceVar) {
    $previousNativePreference = $nativePreferenceVar.Value
    $script:PSNativeCommandUseErrorActionPreference = $false
  }

  try {
    $ErrorActionPreference = "Continue"
    & git @args 1>$null 2>$null
    return $LASTEXITCODE -eq 0
  } finally {
    $ErrorActionPreference = $previousErrorActionPreference
    if ($nativePreferenceVar) {
      $script:PSNativeCommandUseErrorActionPreference = $previousNativePreference
    }
  }
}

$statusOutput = & git -C $ForkPath status --porcelain
if ($LASTEXITCODE -ne 0) {
  throw "No se pudo leer el estado de git en $ForkPath"
}
$hasLocalChanges = -not [string]::IsNullOrWhiteSpace(($statusOutput | Out-String).Trim())

$currentCommit = (& git -C $ForkPath rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) {
  throw "No se pudo leer el commit actual del fork."
}

Write-Host "Aplicador de patches rustdesk-fork"
Write-Host "Fork path: $ForkPath"
Write-Host "Fork commit actual: $currentCommit"
Write-Host "Commit base esperado: $($manifest.fork_base_commit)"
Write-Host "Modo: $(if ($Execute) { 'execute' } else { 'dry-run' })"

if ($hasLocalChanges -and -not $Force) {
  Write-Warning "El fork tiene cambios locales. Se intentara continuar y solo se abortara si el patch no aplica ni coincide con el estado actual."
}

if ($currentCommit -ne $manifest.fork_base_commit) {
  Write-Warning "El fork no esta exactamente en el commit base esperado. Se intentara aplicar igual."
}

foreach ($patchName in $patchNames) {
  $patchPath = Join-Path $PatchDir $patchName
  if (-not (Test-Path $patchPath)) {
    throw "No existe el patch: $patchPath"
  }

  if (Test-GitApply -RepoPath $ForkPath -PatchPath $patchPath) {
    Write-Host "Patch pendiente: $patchName"
    if ($Execute) {
      Invoke-Git -Arguments @("-C", $ForkPath, "apply", "--whitespace=nowarn", $patchPath)
      Write-Host "Aplicado: $patchName"
    }
    continue
  }

  if (Test-GitApply -RepoPath $ForkPath -PatchPath $patchPath -Reverse) {
    Write-Host "Ya aplicado: $patchName"
    continue
  }

  throw "No se pudo aplicar ni reconocer como ya aplicado el patch: $patchName"
}

Write-Host ""
Write-Host "Patches del fork listos."

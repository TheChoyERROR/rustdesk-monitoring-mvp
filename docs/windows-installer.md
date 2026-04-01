# Windows installer for RustDesk corporate fork

This guide generates a corporate Windows package for your RustDesk fork with
`RUSTDESK_MONITORING_URL` preconfigured through a launcher.

It produces:
- Portable ZIP package (always, unless `-SkipZip`)
- NSIS `setup.exe` installer (if `makensis` is installed and `-SkipNsis` is not used)
- Build metadata manifest (`package-manifest.json`)

## Prerequisites

1. Windows machine with:
- PowerShell
- RustDesk fork cloned locally
- RustDesk already built (`rustdesk.exe` exists)

2. Optional for installer EXE:
- NSIS (`makensis`) in `PATH`

3. This monitoring repo cloned locally.

## Script location

- [build-rustdesk-windows-installer.ps1](/home/choy/Escritorio/Reto/scripts/build-rustdesk-windows-installer.ps1)

## Quick command

From this repo root in PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\build-rustdesk-windows-installer.ps1 `
  -RustDeskRepoPath "C:\Users\Choy\Desktop\rustdesk-monitoring-mvp\rustdesk-fork" `
  -MonitoringUrl "http://192.168.0.103:8080" `
  -CompanyName "MyCompany"
```

## First test installer

For a first packaging test on Windows:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\build-rustdesk-windows-test-installer.ps1 `
  -MonitoringUrl "http://127.0.0.1:8080" `
  -CompanyName "MyCompany"
```

This wrapper assumes the fork is in `.\rustdesk-fork` and enables `-BuildRustDesk` by default.

## What the script does

1. Detects `rustdesk.exe` from these paths (first match):
- `flutter\build\windows\x64\runner\Release\rustdesk.exe`
- `flutter\build\windows\x64\runner\Release\RustDesk.exe`
- `target\release\rustdesk.exe`
- `target\release\RustDesk.exe`

You can also bypass autodetection with:
- `-RustDeskExePath "C:\path\to\rustdesk.exe"`

2. Copies the full build directory to a staging folder.

3. Creates launchers:
- `launch-rustdesk.cmd`
- `launch-rustdesk.ps1`

Both launchers set:
- `RUSTDESK_MONITORING_URL=<your-url>`

4. Creates `MONITORING-POLICY.txt` and install notes.
5. Creates `monitoring-launcher.env` with the monitoring URL and package metadata.
6. Creates `package-manifest.json` with source path, version, SHA256 and artifact paths.

5. Outputs artifacts in:
- `artifacts\windows-installer\rustdesk-monitoring-corporate-<version>\`

## Optional flags

- `-BuildRustDesk`: runs `python build.py --flutter --skip-portable-pack` in the fork before packaging.
- `-RustDeskRepoPath`: alias of `-RustDeskRepo` for clearer usage in docs/automation.
- `-RustDeskExePath`: explicit path to the built executable to package.
- `-SkipZip`: do not create portable ZIP.
- `-SkipNsis`: do not try to create NSIS installer EXE.
- `-Version "1.4.6-company.1"`: set package version explicitly.
- `-OutputDir "C:\temp\artifacts"`: custom output root.
- `-InstallDirName "RustDeskMonitoringCorporate"`: customize install folder under `Program Files`.
- `-UninstallKey "RustDeskMonitoringCorporate"`: customize registry uninstall key.

## Expected outputs

Inside package folder:
- `*-portable.zip`
- `*-setup.exe` (if NSIS is available)
- `package-manifest.json`
- `stage\` (staging files used to build installer)

## Validate after install

1. Install `*-setup.exe` or unzip portable package.
2. Start app using shortcut or `launch-rustdesk.cmd`.
3. Open monitoring dashboard and verify new events appear.
4. Review `package-manifest.json` and keep it with the release artifact for traceability.
4. Check backend:

```bash
curl -s http://127.0.0.1:8080/metrics
curl -s http://127.0.0.1:8080/api/v1/sessions/presence
```

## Notes

- Installer currently configures monitoring URL through launcher env var.
- The package manifest records the exact source executable SHA256 used for the build.
- Next step can be code signing (`signtool`) and MSI pipeline if needed.

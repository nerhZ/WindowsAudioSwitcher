# Regenerates the full icon set from src-tauri/icons/icon-source.png, then
# prunes the outputs the bundle config does not reference.
$ErrorActionPreference = "Stop"

$source = "src-tauri\icons\icon-source.png"
if (-not (Test-Path $source)) {
    Write-Error "No source icon at $source - download it from Flaticon (1024x1024 if available) and re-run."
}

bun run tauri icon $source

Remove-Item src-tauri\icons\Square*.png, src-tauri\icons\StoreLogo.png, `
    src-tauri\icons\icon.icns, src-tauri\icons\icon.png -Force

Write-Host "Icons regenerated. Rebuild with scripts/build.ps1 to apply."

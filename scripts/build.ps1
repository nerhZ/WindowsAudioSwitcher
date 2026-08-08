# Builds installers and updater artifacts.
# Reads the signing key from ~/.tauri so you don't have to set env vars by hand.
$ErrorActionPreference = "Stop"

$keyPath = Join-Path $HOME ".tauri\audioswitch.key"
if (-not (Test-Path $keyPath)) {
    Write-Error "No signing key at $keyPath - generate one with: bun run tauri signer generate -w $keyPath"
}

$env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content $keyPath -Raw).Trim()
if (-not $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD) {
    Read-Host -Prompt "Signing key password" -AsSecureString |
        ConvertFrom-SecureString -AsPlainText |
        ForEach-Object { $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $_ }
}

bun run tauri build

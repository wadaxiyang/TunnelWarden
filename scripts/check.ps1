param([switch]$Frontend)
$ErrorActionPreference = 'Stop'
Push-Location (Join-Path $PSScriptRoot '..')
try {
    cargo fmt --all -- --check
    if ($LASTEXITCODE) { throw 'cargo fmt failed' }
    cargo check --workspace
    if ($LASTEXITCODE) { throw 'cargo check failed' }
    cargo test --workspace
    if ($LASTEXITCODE) { throw 'cargo test failed' }
    cargo clippy --workspace --all-targets -- -D warnings
    if ($LASTEXITCODE) { throw 'cargo clippy failed' }
    & ./scripts/verify-boundaries.ps1
    python scripts/verify-vendor.py
    if ($LASTEXITCODE) { throw 'Oryxis source/patch verification failed' }
    if ($Frontend) {
        $npm = if (Get-Command npm -ErrorAction SilentlyContinue) { 'npm' } else { './.upstream/tooling/node_modules/.bin/npm.cmd' }
        & $npm run check
        if ($LASTEXITCODE) { throw 'npm check failed' }
        & $npm test
        if ($LASTEXITCODE) { throw 'frontend telemetry tests failed' }
        & $npm run build
        if ($LASTEXITCODE) { throw 'npm build failed' }
    }
} finally { Pop-Location }

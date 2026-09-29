param([Parameter(Mandatory)][string]$Tag)
$ErrorActionPreference = 'Stop'
& "$PSScriptRoot/verify-release.ps1" -Tag $Tag
$installers = @(Get-ChildItem target/release/bundle -Recurse -File | Where-Object { $_.Extension -in '.exe', '.msi' })
if ($installers.Count -ne 2 -or @($installers | Where-Object Extension -eq '.msi').Count -ne 1) { throw 'Expected NSIS and MSI installers' }
foreach ($file in $installers) { if ($file.Length -lt 1MB) { throw "Unexpectedly small installer: $file" } }
# Run only on an ephemeral runner, never over a user's installed application.
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'Installation smoke test requires GitHub Actions' }
$dest = Join-Path $env:RUNNER_TEMP 'TunnelWarden-install-smoke'
if (Test-Path $dest) { throw 'Smoke-test destination must be new' }
$nsis = $installers | Where-Object Extension -eq '.exe'
$install = Start-Process -FilePath $nsis.FullName -ArgumentList @('/S', "/D=$dest") -WindowStyle Hidden -Wait -PassThru
if ($install.ExitCode -ne 0) { throw "NSIS install exited $($install.ExitCode)" }
foreach ($name in @('tunnelwarden-tauri.exe', 'LICENSE', 'NOTICE.ORYXIS', 'THIRD_PARTY_NOTICES.md', 'THIRD_PARTY_LICENSES.txt', 'README.md')) {
    if (-not (Test-Path (Join-Path $dest $name))) { throw "Missing installed file: $name" }
}
$uninstaller = Get-ChildItem $dest -Filter '*uninstall*.exe' | Select-Object -First 1
if (-not $uninstaller) { throw 'NSIS uninstaller missing' }
$uninstall = Start-Process -FilePath $uninstaller.FullName -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
if ($uninstall.ExitCode -ne 0) { throw "Uninstall exited $($uninstall.ExitCode)" }
New-Item release-assets -ItemType Directory -Force | Out-Null
$installers | Copy-Item -Destination release-assets
Copy-Item LICENSE,NOTICE.ORYXIS,THIRD_PARTY_NOTICES.md,THIRD_PARTY_LICENSES.txt -Destination release-assets
$archive = "release-assets/TunnelWarden_${Tag}_source.zip"
git archive --format=zip "--output=$archive" HEAD
if ($LASTEXITCODE) { throw 'Source archive failed' }
Get-ChildItem release-assets -File | Sort-Object Name | ForEach-Object {
    $hash = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $($_.Name)"
} | Set-Content release-assets/SHA256SUMS.txt -Encoding utf8NoBOM

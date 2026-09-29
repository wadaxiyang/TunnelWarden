param([Parameter(Mandatory)][string]$Tag)
$ErrorActionPreference = 'Stop'
$version = (Get-Content package.json -Raw | ConvertFrom-Json).version
if ($Tag -ne "v$version") { throw "Tag $Tag does not match version $version" }
if ((Get-Content src-tauri/tauri.conf.json -Raw | ConvertFrom-Json).version -ne $version) { throw 'Tauri version mismatch' }
foreach ($manifest in @('src-tauri/Cargo.toml', 'crates/tunnelwarden-core/Cargo.toml')) {
    if ((Get-Content $manifest -Raw) -notmatch ('(?m)^version = "' + [regex]::Escape($version) + '"\r?$')) { throw "Version mismatch: $manifest" }
}
$pin = '16ad2af83b424780ff60b6420a6c20a3b30c7143'
if (@(Select-String -Path Cargo.toml -Pattern ('rev = "' + $pin + '"')).Count -ne 2) { throw 'Oryxis pin changed' }
if ((Get-Content Cargo.lock -Raw) -notmatch $pin) { throw 'Oryxis lock revision missing' }
Write-Host "Verified $Tag and Oryxis $pin"

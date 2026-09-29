$ErrorActionPreference = "Stop"
$hits = Get-ChildItem -Recurse crates/tunnelwarden-core, src-tauri -File -Include *.rs | Select-String -Pattern 'russh::|channel_open_direct_tcpip|tcpip_forward\('
if ($hits) {
  $hits | ForEach-Object { Write-Host $_ }
  throw "Direct russh/forwarding calls found outside Oryxis dependency. Review AGENTS.md."
}

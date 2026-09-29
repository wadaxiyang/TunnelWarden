$ErrorActionPreference = "Stop"
$Sha = "16ad2af83b424780ff60b6420a6c20a3b30c7143"
$Url = "https://github.com/wilsonglasser/oryxis.git"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Upstream = Join-Path $Root ".upstream/oryxis"
$Dest = Join-Path $Root "upstream/oryxis-app-reference"

if (Test-Path $Upstream) { Remove-Item -Recurse -Force $Upstream }
New-Item -ItemType Directory -Force -Path $Upstream, $Dest | Out-Null

git init $Upstream | Out-Null
git -C $Upstream remote add origin $Url
git -C $Upstream fetch --depth 1 origin $Sha
git -C $Upstream checkout --detach FETCH_HEAD
$Actual = (git -C $Upstream rev-parse HEAD).Trim()
if ($Actual -ne $Sha) { throw "Oryxis SHA mismatch: $Actual" }

function Copy-AppSource([string]$Relative) {
  $Source = Join-Path $Upstream ("crates/oryxis-app/src/" + $Relative)
  $Target = Join-Path $Dest $Relative
  New-Item -ItemType Directory -Force -Path (Split-Path $Target) | Out-Null
  Copy-Item $Source $Target -Force
}

Copy-AppSource "dispatch_port_forwards.rs"
Copy-AppSource "messages/port_forward.rs"
Copy-AppSource "app.rs"
Copy-AppSource "boot/mod.rs"
Copy-AppSource "subscription.rs"
Copy-AppSource "state/forms.rs"
Copy-AppSource "views/port_forwards.rs"
Copy-Item (Join-Path $Upstream "LICENSE") (Join-Path $Root "LICENSE") -Force
Copy-Item (Join-Path $Upstream "NOTICE") (Join-Path $Root "NOTICE.ORYXIS") -Force

Write-Host "Materialized Oryxis reference at $Sha"

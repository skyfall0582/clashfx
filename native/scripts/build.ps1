param([switch]$DebugBuild)
$ErrorActionPreference = 'Stop'
$nativeRoot = Split-Path -Parent $PSScriptRoot
if (-not (Test-Path -LiteralPath (Join-Path $nativeRoot 'bin\mihomo.exe'))) { & "$PSScriptRoot\fetch-core.ps1" }
if (-not (Test-Path -LiteralPath (Join-Path $nativeRoot 'bin\geodata\manifest.json'))) { & "$PSScriptRoot\fetch-geodata.ps1" }
Push-Location $nativeRoot
try {
    if ($DebugBuild) { cargo build --locked -p clashfx; $configuration = 'debug' }
    else { cargo build --locked --release -p clashfx; $configuration = 'release' }
    if ($LASTEXITCODE -ne 0) { throw 'ClashFx build failed.' }
    $destination = Join-Path $nativeRoot 'releases\ClashFx'
    New-Item -ItemType Directory -Force -Path $destination | Out-Null
    Copy-Item -LiteralPath "target\$configuration\clashfx.exe" -Destination (Join-Path $destination 'ClashFx.exe') -Force
    Copy-Item -LiteralPath 'bin\mihomo.exe' -Destination $destination -Force
    Copy-Item -LiteralPath 'bin\core-version.json' -Destination $destination -Force
    New-Item -ItemType Directory -Force -Path (Join-Path $destination 'geodata') | Out-Null
    Get-ChildItem -LiteralPath 'bin\geodata' -File | Copy-Item -Destination (Join-Path $destination 'geodata') -Force
    Copy-Item -LiteralPath '..\LICENSE', 'NOTICE.txt', 'LICENSE-APACHE', 'THIRD-PARTY-NOTICES.txt', 'README.md', 'USAGE.zh-CN.txt' -Destination $destination -Force
    Write-Output "Built $destination\ClashFx.exe"
} finally { Pop-Location }


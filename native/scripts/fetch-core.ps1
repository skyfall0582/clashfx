param([string]$Version = 'v1.19.33')
$ErrorActionPreference = 'Stop'
$nativeRoot = Split-Path -Parent $PSScriptRoot
$binDirectory = Join-Path $nativeRoot 'bin'
$cacheDirectory = Join-Path $nativeRoot 'work\downloads'
New-Item -ItemType Directory -Force -Path $binDirectory, $cacheDirectory | Out-Null
$headers = @{ 'User-Agent' = 'ClashFx-build' }
$release = $null
for ($attempt = 1; $attempt -le 3; $attempt++) {
    try {
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/MetaCubeX/mihomo/releases/tags/$Version" -Headers $headers
        break
    } catch {
        if ($attempt -eq 3) { throw }
        Start-Sleep -Seconds 2
    }
}
$assetName = "mihomo-windows-amd64-compatible-$Version.zip"
$asset = $release.assets | Where-Object { $_.name -eq $assetName } | Select-Object -First 1
if (-not $asset) { throw "Official Mihomo release asset not found: $assetName" }
$archivePath = Join-Path $cacheDirectory $assetName
if (-not (Test-Path -LiteralPath $archivePath)) {
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $archivePath -Headers $headers -MaximumRetryCount 3 -RetryIntervalSec 2
}
$actualHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($asset.digest -and $asset.digest.StartsWith('sha256:')) {
    $expectedHash = $asset.digest.Substring(7)
    if ($actualHash -ne $expectedHash) { throw 'Mihomo archive SHA256 verification failed.' }
}
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [System.IO.Compression.ZipFile]::OpenRead($archivePath)
try {
    $entry = $archive.Entries | Where-Object { $_.Name -like '*.exe' } | Select-Object -First 1
    if (-not $entry) { throw 'Official release archive contains no executable.' }
    [System.IO.Compression.ZipFileExtensions]::ExtractToFile($entry, (Join-Path $binDirectory 'mihomo.exe'), $true)
} finally { $archive.Dispose() }
@{ version = $Version; asset = $assetName; sha256 = $actualHash; source = $asset.browser_download_url } |
    ConvertTo-Json | Set-Content -LiteralPath (Join-Path $binDirectory 'core-version.json') -Encoding utf8
& (Join-Path $binDirectory 'mihomo.exe') -v
if ($LASTEXITCODE -ne 0) { throw 'Downloaded Mihomo binary did not execute successfully.' }


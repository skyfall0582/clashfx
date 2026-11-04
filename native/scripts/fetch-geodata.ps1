$ErrorActionPreference = 'Stop'
$nativeRoot = Split-Path -Parent $PSScriptRoot
$destination = Join-Path $nativeRoot 'bin\geodata'
New-Item -ItemType Directory -Force -Path $destination | Out-Null
$headers = @{ 'User-Agent' = 'ClashFx-build' }
$release = Invoke-RestMethod -Uri 'https://api.github.com/repos/MetaCubeX/meta-rules-dat/releases/latest' -Headers $headers
$manifest = @()
foreach ($name in @('country.mmdb', 'geoip.dat', 'geosite.dat', 'GeoLite2-ASN.mmdb')) {
    $asset = $release.assets | Where-Object name -eq $name | Select-Object -First 1
    if (-not $asset -or -not $asset.digest.StartsWith('sha256:')) { throw "Missing verified asset: $name" }
    $localName = switch ($name) { 'country.mmdb' { 'Country.mmdb' }; 'GeoLite2-ASN.mmdb' { 'ASN.mmdb' }; default { $name } }
    $path = Join-Path $destination $localName
    $expected = $asset.digest.Substring(7)
    if (-not (Test-Path -LiteralPath $path) -or (Get-FileHash -LiteralPath $path).Hash.ToLowerInvariant() -ne $expected) {
        Invoke-WebRequest -Uri $asset.browser_download_url -OutFile ($path + '.download') -Headers $headers -TimeoutSec 60
        if ((Get-FileHash -LiteralPath ($path + '.download')).Hash.ToLowerInvariant() -ne $expected) { throw "Hash mismatch: $name" }
        Move-Item -LiteralPath ($path + '.download') -Destination $path -Force
    }
    $manifest += @{ name = $localName; sha256 = $expected; source = $asset.browser_download_url; release_date = $release.published_at }
    Write-Output "Verified $localName"
}
$manifest | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $destination 'manifest.json') -Encoding utf8

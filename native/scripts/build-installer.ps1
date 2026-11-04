param(
    [string]$SigningRule = '梵星',
    [string]$SigningToolPath = "${env:ProgramFiles(x86)}\DSignTool\CSignTool.exe",
    [string]$CompilerPath,
    [string]$RuntimeDirectory,
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
$nativeRoot = Split-Path -Parent $PSScriptRoot
$projectRoot = Split-Path -Parent $nativeRoot
$manifest = Get-Content -LiteralPath (Join-Path $nativeRoot 'Cargo.toml') -Raw
$version = [regex]::Match($manifest, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if (-not $version) { throw 'The native workspace version was not found.' }
if (-not (Test-Path -LiteralPath $SigningToolPath)) { throw "Local signing tool not found: $SigningToolPath" }
if (-not $CompilerPath) {
    $candidates = @(
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
    )
    $CompilerPath = $candidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
}
if (-not $CompilerPath -or -not (Test-Path -LiteralPath $CompilerPath)) {
    throw 'Install Inno Setup 6 or pass -CompilerPath.'
}
if (-not $RuntimeDirectory) {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path -LiteralPath $vswhere)) { throw 'Pass -RuntimeDirectory with the Visual C++ x64 CRT redistributable directory.' }
    $visualStudio = & $vswhere -latest -products '*' -property installationPath
    if (-not $visualStudio) { throw 'Visual Studio Build Tools were not found.' }
    $runtime = Get-ChildItem -LiteralPath (Join-Path $visualStudio 'VC\Redist\MSVC') -Filter vcruntime140.dll -File -Recurse |
        Where-Object { $_.FullName -match '\\x64\\Microsoft\.VC\d+\.CRT\\' -and $_.FullName -notmatch '\\onecore\\' } |
        Sort-Object FullName -Descending | Select-Object -First 1
    if (-not $runtime) { throw 'The Visual C++ x64 runtime was not found.' }
    $RuntimeDirectory = $runtime.DirectoryName
}
$runtimePath = Join-Path $RuntimeDirectory 'vcruntime140.dll'
$runtimeSignature = Get-AuthenticodeSignature -LiteralPath $runtimePath
if ($runtimeSignature.Status -ne 'Valid' -or $runtimeSignature.SignerCertificate.Subject -notmatch 'Microsoft Corporation') {
    throw 'The Visual C++ runtime must have a valid Microsoft signature.'
}

Push-Location $nativeRoot
try {
    if (-not $SkipBuild) {
        & "$PSScriptRoot\build.ps1"
    }
    if (-not (Test-Path -LiteralPath 'bin\mihomo.exe')) { & "$PSScriptRoot\fetch-core.ps1" }
    & "$PSScriptRoot\fetch-geodata.ps1"
    $stage = Join-Path $nativeRoot "work\installer-$version\payload"
    $release = Join-Path $nativeRoot 'releases\installer'
    New-Item -ItemType Directory -Force -Path $stage, (Join-Path $stage 'geodata'), (Join-Path $stage 'signed-uninstaller'), $release | Out-Null
    Copy-Item -LiteralPath 'releases\ClashFx\ClashFx.exe' -Destination (Join-Path $stage 'ClashFx.exe') -Force
    Copy-Item -LiteralPath 'bin\mihomo.exe','bin\core-version.json','NOTICE.txt','LICENSE-APACHE','THIRD-PARTY-NOTICES.txt','README.md','USAGE.zh-CN.txt' -Destination $stage -Force
    Copy-Item -LiteralPath (Join-Path $projectRoot 'LICENSE'),$runtimePath -Destination $stage -Force
    foreach ($name in @('Country.mmdb','geoip.dat','geosite.dat','ASN.mmdb','manifest.json')) {
        Copy-Item -LiteralPath (Join-Path $nativeRoot "bin\geodata\$name") -Destination (Join-Path $stage 'geodata') -Force
    }
    foreach ($name in @('ClashFx.exe','mihomo.exe')) {
        $file = Join-Path $stage $name
        & $SigningToolPath sign /r $SigningRule /f $file
        if ($LASTEXITCODE -ne 0) { throw "Local signing failed: $name" }
        $signature = Get-AuthenticodeSignature -LiteralPath $file
        if ($signature.Status -ne 'Valid' -or -not $signature.TimeStamperCertificate) {
            throw "The signature or timestamp is invalid: $name"
        }
    }
    $shellPath = (Get-Process -Id $PID).Path
    $signCommand = '$q{0}$q -NoProfile -ExecutionPolicy Bypass -File $q{1}$q -FilePath $f -SigningToolPath $q{2}$q -SigningRule $q{3}$q' -f $shellPath, (Join-Path $PSScriptRoot 'sign-installer.ps1'), $SigningToolPath, $SigningRule
    & $CompilerPath "/DAppVersion=$version" "/DPayloadDir=$stage" "/DReleaseDir=$release" "/Sclashfx=$signCommand" (Join-Path $nativeRoot 'packaging\windows\installer.iss')
    if ($LASTEXITCODE -ne 0) { throw 'Installer compilation or signing failed.' }
    $installer = Join-Path $release "ClashFx-$version-x64-Setup.exe"
    $signature = Get-AuthenticodeSignature -LiteralPath $installer
    if ($signature.Status -ne 'Valid' -or -not $signature.TimeStamperCertificate) {
        throw 'The installer signature or timestamp is invalid.'
    }
    $hash = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $([IO.Path]::GetFileName($installer))" | Set-Content -LiteralPath "$installer.sha256" -Encoding ascii
    Write-Output "Signed installer: $installer"
} finally { Pop-Location }

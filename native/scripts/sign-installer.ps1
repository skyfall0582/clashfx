param(
    [Parameter(Mandatory)][string]$FilePath,
    [Parameter(Mandatory)][string]$SigningToolPath,
    [Parameter(Mandatory)][string]$SigningRule
)
$ErrorActionPreference = 'Stop'
$signablePath = $FilePath
if ([IO.Path]::GetExtension($FilePath) -ne '.exe') {
    # DSignTool skips Inno Setup's temporary .e32 and .tmp files.
    $signablePath = "$FilePath.exe"
    Copy-Item -LiteralPath $FilePath -Destination $signablePath -Force
}
try {
    & $SigningToolPath sign /r $SigningRule /f $signablePath
    if ($LASTEXITCODE -ne 0) { throw "Local signing failed: $FilePath" }
    $signature = Get-AuthenticodeSignature -LiteralPath $signablePath
    if ($signature.Status -ne 'Valid' -or -not $signature.TimeStamperCertificate) {
        throw "The signature or timestamp is invalid: $FilePath"
    }
    if ($signablePath -ne $FilePath) {
        Copy-Item -LiteralPath $signablePath -Destination $FilePath -Force
    }
} finally {
    if ($signablePath -ne $FilePath) { Remove-Item -LiteralPath $signablePath }
}

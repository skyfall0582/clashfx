param([switch]$Build)
$ErrorActionPreference = 'Stop'
$nativeRoot = Join-Path $PSScriptRoot 'native'
$executable = Join-Path $nativeRoot 'releases\ClashFx\ClashFx.exe'
if ($Build -or -not (Test-Path -LiteralPath $executable)) { & (Join-Path $nativeRoot 'scripts\build.ps1') }
Start-Process -FilePath $executable -WorkingDirectory (Split-Path -Parent $executable)


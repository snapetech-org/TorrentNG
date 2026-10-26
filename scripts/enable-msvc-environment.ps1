$ErrorActionPreference = 'Stop'

$programFilesX86 = [Environment]::GetFolderPath('ProgramFilesX86')
$vswhere = Join-Path $programFilesX86 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path -LiteralPath $vswhere -PathType Leaf)) {
  throw 'Visual Studio locator is unavailable on the private Windows runner'
}

$installation = & $vswhere -latest -products '*' `
  -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
  -property installationPath
if ($LASTEXITCODE -ne 0 -or -not $installation) {
  throw 'Visual Studio C++ Build Tools are unavailable on the private Windows runner'
}

$vcvars = Join-Path $installation 'VC\Auxiliary\Build\vcvars64.bat'
if (-not (Test-Path -LiteralPath $vcvars -PathType Leaf)) {
  throw 'The x64 Visual Studio compiler environment is unavailable'
}

$command = 'call "{0}" >nul && set' -f $vcvars
$lines = & $env:ComSpec /d /s /c $command
if ($LASTEXITCODE -ne 0) {
  throw 'Could not load the Visual Studio x64 compiler environment'
}

$toolchainEnvironment = @{}
foreach ($line in $lines) {
  if ($line -match '^(PATH|INCLUDE|LIB|LIBPATH)=(.*)$') {
    $toolchainEnvironment[$Matches[1]] = $Matches[2]
  }
}

foreach ($name in @('PATH', 'INCLUDE', 'LIB', 'LIBPATH')) {
  if (-not $toolchainEnvironment.ContainsKey($name)) {
    throw "Visual Studio did not provide the required $name environment"
  }
}

$encoding = [System.Text.UTF8Encoding]::new($false)
$writer = [System.IO.StreamWriter]::new($env:GITHUB_ENV, $true, $encoding)
try {
  foreach ($name in @('PATH', 'INCLUDE', 'LIB', 'LIBPATH')) {
    $writer.WriteLine('{0}={1}' -f $name, $toolchainEnvironment[$name])
  }
} finally {
  $writer.Dispose()
}

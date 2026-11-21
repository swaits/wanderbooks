# Sign Windows files with an Authenticode code-signing certificate (plan P15
# unit C1). The release workflow runs this only when its secrets exist
# (WINDOWS_CERTIFICATE_PFX_BASE64 and WINDOWS_CERTIFICATE_PASSWORD); with none
# it prints a notice and the installer stays unsigned (SmartScreen warns).
#
#   sign-windows.ps1 FILE [FILE...]
#
# The certificate is written to a temporary file for the signing and
# removed. Nothing here prints a secret. A failure is loud: a signature that
# was asked for and did not happen stops the release.
param([Parameter(Mandatory = $true, ValueFromRemainingArguments = $true)][string[]]$Files)
$ErrorActionPreference = 'Stop'

if (-not $env:WINDOWS_CERTIFICATE_PFX_BASE64 -or -not $env:WINDOWS_CERTIFICATE_PASSWORD) {
  Write-Host "::notice title=Windows signing skipped::no WINDOWS_CERTIFICATE_PFX_BASE64 / WINDOWS_CERTIFICATE_PASSWORD secret: the installer is unsigned and Windows SmartScreen will warn on the first run."
  exit 0
}

$signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
  Where-Object { $_.FullName -match '\\x64\\' } |
  Sort-Object FullName -Descending | Select-Object -First 1
if (-not $signtool) { throw "signtool.exe was not found in the Windows SDK on this runner" }

$pfx = Join-Path $env:RUNNER_TEMP 'wanderbooks-codesign.pfx'
[IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:WINDOWS_CERTIFICATE_PFX_BASE64))
try {
  foreach ($file in $Files) {
    & $signtool.FullName sign /fd sha256 /f $pfx /p $env:WINDOWS_CERTIFICATE_PASSWORD /tr http://timestamp.digicert.com /td sha256 $file
    if ($LASTEXITCODE -ne 0) { throw "signtool failed on $file" }
    & $signtool.FullName verify /pa $file
    if ($LASTEXITCODE -ne 0) { throw "the signature on $file does not verify" }
    Write-Host "signed $file"
  }
} finally {
  Remove-Item $pfx -Force -ErrorAction SilentlyContinue
}

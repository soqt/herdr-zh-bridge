$ErrorActionPreference = 'Stop'
$parseErrors = $null
$tokens = $null
foreach ($script in @('install.ps1', 'update.ps1')) {
    [void][System.Management.Automation.Language.Parser]::ParseFile(
        (Join-Path $PSScriptRoot $script), [ref]$tokens, [ref]$parseErrors)
    if ($parseErrors.Count) { throw ($parseErrors | Out-String) }
}
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot 'install.ps1'), [ref]$tokens, [ref]$parseErrors)
$functionAst = $ast.Find({param($n)
    $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Invoke-CapturedProcess'
}, $true)
# Load only the process wrapper: the real installer never runs in these tests.
. ([scriptblock]::Create($functionAst.Extent.Text))
$powershellExe = (Get-Process -Id $PID).Path
$LASTEXITCODE = 51
$success = Invoke-CapturedProcess -Executable $powershellExe -Arguments @('-NoProfile', '-Command', '"Write-Output 123"') -LogPrefix (Join-Path $PSScriptRoot 'test-check-success')
if ($success.ExitCode -ne 0 -or $success.Stdout.Trim() -ne '123') { throw 'Success test failed' }
$failure = Invoke-CapturedProcess -Executable $powershellExe -Arguments @('-NoProfile', '-Command', '"[Console]::Error.Write(456); exit 7"') -LogPrefix (Join-Path $PSScriptRoot 'test-check-failure')
if ($failure.ExitCode -ne 7 -or $failure.Stderr.Trim() -ne '456') { throw 'Failure diagnostic test failed' }
Write-Output 'PASS: scripts parse; actual exit code used despite stale LASTEXITCODE; stderr preserved on failure.'

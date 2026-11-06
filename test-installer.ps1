$ErrorActionPreference = 'Stop'
$parseErrors = $null
$tokens = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot 'install.ps1'), [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw ($parseErrors | Out-String) }
$functionAst = $ast.Find({param($n)
    $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Invoke-CapturedProcess'
}, $true)
# Load only the process wrapper: the real installer never runs in these tests.
. ([scriptblock]::Create($functionAst.Extent.Text))
$pythonExe = (Get-Command python.exe).Source
$LASTEXITCODE = 51
$success = Invoke-CapturedProcess -Executable $pythonExe -Arguments @('-c', '"print(123)"') -LogPrefix (Join-Path $PSScriptRoot 'test-check-success')
if ($success.ExitCode -ne 0 -or $success.Stdout.Trim() -ne '123') { throw 'Success test failed' }
$failure = Invoke-CapturedProcess -Executable $pythonExe -Arguments @('-c', '"import sys; sys.stderr.write(str(456)); sys.exit(7)"') -LogPrefix (Join-Path $PSScriptRoot 'test-check-failure')
if ($failure.ExitCode -ne 7 -or $failure.Stderr.Trim() -ne '456') { throw 'Failure diagnostic test failed' }
Write-Output 'PASS: actual exit code used despite stale LASTEXITCODE; stderr preserved on failure.'

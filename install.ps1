param(
    [string]$HerdrHome,
    [string]$HerdrExe
)
$ErrorActionPreference = 'Stop'
function Invoke-CapturedProcess {
    param([string]$Executable, [string[]]$Arguments, [string]$LogPrefix)
    $stdoutPath = $LogPrefix + '.stdout.log'
    $stderrPath = $LogPrefix + '.stderr.log'
    $process = Start-Process -FilePath $Executable -ArgumentList $Arguments `
        -WindowStyle Hidden -Wait -PassThru `
        -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
    # GUI-subsystem executables do not reliably update the caller's LASTEXITCODE.
    # Use the exit code of the exact process we started and waited for.
    $exitCode = $process.ExitCode
    $stdout = Get-Content -LiteralPath $stdoutPath -Raw -Encoding UTF8 -ErrorAction SilentlyContinue
    $stderr = Get-Content -LiteralPath $stderrPath -Raw -Encoding UTF8 -ErrorAction SilentlyContinue
    [pscustomobject]@{ ExitCode = $exitCode; Stdout = $stdout; Stderr = $stderr }
}
if (-not $HerdrExe) {
    $appCommand = Get-Command herdr-app.exe -ErrorAction SilentlyContinue
    if ($appCommand) { $HerdrExe = $appCommand.Source }
    else { $HerdrExe = Join-Path $env:LOCALAPPDATA 'Programs\herdr-app\herdr-app.exe' }
}
if (-not (Test-Path -LiteralPath $HerdrExe -PathType Leaf)) {
    throw 'Cannot find herdr-app.exe. Run again with -HerdrExe followed by its full path.'
}
if (-not $HerdrHome) {
    $candidates = @(
        $env:HERDR_APP_HOME,
        (Join-Path $env:USERPROFILE '.config\herdr-app'),
        (Join-Path $env:APPDATA 'herdr-app')
    )
    if ($env:XDG_CONFIG_HOME) { $candidates += Join-Path $env:XDG_CONFIG_HOME 'herdr-app' }
    $found = @($candidates | Where-Object { $_ -and (Test-Path -LiteralPath (Join-Path $_ 'AGENTS.md')) } | Select-Object -Unique)
    if ($found.Count -ne 1) {
        throw 'Could not identify one Herdr configuration folder. Run again with -HerdrHome pointing to the folder containing AGENTS.md.'
    }
    $HerdrHome = $found[0]
}
$destination = Join-Path $HerdrHome 'mods\zh-bridge'
if (-not (Test-Path -LiteralPath (Join-Path $HerdrHome 'AGENTS.md'))) {
    throw 'HerdrHome must point to the configuration directory containing AGENTS.md.'
}
if (Test-Path -LiteralPath $destination) {
    throw 'zh-bridge already exists. No existing files were replaced.'
}
# Validate before installing; checks do not modify existing layout/configuration.
$modPath = Join-Path $PSScriptRoot 'mod'
$logPrefix = Join-Path $PSScriptRoot 'herdr-check'
$check = Invoke-CapturedProcess -Executable $HerdrExe `
    -Arguments @('check', ('"{0}"' -f $modPath)) -LogPrefix $logPrefix
$summary = @(
    "Executable: $HerdrExe",
    "Configuration: $HerdrHome",
    "Mod: $modPath",
    "Exit code: $($check.ExitCode)",
    '--- stdout ---', $check.Stdout,
    '--- stderr ---', $check.Stderr
) -join [Environment]::NewLine
$summary | Set-Content -LiteralPath (Join-Path $PSScriptRoot 'herdr-check.log') -Encoding UTF8
if ($check.Stdout) { Write-Output $check.Stdout }
if ($check.Stderr) { Write-Output $check.Stderr }
if ($null -eq $check.ExitCode -or $check.ExitCode -ne 0) {
    Write-Output $summary
    throw "Herdr validation did not pass. Details saved to $PSScriptRoot\herdr-check.log. No mod was installed."
}
New-Item -ItemType Directory -Path $destination | Out-Null
foreach ($name in @('bridge.py', 'panel.py', 'catalog.py', 'widgets.py', 'replies.py', 'conversation.py', 'platform_support.py', 'tab_group.py')) {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination (Join-Path $destination $name)
}
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'mod\main.js') -Destination (Join-Path $destination 'main.js')
# Manifest last, so the initial discovery sees a complete mod.
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'mod\mod.json') -Destination (Join-Path $destination 'mod.json')
Write-Output "Installed preview at $destination. Open the Chinese translation panel from Herdr."

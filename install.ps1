param(
    [string]$HerdrHome,
    [string]$HerdrExe,
    [string]$Binary
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
function Resolve-BridgeBinary {
    param([string]$Binary)
    if ($Binary) { return (Resolve-Path -LiteralPath $Binary).Path }
    $release = Join-Path $PSScriptRoot 'herdr-zh-bridge.exe'
    if (Test-Path -LiteralPath $release -PathType Leaf) { return $release }
    $built = Join-Path $PSScriptRoot 'target\release\herdr-zh-bridge.exe'
    if ((Test-Path -LiteralPath (Join-Path $PSScriptRoot 'Cargo.toml')) -and (Get-Command cargo.exe -ErrorAction SilentlyContinue)) {
        Push-Location $PSScriptRoot
        try { & cargo build --release; if ($LASTEXITCODE -ne 0) { throw 'cargo build failed.' } }
        finally { Pop-Location }
    }
    if (Test-Path -LiteralPath $built -PathType Leaf) { return $built }
    throw 'Cannot find herdr-zh-bridge.exe. Use a release package, pass -Binary, or install Rust and run from the source folder.'
}
function Test-BridgeBinary {
    param([string]$Path)
    $check = Invoke-CapturedProcess -Executable $Path -Arguments @('--version') -LogPrefix (Join-Path $env:TEMP 'herdr-zh-bridge-version')
    if ($check.ExitCode -ne 0 -or -not $check.Stdout -or -not $check.Stdout.StartsWith('herdr-zh-bridge ')) {
        throw "herdr-zh-bridge.exe did not start correctly (exit code $($check.ExitCode))."
    }
}

$bridge = Resolve-BridgeBinary -Binary $Binary
Test-BridgeBinary -Path $bridge
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
    throw 'zh-bridge already exists. No existing files were replaced. Use update.ps1 to update it.'
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
Copy-Item -LiteralPath $bridge -Destination (Join-Path $destination 'herdr-zh-bridge.exe')
Copy-Item -LiteralPath (Join-Path $modPath 'main.js') -Destination (Join-Path $destination 'main.js')
# Manifest last, so the initial discovery sees a complete mod.
Copy-Item -LiteralPath (Join-Path $modPath 'mod.json') -Destination (Join-Path $destination 'mod.json')

# A Start menu entry, so the app also opens without Herdr.
$programs = [Environment]::GetFolderPath('Programs')
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut((Join-Path $programs 'Herdr ZH Bridge.lnk'))
$shortcut.TargetPath = Join-Path $destination 'herdr-zh-bridge.exe'
$shortcut.WorkingDirectory = $destination
$shortcut.Description = 'Herdr ZH Bridge'
$shortcut.Save()
Write-Output "Installed at $destination. Open the Chinese bridge from Herdr or the Start menu."

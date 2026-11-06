param([string]$Target = '', [string]$HerdrHome = '', [string]$Binary = '')
$ErrorActionPreference = 'Stop'
function Invoke-CapturedProcess {
    param([string]$Executable, [string[]]$Arguments, [string]$LogPrefix)
    $stdoutPath = $LogPrefix + '.stdout.log'
    $stderrPath = $LogPrefix + '.stderr.log'
    $process = Start-Process -FilePath $Executable -ArgumentList $Arguments `
        -WindowStyle Hidden -Wait -PassThru `
        -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
    # GUI-subsystem executables do not reliably update the caller's LASTEXITCODE.
    $stdout = Get-Content -LiteralPath $stdoutPath -Raw -Encoding UTF8 -ErrorAction SilentlyContinue
    [pscustomobject]@{ ExitCode = $process.ExitCode; Stdout = "$stdout".Trim() }
}
if (-not $HerdrHome) {
    $HerdrHome = Join-Path $env:USERPROFILE '.config\herdr-app'
    if ($env:HERDR_APP_HOME) { $HerdrHome = $env:HERDR_APP_HOME }
}
$destination = Join-Path $HerdrHome 'mods\zh-bridge'
if (-not (Test-Path -LiteralPath (Join-Path $destination 'mod.json'))) {
    throw 'Installed zh-bridge not found. Specify -HerdrHome with the actual configuration path.'
}
if ($Target -and ($Target -notmatch '^[A-Za-z0-9_./-]+$' -or $Target.StartsWith('-'))) {
    throw 'Invalid terminal address.'
}
if ($Binary) { $bridge = (Resolve-Path -LiteralPath $Binary).Path }
else {
    $bridge = Join-Path $PSScriptRoot 'herdr-zh-bridge.exe'
    if (-not (Test-Path -LiteralPath $bridge -PathType Leaf)) {
        if ((Test-Path -LiteralPath (Join-Path $PSScriptRoot 'Cargo.toml')) -and (Get-Command cargo.exe -ErrorAction SilentlyContinue)) {
            Push-Location $PSScriptRoot
            try { & cargo build --release; if ($LASTEXITCODE -ne 0) { throw 'cargo build failed; update stopped.' } }
            finally { Pop-Location }
        }
        $bridge = Join-Path $PSScriptRoot 'target\release\herdr-zh-bridge.exe'
    }
}
if (-not (Test-Path -LiteralPath $bridge -PathType Leaf)) { throw 'Cannot find the new herdr-zh-bridge.exe; update stopped.' }
$check = Invoke-CapturedProcess -Executable $bridge -Arguments @('--version') -LogPrefix (Join-Path $env:TEMP 'herdr-zh-bridge-version')
$version = $check.Stdout
if ($check.ExitCode -ne 0 -or -not $version.StartsWith('herdr-zh-bridge ')) { throw 'The new executable failed validation; update stopped.' }

$backup = Join-Path $destination ('.backup-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $backup | Out-Null
# Moving works even while the old window is open; Python 0.7 modules move too.
foreach ($name in @('herdr-zh-bridge.exe', 'main.js', 'bridge.py', 'panel.py', 'catalog.py', 'widgets.py',
                    'replies.py', 'conversation.py', 'platform_support.py', 'tab_group.py')) {
    $old = Join-Path $destination $name
    if (Test-Path -LiteralPath $old) { Move-Item -LiteralPath $old -Destination (Join-Path $backup $name) }
}
Copy-Item -LiteralPath $bridge -Destination (Join-Path $destination 'herdr-zh-bridge.exe')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'mod\main.js') -Destination (Join-Path $destination 'main.js')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'mod\mod.json') -Destination (Join-Path $destination 'mod.json') -Force
Write-Output "Updated to $version. Backup: $backup"
$arguments = @()
if ($Target) { $arguments += @('--target', $Target) }
Start-Process -FilePath (Join-Path $destination 'herdr-zh-bridge.exe') -ArgumentList $arguments | Out-Null
Write-Output 'Opened the updated workspace. Use + beside the tabs to add a terminal. Drag a tab to the right to split, or onto a tab bar to merge.'

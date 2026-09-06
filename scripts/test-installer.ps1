#Requires -Version 5.1
<#
.SYNOPSIS
Exercise install, upgrade and uninstall in a workspace test directory.
.DESCRIPTION
Close OpenCrate first. Requires no pre-existing installed copy or shortcuts.
Touches current-user installation/shortcut records and the OpenCrate startup
value temporarily; restores the original startup value and preserves settings.
#>
[CmdletBinding()]
param([Parameter(Mandatory)][string]$InstallerPath)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$InstallerPath = (Resolve-Path -LiteralPath $InstallerPath).Path
$testRoot = [IO.Path]::GetFullPath((Join-Path $projectRoot 'target/installer smoke test'))
$workspaceTarget = [IO.Path]::GetFullPath((Join-Path $projectRoot 'target')) + [IO.Path]::DirectorySeparatorChar
if (-not $testRoot.StartsWith($workspaceTarget, [StringComparison]::OrdinalIgnoreCase)) { throw 'Test path is outside target.' }
$appDir = Join-Path $testRoot 'OpenCrate'
$uninstaller = Join-Path $appDir 'unins000.exe'
$uninstallKey = 'HKCU:/Software/Microsoft/Windows/CurrentVersion/Uninstall/{E83E52E6-E77A-4C03-B131-21AEBC26DDE9}_is1'
$runKey = 'HKCU:/Software/Microsoft/Windows/CurrentVersion/Run'
$desktopLink = Join-Path ([Environment]::GetFolderPath('Desktop')) 'OpenCrate.lnk'
$startLink = Join-Path ([Environment]::GetFolderPath('Programs')) 'OpenCrate.lnk'
$settingsFile = Join-Path $env:APPDATA 'opencrate/settings.json'
if ((Test-Path -LiteralPath $uninstallKey) -or (Test-Path -LiteralPath $uninstaller) -or
    (Test-Path -LiteralPath $desktopLink) -or (Test-Path -LiteralPath $startLink)) {
    throw 'An installation or shortcut already exists. Use a clean test account.'
}
$mutex = $null
if ([Threading.Mutex]::TryOpenExisting('Local\opencrate.ui.v1', [ref]$mutex)) {
    $mutex.Dispose()
    throw 'Quit OpenCrate from the tray before this test.'
}
$originalStartup = (Get-ItemProperty -LiteralPath $runKey -Name opencrate -ErrorAction SilentlyContinue).opencrate
$settingsHash = if (Test-Path -LiteralPath $settingsFile) { (Get-FileHash -LiteralPath $settingsFile).Hash } else { $null }
$installedStartup = '"' + (Join-Path $appDir 'OpenCrate.exe') + '" --startup'
$testForeignStartup = '"' + (Join-Path $testRoot 'another-copy.exe') + '" --startup'

function Assert-True([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function Read-Startup { (Get-ItemProperty -LiteralPath $runKey -Name opencrate -ErrorAction SilentlyContinue).opencrate }
function Run-Setup([string]$Language, [string]$LogName) {
    $process = Start-Process -FilePath $InstallerPath -ArgumentList @(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/TASKS=desktopicon',
        "/LANG=$Language", ('/DIR="' + $appDir + '"'), ('/LOG="' + (Join-Path $testRoot $LogName) + '"')
    ) -WindowStyle Hidden -Wait -PassThru
    Assert-True ($process.ExitCode -eq 0) "Setup failed: $($process.ExitCode). See $LogName."
}
function Run-Uninstall([string]$LogName) {
    $process = Start-Process -FilePath $uninstaller -ArgumentList @(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', ('/LOG="' + (Join-Path $testRoot $LogName) + '"')
    ) -WindowStyle Hidden -Wait -PassThru
    Assert-True ($process.ExitCode -eq 0) "Uninstall failed: $($process.ExitCode)."
}
New-Item -ItemType Directory -Force -Path $testRoot | Out-Null
try {
    # Simulate an existing portable startup preference using the real executable.
    $portable = Join-Path $projectRoot 'target/installer-payload/OpenCrate.exe'
    Assert-True (Test-Path -LiteralPath $portable) 'Build the installer before testing.'
    if (-not (Test-Path -LiteralPath $runKey)) { New-Item -Path $runKey | Out-Null }
    Set-ItemProperty -LiteralPath $runKey -Name opencrate -Value ('"' + $portable + '" --startup')
    Run-Setup 'en' 'install.log'
    $registration = Get-ItemProperty -LiteralPath $uninstallKey
    Assert-True ($registration.DisplayName -like 'OpenCrate*') 'Missing installed-app registration.'
    Assert-True ($registration.InstallLocation.TrimEnd('\') -eq $appDir) 'Wrong installation directory.'
    Assert-True ((Read-Startup) -eq $installedStartup) 'Portable startup preference was not migrated.'
    Assert-True ((Get-FileHash -LiteralPath (Join-Path $appDir 'OpenCrate.exe')).Hash -eq (Get-FileHash -LiteralPath $portable).Hash) 'Installed executable differs from payload.'
    $shell = New-Object -ComObject WScript.Shell
    foreach ($link in @($desktopLink, $startLink)) {
        Assert-True (Test-Path -LiteralPath $link) "Missing shortcut: $link"
        Assert-True ($shell.CreateShortcut($link).TargetPath -eq (Join-Path $appDir 'OpenCrate.exe')) 'Shortcut points to the wrong executable.'
    }
    # Reinstall over the same stable AppId, with another installer language.
    Run-Setup 'tr' 'upgrade.log'
    Assert-True ((Read-Startup) -eq $installedStartup) 'Upgrade changed the startup preference.'
    Run-Uninstall 'uninstall-owned.log'
    Assert-True (-not (Read-Startup)) 'Uninstall left an owned startup entry.'
    Assert-True (-not (Test-Path -LiteralPath $uninstallKey)) 'Uninstall registration was not removed.'
    foreach ($link in @($desktopLink, $startLink)) { Assert-True (-not (Test-Path -LiteralPath $link)) 'Uninstall left a shortcut.' }
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $appDir 'OpenCrate.exe'))) 'Uninstall left the app executable.'

    Run-Setup 'zh_CN' 'install-chinese.log'
    Assert-True (-not (Read-Startup)) 'Fresh install unexpectedly enabled startup.'
    Set-ItemProperty -LiteralPath $runKey -Name opencrate -Value $testForeignStartup
    Run-Uninstall 'uninstall-foreign.log'
    Assert-True ((Read-Startup) -eq $testForeignStartup) 'Uninstall removed another copy startup entry.'
    $currentHash = if (Test-Path -LiteralPath $settingsFile) { (Get-FileHash -LiteralPath $settingsFile).Hash } else { $null }
    Assert-True ($currentHash -eq $settingsHash) 'Installer changed user preferences.'
    Write-Output 'PASS: install, upgrade, all installer languages, shortcuts, uninstall, startup ownership and preference preservation.'
}
finally {
    try {
        if (Test-Path -LiteralPath $uninstaller) { Run-Uninstall 'cleanup.log' }
    }
    finally {
        $currentStartup = Read-Startup
        $portableStartup = '"' + (Join-Path $projectRoot 'target/installer-payload/OpenCrate.exe') + '" --startup'
        if (-not $currentStartup -or $currentStartup -in @($installedStartup, $testForeignStartup, $portableStartup, $originalStartup)) {
            if ($null -eq $originalStartup) { Remove-ItemProperty -LiteralPath $runKey -Name opencrate -ErrorAction SilentlyContinue }
            else { Set-ItemProperty -LiteralPath $runKey -Name opencrate -Value $originalStartup }
        }
        else { Write-Warning 'Startup changed externally during the test; its new value was preserved.' }
    }
}

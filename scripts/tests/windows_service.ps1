# Tests the Task Scheduler helpers offline, then has Windows check the task XML without
# registering it. Run: ./scripts/tests/windows_service.ps1 -FixtureDirectory DIR
param([Parameter(Mandatory=$true)][string]$FixtureDirectory,[switch]$SkipNativeValidation)
$ErrorActionPreference = 'Stop'
$source = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot '../../bin/hoist/src/service/windows.ps1')
$parseErrors = $null
$tokens = $null
$null = [Management.Automation.Language.Parser]::ParseInput($source, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw 'Windows manager script failed PowerShell parsing' }
$installer = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot '../install/install.ps1')
$null = [Management.Automation.Language.Parser]::ParseInput($installer, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw 'Windows installer failed PowerShell parsing' }
# Import only the pure comparison helper, never manager operations.
. ([ScriptBlock]::Create(($source -split '# BEGIN_MANAGER_OPERATIONS', 2)[0]))
$xml = Get-Content -Raw -LiteralPath (Join-Path $FixtureDirectory 'task.xml')
$shape = Get-HoistaTaskShape $xml
if (-not $shape) { throw 'Task shape is empty' }
[xml]$document = $xml
if ($document.Task.Actions.Exec.Command -ne "C:\O'Brien & tools\hoistd.exe") { throw 'XML path escaping did not round trip' }
if ((Get-HoistaTaskShape $document.OuterXml) -cne $shape) { throw 'Whitespace/serialization changed task identity' }
foreach ($change in @(
    @('hoistd.exe', 'other.exe'),
    @('</Arguments>', '</Arguments><WorkingDirectory>C:\different</WorkingDirectory>'),
    @('</TimeTrigger>', '<RandomDelay>PT5M</RandomDelay></TimeTrigger>'),
    @('InteractiveToken', 'Password'),
    @('LeastPrivilege', 'HighestAvailable'),
    @('<Interval>PT1M</Interval>', '<Interval>PT5M</Interval>'),
    @('<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>', '<ExecutionTimeLimit>PT72H</ExecutionTimeLimit>'),
    @('<RunOnlyIfNetworkAvailable>false', '<RunOnlyIfNetworkAvailable>true'),
    @('<StopIfGoingOnBatteries>false', '<StopIfGoingOnBatteries>true'),
    @('<MultipleInstancesPolicy>IgnoreNew', '<MultipleInstancesPolicy>Parallel'),
    @('</Repetition>', '<Duration>PT5M</Duration></Repetition>')
)) {
    $changed = $xml.Replace($change[0], $change[1])
    if ($changed -ceq $xml -or (Get-HoistaTaskShape $changed) -ceq $shape) { throw "Missed task policy change: $($change[0])" }
}
# Task enabled/disabled is an operational switch, not a different task definition.
$document.Task.Settings.Enabled = 'false'
if ((Get-HoistaTaskShape $document.OuterXml) -cne $shape) { throw 'Disabled task cannot be re-enabled safely' }
# Run the real manager script against in-memory stand-ins for the COM objects.
$script:FixturePresent = $false
$script:FixtureLookupError = 0
$script:FixtureCreates = 0
$script:FixtureRuns = 0
$script:FixtureStops = 0
$script:FixtureDeletes = 0
$script:FixtureTask = [pscustomobject]@{Xml=$xml; Enabled=$true; State=3}
$script:FixtureTask | Add-Member ScriptMethod Run { param($parameters) $script:FixtureRuns++; $this.State=4; return $null }
$script:FixtureTask | Add-Member ScriptMethod Stop { param($flags) if ($flags -ne 0) { throw 'Invalid stop flags' }; $script:FixtureStops++; $this.State=3 }
$script:FixtureFolder = [pscustomobject]@{}
$script:FixtureFolder | Add-Member ScriptMethod GetTask {
    param($name)
    if ($script:FixtureLookupError) { throw [Runtime.InteropServices.COMException]::new('Fixture access error', $script:FixtureLookupError) }
    if (-not $script:FixturePresent) { throw [Runtime.InteropServices.COMException]::new('Fixture absent', -2147024894) }
    return $script:FixtureTask
}
$script:FixtureFolder | Add-Member ScriptMethod RegisterTask {
    param($name,$definition,$flags,$sid,$password,$logon,$sddl)
    if ($flags -ne 2 -or $logon -ne 3 -or $null -ne $password -or $sid -ne 'S-1-5-21-123') { throw 'Unexpected task permissions or credentials' }
    $script:FixtureCreates++
    $script:FixturePresent=$true
    $script:FixtureTask.Xml=$definition
    return $script:FixtureTask
}
$script:FixtureFolder | Add-Member ScriptMethod DeleteTask { param($name,$flags) $script:FixtureDeletes++; $script:FixturePresent=$false }
$script:FixtureService = [pscustomobject]@{}
$script:FixtureService | Add-Member ScriptMethod Connect { }
$script:FixtureService | Add-Member ScriptMethod GetFolder { param($path) if ($path -ne '\') { throw 'Unexpected task folder' }; return $script:FixtureFolder }
function New-Object {
    param([Parameter(Position=0)][string]$TypeName,[Parameter(Position=1)][object[]]$ArgumentList,[string]$ComObject)
    if ($ComObject) {
        if ($ComObject -ne 'Schedule.Service') { throw 'Unexpected COM class' }
        return $script:FixtureService
    }
    Microsoft.PowerShell.Utility\New-Object @PSBoundParameters
}
function Invoke-FixtureManager([string]$Operation) {
    $HoistaTaskName='Hoista-Fixture'
    $HoistaXml=$xml
    $HoistaAction=$Operation
    return (& ([ScriptBlock]::Create($source)) | ConvertFrom-Json)
}
try {
    if ((Invoke-FixtureManager 'status').registered) { throw 'Absent task reported registered' }
    if (-not (Invoke-FixtureManager 'install').registered) { throw 'Task was not registered' }
    $null=Invoke-FixtureManager 'install'
    if ($script:FixtureCreates -ne 1 -or $script:FixtureRuns -ne 1) { throw 'Install was not idempotent' }
    $script:FixtureTask.Enabled=$false
    if ((Invoke-FixtureManager 'status').registered) { throw 'Disabled task reported enabled' }
    $null=Invoke-FixtureManager 'install'
    if (-not $script:FixtureTask.Enabled) { throw 'Install did not re-enable matching task' }
    $script:FixtureTask.Xml=$xml.Replace('hoistd.exe','different.exe')
    foreach ($operation in @('install','uninstall','status')) {
        $rejected=$false
        try { $null=Invoke-FixtureManager $operation } catch { $rejected=$true }
        if (-not $rejected) { throw 'Different task was accepted' }
    }
    if ($script:FixtureDeletes -or $script:FixtureCreates -ne 1) { throw 'Different task was mutated' }
    $script:FixtureTask.Xml=$xml
    $script:FixtureLookupError=-2147024891 # E_ACCESSDENIED must not mean absent.
    $rejected=$false
    try { $null=Invoke-FixtureManager 'install' } catch { $rejected=$true }
    if (-not $rejected -or $script:FixtureCreates -ne 1) { throw 'Access failure was treated as absent' }
    $script:FixtureLookupError=0
    $rejected=$false
    try { $null=Invoke-FixtureManager 'restart' } catch { $rejected=$true }
    if (-not $rejected -or $script:FixtureRuns -ne 1 -or $script:FixtureStops -ne 0) { throw 'Force restart must not be exposed by the manager helper' }
    if ((Invoke-FixtureManager 'uninstall').registered) { throw 'Removal reported registered' }
    if ($script:FixtureStops -ne 1 -or $script:FixtureDeletes -ne 1) { throw 'Task was not stopped and removed' }
    $null=Invoke-FixtureManager 'uninstall'
    if ($script:FixtureDeletes -ne 1) { throw 'Absent task removal was not idempotent' }
} finally { Remove-Item Function:New-Object }
Write-Host 'Manager fixtures passed: create/reuse/enable/status/remove, different-task refusal and access errors.'
if ($SkipNativeValidation) { Write-Host 'Cross-platform PowerShell task comparisons and script parsing passed; native validation skipped.'; return }
$service = New-Object -ComObject 'Schedule.Service'
$service.Connect()
$folder = $service.GetFolder('\')
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$validationXml = $xml.Replace('S-1-5-21-123', $sid)
# TASK_VALIDATE_ONLY=1 checks XML using Windows but never persists or executes it.
$null = $folder.RegisterTask('Hoista-Validation-Only', $validationXml, 1, $sid, $null, 3, $null)
Write-Host 'Windows task policy comparisons, script parsing and native XML validation passed; no task registered.'

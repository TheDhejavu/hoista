# Embedded by the CLI after literal task-name/XML/action assignments. No credentials.
$ErrorActionPreference = 'Stop'
function Get-HoistaTaskShape([string]$Text) {
    $document = New-Object System.Xml.XmlDocument
    $document.XmlResolver = $null
    $document.LoadXml($Text)
    $namespaces = New-Object System.Xml.XmlNamespaceManager($document.NameTable)
    $namespaces.AddNamespace('t', 'http://schemas.microsoft.com/windows/2004/02/mit/task')
    $result = @()
    foreach ($path in @('/t:Task/t:Actions/*', '/t:Task/t:Principals/*', '/t:Task/t:Triggers/*')) {
        $result += $document.SelectNodes($path, $namespaces).Count
    }
    $paths = @(
        'RegistrationInfo/Description', 'Actions/Exec/Command', 'Actions/Exec/Arguments', 'Actions/Exec/WorkingDirectory',
        'Principals/Principal/UserId', 'Principals/Principal/LogonType', 'Principals/Principal/RunLevel',
        'Settings/MultipleInstancesPolicy', 'Settings/DisallowStartIfOnBatteries',
        'Settings/StopIfGoingOnBatteries', 'Settings/AllowHardTerminate', 'Settings/StartWhenAvailable',
        'Settings/RunOnlyIfNetworkAvailable', 'Settings/RunOnlyIfIdle', 'Settings/ExecutionTimeLimit',
        'Settings/AllowStartOnDemand', 'Settings/WakeToRun', 'Settings/Hidden',
        'Triggers/LogonTrigger/UserId', 'Triggers/LogonTrigger/Enabled',
        'Triggers/TimeTrigger/StartBoundary', 'Triggers/TimeTrigger/Enabled',
        'Triggers/TimeTrigger/Repetition/Interval', 'Triggers/TimeTrigger/Repetition/Duration',
        'Triggers/TimeTrigger/Repetition/StopAtDurationEnd', 'Triggers/TimeTrigger/EndBoundary',
        'Triggers/LogonTrigger/EndBoundary', 'Triggers/LogonTrigger/Delay',
        'Triggers/TimeTrigger/RandomDelay', 'Triggers/TimeTrigger/ExecutionTimeLimit',
        'Triggers/LogonTrigger/ExecutionTimeLimit', 'Triggers/LogonTrigger/Repetition/Interval'
    )
    foreach ($path in $paths) {
        $nodes = $document.SelectNodes(('/t:Task/t:' + $path.Replace('/', '/t:')), $namespaces)
        $result += $nodes.Count
        foreach ($node in $nodes) { $result += $node.InnerText }
    }
    return ConvertTo-Json -InputObject $result -Compress
}
# BEGIN_MANAGER_OPERATIONS
$expected = Get-HoistaTaskShape $HoistaXml
$service = New-Object -ComObject 'Schedule.Service'
$service.Connect()
$folder = $service.GetFolder('\')
$existing = $null
try { $existing = $folder.GetTask($HoistaTaskName) }
catch {
    # Only ERROR_FILE_NOT_FOUND / ERROR_PATH_NOT_FOUND means absent; do not hide access errors.
    $exception = $_.Exception
    while ($exception.InnerException) { $exception = $exception.InnerException }
    if ($exception.HResult -notin @(-2147024894, -2147024893)) { throw }
}
if ($existing -and (Get-HoistaTaskShape $existing.Xml) -cne $expected) {
    throw 'Existing task differs from this Hoista installation.'
}
switch ($HoistaAction) {
    'install' {
        if (-not $existing) {
            $sid = ([xml]$HoistaXml).Task.Principals.Principal.UserId
            # TASK_CREATE=2, TASK_LOGON_INTERACTIVE_TOKEN=3. Never overwrite another task.
            $existing = $folder.RegisterTask($HoistaTaskName, $HoistaXml, 2, $sid, $null, 3, $null)
        }
        $existing.Enabled = $true
        if ($existing.State -ne 4) { $null = $existing.Run($null) }
        @{registered=$true} | ConvertTo-Json -Compress
    }
    'status' { @{registered=[bool]($existing -and $existing.Enabled)} | ConvertTo-Json -Compress }
    'uninstall' {
        if ($existing) {
            $existing.Enabled = $false
            $existing.Stop(0)
            $folder.DeleteTask($HoistaTaskName, 0)
        }
        @{registered=$false} | ConvertTo-Json -Compress
    }
    default { throw 'Invalid Hoista service operation.' }
}

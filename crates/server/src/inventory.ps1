$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$ProgressPreference = 'SilentlyContinue'
$session = $null
function Write-InventoryFailure([string]$Code) {
    [Console]::Out.Write((ConvertTo-Json -InputObject @{ failure=$Code; providers=@() } -Compress))
    exit 0
}
try {
    $inputData = [Console]::In.ReadToEnd() | ConvertFrom-Json
    $tcp = [Net.Sockets.TcpClient]::new()
    $tls = $null
    $connectionFailure = 'Unreachable'
    try {
        if ($tcp.ConnectAsync([string]$inputData.host, 5986).Wait(4000)) {
            $connectionFailure = 'TlsFailed'
            $tls = [Net.Security.SslStream]::new($tcp.GetStream(), $false)
            if ($tls.AuthenticateAsClientAsync([string]$inputData.host).Wait(4000)) { $connectionFailure = $null }
        }
    } catch {} finally {
        if ($tls) { $tls.Dispose() }
        $tcp.Dispose()
    }
    if ($connectionFailure) { Write-InventoryFailure $connectionFailure }

    $credentialParameters=@{}
    if (-not $inputData.use_server_identity) {
    $securePassword = ConvertTo-SecureString $inputData.password -AsPlainText -Force
    $credential = [Management.Automation.PSCredential]::new($inputData.username, $securePassword)
    $credentialParameters.Credential=$credential
    }
    $inputData.password = $null
    $option = New-CimSessionOption -UseSsl
    try {
        $session = New-CimSession -ComputerName $inputData.host -Port 5986 @credentialParameters `
            -Authentication $inputData.winrm_authentication -SessionOption $option -OperationTimeoutSec 8
    } catch {
        if ($_.FullyQualifiedErrorId -match '(?i)0x803381ba') { Write-InventoryFailure 'TlsFailed' }
        if ($_.Exception.StatusCode -eq 'AccessDenied' -or $_.FullyQualifiedErrorId -match '(?i)0x(?:80070005|8007052e|8009030c|8009030e)') {
            Write-InventoryFailure 'AccessDenied'
        }
        Write-InventoryFailure 'SessionFailed'
    }

    $queries = [ordered]@{
        ComputerSystem = @('Win32_ComputerSystem','Name,Domain,Manufacturer,Model,SystemType,TotalPhysicalMemory')
        OperatingSystem = @('Win32_OperatingSystem','Caption,Version,BuildNumber,OSArchitecture,LastBootUpTime,InstallDate,SystemDrive')
        Bios = @('Win32_BIOS','Manufacturer,SMBIOSBIOSVersion,SerialNumber,ReleaseDate')
        Cpus = @('Win32_Processor','Name,Manufacturer,NumberOfCores,NumberOfLogicalProcessors,MaxClockSpeed')
        MemoryModules = @('Win32_PhysicalMemory','Capacity,Speed,Manufacturer,PartNumber,DeviceLocator')
        Nics = @('Win32_NetworkAdapterConfiguration','Description,MACAddress,IPAddress,IPSubnet,DefaultIPGateway,DHCPEnabled,DNSHostName')
        DiskDrives = @('Win32_DiskDrive','Model,SerialNumber,Size,InterfaceType,MediaType')
        LogicalDisks = @('Win32_LogicalDisk','DeviceID,DriveType,FileSystem,Size,FreeSpace,VolumeName')
        Displays = @('Win32_VideoController','Name,DriverVersion,AdapterRAM,CurrentHorizontalResolution,CurrentVerticalResolution')
        Hardware = @('Win32_PnPEntity','Name,Manufacturer,PNPClass,Status')
        HotFixes = @('Win32_QuickFixEngineering','HotFixID,Description,InstalledOn')
        LocalUsers = @('Win32_UserAccount','Name,Disabled,Lockout,SID','root/cimv2','LocalAccount=TRUE')
        LocalGroups = @('Win32_Group','Name,Description,SID','root/cimv2','LocalAccount=TRUE')
        Printers = @('Win32_Printer','Name,DriverName,PortName,Network,Default,PrinterStatus')
        Processes = @('Win32_Process','Name,ProcessId,ParentProcessId,ThreadCount,WorkingSetSize')
        Services = @('Win32_Service','Name,DisplayName,StartMode,State,ProcessId')
        Shares = @('Win32_Share','Name,Path,Type,Description')
        Environment = @('Win32_Environment','Name,UserName,SystemVariable')
        WindowsFeatures = @('Win32_OptionalFeature','Name,Caption,InstallState')
        Startup = @('Win32_StartupCommand','Name,Location,User')
        TaskSchedules = @('MSFT_ScheduledTask','TaskName,TaskPath,State','root/Microsoft/Windows/TaskScheduler')
    }
    $reports = [Collections.Generic.List[object]]::new()
    foreach ($item in $queries.GetEnumerator()) {
        $report = @{provider=$item.Key; state='Complete'; failure=$null; rows=@()}
        try {
            $properties = $item.Value[1] -split ','
            $query = @{CimSession=$session; ClassName=$item.Value[0]; Property=$properties; OperationTimeoutSec=6}
            if ($item.Value.Count -gt 2) { $query.Namespace = $item.Value[2] }
            if ($item.Value.Count -gt 3) { $query.Filter = $item.Value[3] }
            $rows = @(Get-CimInstance @query | Select-Object -Property $properties -First 513)
            if ($rows.Count -gt 512 -or $item.Key -in @('Environment','Startup')) {
                $report.state = 'Partial'; $report.failure = 'Unsupported'
            }
            $report.rows = @($rows | Select-Object -First 512)
        } catch {
            $report.state = 'Failed'
            $report.failure = switch ([string]$_.Exception.StatusCode) {
                'AccessDenied' { 'AccessDenied' }
                'InvalidClass' { 'Unsupported' }
                'InvalidNamespace' { 'Unsupported' }
                'OperationTimeout' { 'Timeout' }
                default { 'Unavailable' }
            }
        }
        $reports.Add($report)
    }

    try {
        $options = New-PSSessionOption -OpenTimeout 9000 -OperationTimeout 12000
        $software = Invoke-Command -ComputerName $inputData.host -Port 5986 -UseSSL `
            @credentialParameters -Authentication $inputData.winrm_authentication -SessionOption $options -ScriptBlock {
            $ErrorActionPreference = 'Stop'
            function Read-UninstallKeys($Branch, $Architecture, $UserSid) {
                if (-not (Test-Path -LiteralPath $Branch)) { return }
                Get-ChildItem -LiteralPath $Branch | ForEach-Object {
                    $row = Get-ItemProperty -LiteralPath $_.PSPath
                    if ($row.DisplayName) {
                        @{
                            name=[string]$row.DisplayName; publisher=[string]$row.Publisher
                            version=[string]$row.DisplayVersion; architecture=$Architecture
                            registry_key=[string]$_.PSChildName; user_sid=$UserSid
                        }
                    }
                }
            }
            $uninstall = 'Software\Microsoft\Windows\CurrentVersion\Uninstall'
            $nativeArchitecture = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'arm64' } elseif ([Environment]::Is64BitOperatingSystem) { 'x64' } else { 'x86' }
            $machine = @(& {
                Read-UninstallKeys ('HKLM:\' + $uninstall) $nativeArchitecture $null
                Read-UninstallKeys 'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall' 'x86' $null
            } | Select-Object -First 513)
            $users = @(& {
                foreach ($hive in (Get-ChildItem -LiteralPath 'Registry::HKEY_USERS')) {
                    if ($hive.PSChildName -match '^S-1-\d+(-\d+)+$') {
                        Read-UninstallKeys ($hive.PSPath + '\' + $uninstall) $null $hive.PSChildName
                    }
                }
            } | Select-Object -First 513)
            @{ machine = $machine; users = $users }
        }
        $reports.Add(@{provider='SoftwareMachine'; state=if ($software.machine.Count -gt 512) {'Partial'} else {'Complete'};
            failure=if ($software.machine.Count -gt 512) {'Unsupported'} else {$null}; rows=@($software.machine | Select-Object -First 512)})
        # Only already loaded hives were read. Absent rows cannot prove absence for all users.
        $reports.Add(@{provider='SoftwareUsers'; state='Partial'; failure='Unsupported'; rows=@($software.users | Select-Object -First 512)})
    } catch {
        foreach ($provider in @('SoftwareMachine','SoftwareUsers')) {
            $reports.Add(@{provider=$provider; state='Failed'; failure='Unavailable'; rows=@()})
        }
    }
    [Console]::Out.Write((ConvertTo-Json -InputObject @{failure=$null; providers=@($reports.ToArray())} -Depth 8 -Compress))
} catch { exit 1 } finally { if ($session) { Remove-CimSession $session -ErrorAction SilentlyContinue } }

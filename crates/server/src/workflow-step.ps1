$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$ProgressPreference = 'SilentlyContinue'
$result = @{state='BeforeStep';exit_code=$null;evidence=$null}
$session = $null
try {
    $data = [Console]::In.ReadToEnd() | ConvertFrom-Json
    $spec = $data.options
    $credentialParameters=@{}
    if (-not $data.use_server_identity) {
    $credential = [Management.Automation.PSCredential]::new($data.username,(ConvertTo-SecureString $data.password -AsPlainText -Force))
    $credentialParameters.Credential=$credential
    }
    $data.password = $null
    $session = New-PSSession -ComputerName $data.host -Port 5986 -UseSSL -Authentication $data.winrm_authentication @credentialParameters -SessionOption (New-PSSessionOption -OpenTimeout 10000 -OperationTimeout 30000)
    $step = $spec.step
    if ($spec.reconcile) {
        $result = Invoke-Command -Session $session -ArgumentList $step,$spec.evidence -ScriptBlock {
            param($Step,$Evidence)
            $ErrorActionPreference = 'Stop'
            $confirmed = $false
            if ($Step.kind -eq 'FileCopy' -and [IO.File]::Exists($Step.destination)) {
                $confirmed = (Get-FileHash -LiteralPath $Step.destination -Algorithm SHA256).Hash -ieq $Step.sha256
            } elseif ($Step.kind -eq 'Reboot' -and $Evidence) {
                $boot = (Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToUniversalTime().ToString('o')
                $confirmed = $boot -cne $Evidence
            } elseif ($Step.kind -eq 'Sleep') { $confirmed = $true }
            @{state=$(if($confirmed){'Succeeded'}else{'Unknown'});exit_code=$null;evidence=$null}
        }
    } else {
        $staged = $null
        if ($step.kind -eq 'FileCopy') {
            $staged = Invoke-Command -Session $session -ScriptBlock {
                $folder = Join-Path ([IO.Path]::GetTempPath()) ('OrkestrEMP-' + [Guid]::NewGuid().ToString('N'))
                $item = New-Item -ItemType Directory -Path $folder
                $acl = Get-Acl -LiteralPath $item.FullName
                $acl.SetAccessRuleProtection($true,$false)
                foreach ($sid in @([Security.Principal.WindowsIdentity]::GetCurrent().User,[Security.Principal.SecurityIdentifier]::new('S-1-5-18'))) {
                    $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($sid,'FullControl','ContainerInherit,ObjectInherit','None','Allow'))
                }
                Set-Acl -LiteralPath $folder -AclObject $acl
                Join-Path $folder 'payload.bin'
            }
            $transfer = Send-OrkestrFile -Source $spec.path -Destination $staged -Session $session -BytesPerSecond ([long]$spec.bytes_per_second)
        }
        $result.state = 'Unknown'
        $result = Invoke-Command -Session $session -ArgumentList $step,$staged -ScriptBlock {
            param($Step,[string]$Staged)
            $ErrorActionPreference = 'Stop'
            function Run-Process([string]$File,[string]$Arguments,[string]$InputText=$null) {
                $start = [Diagnostics.ProcessStartInfo]::new()
                $start.FileName=$File; $start.Arguments=$Arguments; $start.UseShellExecute=$false
                $start.CreateNoWindow=$true; $start.RedirectStandardOutput=$true; $start.RedirectStandardError=$true
                $start.RedirectStandardInput= -not [string]::IsNullOrEmpty($InputText)
                $start.EnvironmentVariables.Remove('PSModulePath')
                $process=[Diagnostics.Process]::new();$process.StartInfo=$start
                try {
                    if (-not $process.Start()) { throw 'Process start failed' }
                    $stdout=$process.StandardOutput.BaseStream.CopyToAsync([IO.Stream]::Null)
                    $stderr=$process.StandardError.BaseStream.CopyToAsync([IO.Stream]::Null)
                    if ($start.RedirectStandardInput) { $process.StandardInput.Write($InputText);$process.StandardInput.Close() }
                    $process.WaitForExit();$stdout.GetAwaiter().GetResult() | Out-Null;$stderr.GetAwaiter().GetResult() | Out-Null
                    return $process.ExitCode
                } finally { $process.Dispose() }
            }
            $code = 0; $evidence = $null
            switch ($Step.kind) {
                'Command' { $code = Run-Process $Step.executable $Step.arguments }
                'PowerShell' {
                    $powershell=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
                    $command='[Console]::InputEncoding=[Text.UTF8Encoding]::new($false);$ErrorActionPreference=''Stop'';$global:LASTEXITCODE=0;try{&([scriptblock]::Create([Console]::In.ReadToEnd()));if(-not $?){exit 1};exit $global:LASTEXITCODE}catch{exit 1}'
                    $code=Run-Process $powershell ('-NoProfile -NonInteractive -Command "' + $command + '"') $Step.script
                }
                'FileCopy' {
                    try {
                        if ((Get-FileHash -LiteralPath $Staged -Algorithm SHA256).Hash -ine $Step.sha256) { return @{state='BeforeStep';exit_code=$null;evidence=$null} }
                        $destination=[IO.Path]::GetFullPath($Step.destination)
                        $parent=Get-Item -LiteralPath ([IO.Path]::GetDirectoryName($destination)) -Force
                        $check=$parent
                        while ($null -ne $check) {
                            if (($check.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Reparse destination' }
                            $check=$check.Parent
                        }
                        if ([IO.File]::Exists($destination)) {
                            $item=Get-Item -LiteralPath $destination -Force
                            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or -not $Step.overwrite) { throw 'Destination exists' }
                        }
                        $temp=Join-Path $parent.FullName ('.orkestr-' + [Guid]::NewGuid().ToString('N'))
                        [IO.File]::Copy($Staged,$temp,$false)
                        if ([IO.File]::Exists($destination)) { [IO.File]::Replace($temp,$destination,$null) }
                        else { [IO.File]::Move($temp,$destination) }
                        if ((Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash -ine $Step.sha256) { throw 'Copy verification failed' }
                    } finally {
                        if ($Staged -and [IO.File]::Exists($Staged)) { [IO.File]::Delete($Staged); [IO.Directory]::Delete([IO.Path]::GetDirectoryName($Staged),$false) }
                    }
                }
                'Message' { $file=Join-Path $env:SystemRoot 'System32\msg.exe'; $code=Run-Process $file ([string]$Step.session_id + ' /time:' + [string]$Step.seconds + ' "' + $Step.text.Replace('"','') + '"') }
                'Reboot' {
                    $evidence=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToUniversalTime().ToString('o')
                    $code=Run-Process (Join-Path $env:SystemRoot 'System32\shutdown.exe') ('/r /t ' + [string]$Step.delay_seconds + ' /d p:4:1')
                    if ($code -eq 0) { return @{state='RebootRequested';exit_code=0;evidence=$evidence} }
                }
                'Logoff' { $code=Run-Process (Join-Path $env:SystemRoot 'System32\logoff.exe') ([string]$Step.session_id) }
                default { throw 'Unsupported step' }
            }
            $success = $code -eq 0
            if ($Step.kind -eq 'PowerShell' -and $code -eq 3010) { $success=$true }
            if ($Step.kind -eq 'Command') { $success = $Step.success_codes -contains $code }
            @{state=$(if($success -and $code -eq 3010){'RebootRequired'}elseif($success){'Succeeded'}else{'Failed'});exit_code=$code;evidence=$evidence}
        }
    }
    if ($transfer) { $result.transfer = @{bytes=$transfer.bytes;elapsed_ms=$transfer.elapsed_ms} }
} catch {} finally { if ($session) { Remove-PSSession $session -ErrorAction SilentlyContinue } }
[Console]::Out.Write(($result | ConvertTo-Json -Depth 4 -Compress))

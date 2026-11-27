$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$ProgressPreference = 'SilentlyContinue'
$result = @{outcome='BeforeInstall'; exit_code=$null; detection='Unknown'}
$session = $null
$staging = $null
$detect = {
    param([string]$ProductCode, [string]$Version, [string]$RegistryKey, [bool]$AllowUpgrade, $Components)
    $ErrorActionPreference = 'Stop'
    function Compare-Numeric([string]$Left,[string]$Right) {
        if ($Left.Length -gt 128 -or $Right.Length -gt 128) { return $null }
        $a=$Left.Trim().TrimStart('v').Split('.')
        $b=$Right.Trim().TrimStart('v').Split('.')
        if ($a.Length -gt 16 -or $b.Length -gt 16) { return $null }
        foreach ($part in @($a)+@($b)) { $parsed=[UInt64]0;if ($part -notmatch '^\d+$' -or -not [UInt64]::TryParse($part,[ref]$parsed)) { return $null } }
        for($i=0;$i -lt [Math]::Max($a.Length,$b.Length);$i++) {
            $x=[UInt64]0;$y=[UInt64]0;if($i -lt $a.Length){$x=[UInt64]$a[$i]};if($i -lt $b.Length){$y=[UInt64]$b[$i]}
            if($x -lt $y){return -1};if($x -gt $y){return 1}
        }
        return 0
    }
    $product = if ($RegistryKey) { $RegistryKey } else { ([Guid]$ProductCode).ToString('B') }
    $otherVersion = $false
    $upgradeable=$false;$blockedUpgrade=$false
    $views = @([Microsoft.Win32.RegistryView]::Registry32)
    if ([Environment]::Is64BitOperatingSystem) { $views += [Microsoft.Win32.RegistryView]::Registry64 }
    foreach ($view in $views) {
        $root = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine, $view)
        try {
            $key = $root.OpenSubKey('SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\' + $product)
            if ($null -eq $key) { continue }
            try {
                $installed=[string]$key.GetValue('DisplayVersion')
                if ($installed -ceq $Version) { return 'Present' }
                if ($Components -and $null -ne (Compare-Numeric $installed $installed)) {
                    $actualPrefix=($installed.Trim().TrimStart('v').Split('.') | Select-Object -First ([int]$Components)) -join '.'
                    $expectedPrefix=($Version.Trim().TrimStart('v').Split('.') | Select-Object -First ([int]$Components)) -join '.'
                    if ((Compare-Numeric $actualPrefix $expectedPrefix) -eq 0) { return 'Present' }
                }
                $comparison=Compare-Numeric $installed $Version
                if ($AllowUpgrade -and $null -ne $comparison -and $comparison -lt 0) { $upgradeable=$true } else { $blockedUpgrade=$true }
                $otherVersion = $true
            } finally { $key.Dispose() }
        } finally { $root.Dispose() }
    }
    if ($otherVersion) { if($upgradeable -and -not $blockedUpgrade){return 'Upgradeable'};return 'DifferentVersion' }
    return 'Absent'
}
try {
    $inputData = [Console]::In.ReadToEnd() | ConvertFrom-Json
    $spec = $inputData.options
    $extension = switch ($spec.execution.kind) {
        'Msi' { 'msi' }
        'Exe' {
            if ([string]::IsNullOrWhiteSpace($spec.execution.arguments) -or
                [Text.Encoding]::UTF8.GetByteCount($spec.execution.arguments) -gt 2048 -or
                $spec.execution.arguments -match '[\x00-\x1f\x7f-\x9f]') { throw 'Invalid arguments' }
            'exe'
        }
        default { throw 'Invalid installer kind' }
    }
    $credentialParameters=@{}
    if (-not $inputData.use_server_identity) {
    $securePassword = ConvertTo-SecureString $inputData.password -AsPlainText -Force
    $credential = [Management.Automation.PSCredential]::new($inputData.username, $securePassword)
    $credentialParameters.Credential=$credential
    }
    $inputData.password = $null
    $options = New-PSSessionOption -OpenTimeout 12000 -OperationTimeout 30000
    $session = New-PSSession -ComputerName $inputData.host -Port 5986 -UseSSL `
        @credentialParameters -Authentication $inputData.winrm_authentication -SessionOption $options
    $initial = [string](Invoke-Command -Session $session -ScriptBlock $detect -ArgumentList $spec.product_code,$spec.version,$spec.registry_key,([bool]$spec.allow_upgrade -and -not $spec.reconcile),$spec.version_components)
    $result.detection = if($initial -eq 'Upgradeable'){'DifferentVersion'}else{$initial}
    if ($spec.reconcile) {
        $result.outcome = 'DetectionOnly'
    } elseif ($result.detection -eq 'Present') {
        $result.outcome = 'AlreadyPresent'
    } elseif ($initial -eq 'Absent' -or $initial -eq 'Upgradeable') {
        $staging = Invoke-Command -Session $session -ScriptBlock {
            $ErrorActionPreference = 'Stop'
            $folder = Join-Path ([IO.Path]::GetTempPath()) ('OrkestrEMP-' + [Guid]::NewGuid().ToString('N'))
            $item = New-Item -ItemType Directory -Path $folder
            $acl = Get-Acl -LiteralPath $item.FullName
            $acl.SetAccessRuleProtection($true, $false)
            $owner = [Security.Principal.WindowsIdentity]::GetCurrent().User
            foreach ($sid in @($owner, [Security.Principal.SecurityIdentifier]::new('S-1-5-18'))) {
                $rule = [Security.AccessControl.FileSystemAccessRule]::new($sid, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow')
                $acl.AddAccessRule($rule)
            }
            Set-Acl -LiteralPath $item.FullName -AclObject $acl
            $item.FullName
        }
        $transfer = Send-OrkestrFile -Source $spec.path -Destination ([IO.Path]::Combine($staging, ('package.' + $extension))) -Session $session -BytesPerSecond ([long]$spec.bytes_per_second)
        $result.transfer_bytes = $transfer.bytes
        $result.transfer_ms = $transfer.elapsed_ms
        # From this point an interrupted transport can leave the installer running.
        $result.outcome = 'Indeterminate'
        $execution = Invoke-Command -Session $session -ArgumentList $staging,$spec.sha256,$extension,$spec.execution.arguments -ScriptBlock {
            param([string]$Folder, [string]$ExpectedHash, [string]$Extension, [string]$Arguments)
            $ErrorActionPreference = 'Stop'
            if ($Extension -notin @('msi', 'exe')) { throw 'Invalid installer kind' }
            $file = Join-Path $Folder ('package.' + $Extension)
            if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash -ine $ExpectedHash) {
                return @{verified=$false; exit_code=$null}
            }
            $start = [Diagnostics.ProcessStartInfo]::new()
            if ($Extension -eq 'msi') {
                $start.FileName = Join-Path $env:SystemRoot 'System32\msiexec.exe'
                $start.Arguments = '/i "' + $file + '" /qn /norestart REBOOT=ReallySuppress ALLUSERS=1'
            } else {
                $start.FileName = $file
                $start.Arguments = $Arguments
            }
            $start.WorkingDirectory = $Folder
            $start.UseShellExecute = $false
            $start.CreateNoWindow = $true
            $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
            $start.RedirectStandardOutput = $true
            $start.RedirectStandardError = $true
            $process = [Diagnostics.Process]::new()
            $process.StartInfo = $start
            try {
                if (-not $process.Start()) { throw 'Installer failed to start' }
                # Drain both streams without retaining installer output or blocking its pipes.
                $stdout = $process.StandardOutput.BaseStream.CopyToAsync([IO.Stream]::Null)
                $stderr = $process.StandardError.BaseStream.CopyToAsync([IO.Stream]::Null)
                $process.WaitForExit()
                $stdout.GetAwaiter().GetResult() | Out-Null
                $stderr.GetAwaiter().GetResult() | Out-Null
                @{verified=$true; exit_code=$process.ExitCode}
            } finally { $process.Dispose() }
        }
        if ($execution.verified) {
            $result.exit_code = [int]$execution.exit_code
            $result.detection = 'Unknown'
            $result.outcome = 'Finished'
            $result.detection = [string](Invoke-Command -Session $session -ScriptBlock $detect -ArgumentList $spec.product_code,$spec.version,$spec.registry_key,$false,$spec.version_components)
        } else {
            $result.outcome = 'BeforeInstall'
        }
    }
} catch {
    # The boundary in $result determines whether an installer might have been launched.
} finally {
    if ($session -and $staging -and $result.outcome -ne 'Indeterminate') {
        try {
            Invoke-Command -Session $session -ArgumentList $staging,$extension -ScriptBlock {
                param([string]$Folder, [string]$Extension)
                $file = Join-Path $Folder ('package.' + $Extension)
                if ([IO.File]::Exists($file)) { [IO.File]::Delete($file) }
                [IO.Directory]::Delete($Folder, $false)
            } | Out-Null
        } catch {}
    }
    if ($session) { Remove-PSSession $session -ErrorAction SilentlyContinue }
}
[Console]::Out.Write((ConvertTo-Json -InputObject $result -Depth 3 -Compress))

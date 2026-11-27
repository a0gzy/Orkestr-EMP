#Requires -RunAsAdministrator
[CmdletBinding(SupportsShouldProcess)]
param(
    [Parameter(Mandatory)][string]$DataDirectory,
    [Parameter(Mandatory)][System.Management.Automation.PSCredential]$ServiceCredential
)
$ErrorActionPreference = 'Stop'
$data = (Resolve-Path -LiteralPath $DataDirectory).Path
$exe = Join-Path $data 'orkestr-server.exe'
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
    throw 'Copy the built orkestr-server.exe into the protected data directory first.'
}
if (Get-Service -Name OrkestrEMP -ErrorAction SilentlyContinue) {
    throw 'OrkestrEMP already exists; this helper never overwrites an existing service.'
}
$account = New-Object Security.Principal.NTAccount($ServiceCredential.UserName)
$sid = $account.Translate([Security.Principal.SecurityIdentifier]).Value
$allowed = @($sid, 'S-1-5-18', 'S-1-5-32-544')
foreach ($item in @((Get-Item -LiteralPath $data)) + @(Get-ChildItem -LiteralPath $data -Force -Recurse)) {
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Reparse paths are forbidden.' }
    $acl = Get-Acl -LiteralPath $item.FullName
    if ($acl.GetOwner([Security.Principal.SecurityIdentifier]).Value -notin $allowed) { throw 'Unexpected ACL owner.' }
    if ($item.FullName -eq $data -and -not $acl.AreAccessRulesProtected) { throw 'Protect the directory ACL first.' }
    foreach ($rule in $acl.Access) {
        $owner = $rule.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value
        if ($rule.AccessControlType -ne 'Allow' -or $owner -notin $allowed) { throw 'Unexpected ACL principal; initialize as the intended service identity.' }
    }
}
if ($PSCmdlet.ShouldProcess('OrkestrEMP', 'Create automatic Windows service (does not start)')) {
    $binary = '"' + $exe + '" service --data "' + $data.TrimEnd('\') + '"'
    New-Service -Name OrkestrEMP -DisplayName 'Orkestr EMP Server' -BinaryPathName $binary `
        -StartupType Automatic -Credential $ServiceCredential -Description 'Orkestr EMP HTTPS backend; no deployment engine in M2.' | Out-Null
    Write-Output 'Service registered, not started. No firewall, domain, WinRM, trust-store or user-rights policy changed.'
}

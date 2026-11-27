# Fixed read-only probe, input only on stdin. Never log input or exception text.
$ErrorActionPreference='Stop'
[Console]::InputEncoding=[Text.UTF8Encoding]::new($false)
$ProgressPreference='SilentlyContinue'
try {
  $p=[Console]::In.ReadToEnd() | ConvertFrom-Json
  $credentialParameters=@{}
  if (-not $p.use_server_identity) {
  $secure=ConvertTo-SecureString $p.password -AsPlainText -Force
  $credential=New-Object Management.Automation.PSCredential($p.username,$secure)
  $credentialParameters.Credential=$credential
  }
  $p.password=$null
  if ($p.kind -eq 'AdRead') {
    Add-Type -AssemblyName System.DirectoryServices.Protocols
    $endpoint=New-Object System.DirectoryServices.Protocols.LdapDirectoryIdentifier($p.host,[int]$p.port,$true,$false)
    $ldap=New-Object System.DirectoryServices.Protocols.LdapConnection($endpoint)
    try {
      $ldap.Timeout=[TimeSpan]::FromSeconds(12)
      $ldap.AuthType=[DirectoryServices.Protocols.AuthType]::Negotiate
      $ldap.Credential=$credential.GetNetworkCredential()
      $ldap.SessionOptions.ProtocolVersion=3
      $ldap.SessionOptions.SecureSocketLayer=$true
      $ldap.SessionOptions.ReferralChasing=[DirectoryServices.Protocols.ReferralChasingOptions]::None
      $ldap.Bind()
      $request=New-Object DirectoryServices.Protocols.SearchRequest('', '(objectClass=*)', [DirectoryServices.Protocols.SearchScope]::Base, @('defaultNamingContext'))
      $response=$ldap.SendRequest($request)
      if ($response.Entries.Count -ne 1) { throw 'read' }
    } finally { $ldap.Dispose() }
  } elseif ($p.kind -eq 'Execution') {
    $option=New-CimSessionOption -UseSsl
    $session=New-CimSession -ComputerName $p.host -Port ([int]$p.port) @credentialParameters -Authentication $p.winrm_authentication -SessionOption $option -OperationTimeoutSec 12
    try {
      $os=Get-CimInstance -CimSession $session -ClassName Win32_OperatingSystem -Property Caption -OperationTimeoutSec 12
      if ($null -eq $os.Caption) { throw 'read' }
    } finally { Remove-CimSession $session -ErrorAction SilentlyContinue }
  } else { throw 'kind' }
  [Console]::Out.Write('READ_ONLY_OK')
} catch { [Console]::Out.Write('CONNECTION_TEST_FAILED'); exit 1 }

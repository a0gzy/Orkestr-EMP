$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$ProgressPreference = 'SilentlyContinue'
$ldap = $null
try {
    $inputData = [Console]::In.ReadToEnd() | ConvertFrom-Json
    $securePassword = ConvertTo-SecureString $inputData.password -AsPlainText -Force
    $credential = [Management.Automation.PSCredential]::new($inputData.username, $securePassword)
    $inputData.password = $null
    Add-Type -AssemblyName System.DirectoryServices.Protocols
    $endpoint = [DirectoryServices.Protocols.LdapDirectoryIdentifier]::new($inputData.host, 636, $true, $false)
    $ldap = [DirectoryServices.Protocols.LdapConnection]::new($endpoint)
    $ldap.Timeout = [TimeSpan]::FromSeconds(10)
    $ldap.AuthType = [DirectoryServices.Protocols.AuthType]::Negotiate
    $ldap.Credential = $credential.GetNetworkCredential()
    $ldap.SessionOptions.ProtocolVersion = 3
    $ldap.SessionOptions.SecureSocketLayer = $true
    $ldap.SessionOptions.ReferralChasing = [DirectoryServices.Protocols.ReferralChasingOptions]::None
    $ldap.Bind()

    $base = [string]$inputData.options
    $check = [DirectoryServices.Protocols.SearchRequest]::new($base, '(objectClass=organizationalUnit)', 'Base', [string[]]@('distinguishedName'))
    if ($ldap.SendRequest($check).Entries.Count -ne 1) { throw 'OU required' }
    $query = [DirectoryServices.Protocols.SearchRequest]::new($base,
        '(&(objectCategory=computer)(dNSHostName=*))', 'Subtree',
        [string[]]@('objectGUID','dNSHostName','name','operatingSystem','userAccountControl'))
    $page = [DirectoryServices.Protocols.PageResultRequestControl]::new(200)
    [void]$query.Controls.Add($page)
    $computers = [Collections.Generic.List[object]]::new()
    do {
        $response = $ldap.SendRequest($query)
        if ($response.References.Count -ne 0) { throw 'Unexpected referral' }
        foreach ($entry in $response.Entries) {
            if ($computers.Count -ge 1000) { throw 'OU limit exceeded' }
            $guid = [Guid]::new([byte[]]$entry.Attributes['objectGUID'][0])
            $computers.Add(@{
                guid = $guid.ToString()
                host = [string]$entry.Attributes['dNSHostName'][0]
                name = [string]$entry.Attributes['name'][0]
                ou = $entry.DistinguishedName -replace '^(?:\\.|[^\\,])+,'
                os = if ($entry.Attributes.Contains('operatingSystem')) { [string]$entry.Attributes['operatingSystem'][0] } else { '' }
                disabled = (([int]$entry.Attributes['userAccountControl'][0] -band 2) -ne 0)
            })
        }
        $pages = @($response.Controls | Where-Object { $_ -is [DirectoryServices.Protocols.PageResultResponseControl] })
        if ($pages.Count -ne 1) { throw 'Incomplete paginated query' }
        $page.Cookie = $pages[0].Cookie
    } while ($page.Cookie.Length -ne 0)
    [Console]::Out.Write((ConvertTo-Json -InputObject @($computers.ToArray()) -Depth 4 -Compress))
} catch { exit 1 } finally { if ($ldap) { $ldap.Dispose() } }

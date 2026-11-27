$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
try {
    $domain = [Console]::In.ReadToEnd()
    $suffix = '.' + $domain
    $records = Resolve-DnsName -Name ('_ldap._tcp.dc._msdcs.' + $domain + '.') -Type SRV -DnsOnly -QuickTimeout
    $targets = $records | Where-Object { $_.Type -eq 'SRV' -and $_.Port -eq 389 -and $_.NameTarget.TrimEnd('.').EndsWith($suffix, [StringComparison]::OrdinalIgnoreCase) } |
        Sort-Object Priority,NameTarget | Select-Object -First 4
    foreach ($record in $targets) {
        $server = $record.NameTarget.TrimEnd('.')
        $tcp = [Net.Sockets.TcpClient]::new()
        $tls = $null
        try {
            if (-not $tcp.ConnectAsync($server, 636).Wait(1200)) { continue }
            $tls = [Net.Security.SslStream]::new($tcp.GetStream(), $false)
            if ($tls.AuthenticateAsClientAsync($server).Wait(1200)) {
                [Console]::Out.Write($server)
                exit 0
            }
        } catch {} finally {
            if ($tls) { $tls.Dispose() }
            $tcp.Dispose()
        }
    }
    exit 1
} catch { exit 1 }

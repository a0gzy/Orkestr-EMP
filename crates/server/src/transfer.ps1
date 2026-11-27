# Explicit chunk transfer. The caller holds a global/site transfer reservation.
# Byte rate measures payload bytes, not WinRM/TLS/base64 wire overhead.
function Send-OrkestrFile {
    param([string]$Source,[string]$Destination,$Session,[long]$BytesPerSecond)
    $ErrorActionPreference='Stop'
    Invoke-Command -Session $Session -ArgumentList $Destination -ScriptBlock {
        param([string]$Path)
        $stream=[IO.File]::Open($Path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
        $stream.Dispose()
    } | Out-Null
    $inputStream=[IO.File]::Open($Source,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    $clock=[Diagnostics.Stopwatch]::StartNew()
    $sent=0L
    try {
        $buffer=[byte[]]::new(65536)
        while (($count=$inputStream.Read($buffer,0,$buffer.Length)) -gt 0) {
            if ($BytesPerSecond -gt 0) {
                $due=([double]($sent+$count)*1000.0)/$BytesPerSecond
                while ($clock.Elapsed.TotalMilliseconds -lt $due) {
                    Start-Sleep -Milliseconds ([int][Math]::Min(50,[Math]::Max(1,$due-$clock.Elapsed.TotalMilliseconds)))
                }
            }
            $chunk=[byte[]]::new($count)
            [Array]::Copy($buffer,$chunk,$count)
            $ack=Invoke-Command -Session $Session -ArgumentList $Destination,$sent,$chunk -ScriptBlock {
                param([string]$Path,[long]$Offset,[byte[]]$Chunk)
                $stream=[IO.File]::Open($Path,[IO.FileMode]::Open,[IO.FileAccess]::Write,[IO.FileShare]::None)
                try {
                    if ($stream.Length -ne $Offset) { throw 'Transfer offset mismatch' }
                    $stream.Position=$Offset
                    $stream.Write($Chunk,0,$Chunk.Length)
                    $stream.Flush($true)
                    $stream.Position
                } finally { $stream.Dispose() }
            }
            $sent+=$count
            if ([long]$ack -ne $sent) { throw 'Transfer acknowledgement mismatch' }
        }
    } finally { $inputStream.Dispose();$clock.Stop() }
    [pscustomobject]@{bytes=$sent;elapsed_ms=$clock.ElapsedMilliseconds}
}

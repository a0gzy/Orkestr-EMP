$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
try {
    $spec = [Console]::In.ReadToEnd() | ConvertFrom-Json
    $signature = Get-AuthenticodeSignature -LiteralPath $spec.path
    if ($signature.Status -ne 'Valid' -or $null -eq $signature.SignerCertificate -or
        $signature.SignerCertificate.Subject -cne $spec.publisher) { exit 1 }
    [Console]::Out.Write('SIGNATURE_VERIFIED')
} catch { exit 1 }

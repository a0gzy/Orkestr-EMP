[CmdletBinding()]
param([string]$OutputDirectory='')
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
if (-not $OutputDirectory) { $OutputDirectory=Join-Path $root ('test-results\native-'+[guid]::NewGuid().ToString()) }
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Choose a NEW smoke output directory.' }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$OutputDirectory=(Resolve-Path -LiteralPath $OutputDirectory).Path
$copies=Join-Path $OutputDirectory 'synthetic-copies'
& py -3 (Join-Path $PSScriptRoot 'create-m1-fixtures.py') --out $copies
if ($LASTEXITCODE -ne 0) { throw 'Synthetic fixture creation failed.' }
$cli=Join-Path $root 'target\debug\orkestr-cli.exe'
$console=Join-Path $root 'target\debug\orkestr-console.exe'
if (-not (Test-Path $cli) -or -not (Test-Path $console)) { throw 'Build workspace first.' }
$database=Join-Path $OutputDirectory 'fixture-orkestr.sqlite'
& $cli init $database
if ($LASTEXITCODE -ne 0) { throw 'Fixture init failed.' }
foreach ($source in @(@('inventory','inventory-copy.sqlite'),@('deploy','console-copy.json'))) {
    $copy=Join-Path $copies $source[1]
    $before=(Get-FileHash -LiteralPath $copy -Algorithm SHA256).Hash
    & $cli import $source[0] $database $copy --apply (Join-Path $OutputDirectory ('before-'+$source[0]+'.sqlite'))
    if ($LASTEXITCODE -ne 0) { throw 'Fixture import failed.' }
    if ($before -ne (Get-FileHash -LiteralPath $copy -Algorithm SHA256).Hash) { throw 'Source copy changed.' }
}
Get-ChildItem (Join-Path $copies 'groups') -Filter '*.json' | Sort-Object Name | ForEach-Object {
    & $cli save-group $database $_.FullName new
    if ($LASTEXITCODE -ne 0) { throw 'Fixture group save failed.' }
}
foreach ($scenario in @(@('light','overview'),@('dark','overview'),@('light','editor'),@('light','device'),@('dark','packages'),@('light','import'),@('light','group'),@('dark','review'),@('light','editor','2.0'),@('dark','settings','2.0'))) {
    $suffix=''
    if ($scenario.Count -eq 3) { $suffix='-zoom'+$scenario[2] }
    $png=Join-Path $OutputDirectory ($scenario[0]+'-'+$scenario[1]+$suffix+'.png')
    $arguments=@('--database',('"'+$database+'"'),'--theme',$scenario[0],'--capture',('"'+$png+'"'),'--capture-view',$scenario[1])
    if ($scenario.Count -eq 3) { $arguments+=@('--zoom',$scenario[2]) }
    $process=Start-Process -FilePath $console -ArgumentList $arguments -PassThru
    if (-not $process.WaitForExit(45000)) { $process.Kill(); throw 'Native smoke timed out; its own process was stopped.' }
    if ($process.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $png)) { throw 'Native screenshot failed.' }
}
& $cli list $database | Out-File -LiteralPath (Join-Path $OutputDirectory 'restart-list.txt') -Encoding utf8
if ($LASTEXITCODE -ne 0) { throw 'Restart list failed.' }
Write-Output ('Native fixture smoke completed. Review screenshots: '+$OutputDirectory)

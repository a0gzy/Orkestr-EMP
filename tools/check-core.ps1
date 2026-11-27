[CmdletBinding()]
param(
    # Empty: normal system Rust/MSVC. GNU selection never changes rustup defaults.
    [switch]$Gnu,
    [string]$CompilerBin = ''
)
$ErrorActionPreference = 'Stop'
$oldPath = $env:PATH
Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    if ($Gnu -and -not $CompilerBin -and -not (Get-Command gcc -ErrorAction SilentlyContinue)) {
        $localCompiler = Join-Path $env:LOCALAPPDATA 'OrkestrEMP\toolchains\w64devkit\bin'
        if (Test-Path -LiteralPath (Join-Path $localCompiler 'gcc.exe') -PathType Leaf) {
            $CompilerBin = $localCompiler
        } else {
            throw 'GNU GCC was not found. Supply -CompilerBin <external-GCC-bin> or use an installed MSVC toolchain without -Gnu.'
        }
    }
    if ($CompilerBin) {
        if (-not (Test-Path -LiteralPath $CompilerBin -PathType Container)) { throw 'CompilerBin must exist.' }
        $env:PATH = $CompilerBin + ';' + $env:PATH
    }
    $toolchain = @()
    if ($Gnu) { $toolchain = @('+stable-x86_64-pc-windows-gnu') }
    & cargo @toolchain fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw 'Formatting check failed.' }
    & cargo @toolchain clippy --workspace --all-targets --locked -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'Clippy failed.' }
    & cargo @toolchain test --workspace --locked
    if ($LASTEXITCODE -ne 0) { throw 'Core tests failed.' }
    & cargo @toolchain build --workspace --locked
    if ($LASTEXITCODE -ne 0) { throw 'Build failed.' }
} finally {
    $env:PATH = $oldPath
    Pop-Location
}

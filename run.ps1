#Requires -Version 7.0
<#
.SYNOPSIS
    Convenience wrapper: builds and runs the ghlinks collector.

    This script is optional. The compiled binary at
    target\release\ghlinks.exe (or target/release/ghlinks on macOS/Linux)
    is the actual program and can be run directly with --input/--output/etc.
    This wrapper exists for two things a plain `cargo run` doesn't give you:
    a secure token prompt (so a GitHub token never has to be typed as a
    plain command-line argument) and a build-then-run convenience path.

.EXAMPLE
    ./run.ps1 -InputFile .\links.txt -OutputFile report.json

.EXAMPLE
    ./run.ps1 -InputFile .\links.txt -SkipExternal -Concurrency 5
#>
param(
    [Parameter(Mandatory = $true)]
    [string]$InputFile,

    # A relative path here (including the default) is always resolved
    # against $InputFile's directory, not the current working directory —
    # so the report lands next to your input file regardless of where you
    # launched this script from. Pass an absolute path to override that
    # and write somewhere else on purpose.
    [string]$OutputFile = "ghlinks-report.json",

    [int]$Concurrency = 3,

    [int]$DelayMs = 250,

    [int]$TimeoutSecs = 30,

    [int]$MaxRetries = 3,

    [switch]$SkipExternal,

    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

if ($Concurrency -lt 1) { throw "Concurrency must be at least 1." }
if ($DelayMs -lt 0) { throw "DelayMs cannot be negative." }
if ($TimeoutSecs -lt 1) { throw "TimeoutSecs must be at least 1." }
if ($MaxRetries -lt 1) { throw "MaxRetries must be at least 1." }
if (-not (Test-Path -LiteralPath $InputFile -PathType Leaf)) {
    throw "InputFile must be an existing file: $InputFile"
}
$InputFile = (Resolve-Path -LiteralPath $InputFile).Path

if ([System.IO.Path]::IsPathRooted($OutputFile)) {
    # Absolute path given explicitly — respect it as a deliberate override.
    $OutputFile = [System.IO.Path]::GetFullPath($OutputFile)
} else {
    # Relative (including the default) — always anchor to the input
    # file's own directory, never to whatever directory this script
    # happened to be launched from (e.g. C:\Windows\System32 when
    # PowerShell is opened "as Administrator").
    $inputDir = Split-Path -Parent $InputFile
    $OutputFile = [System.IO.Path]::GetFullPath((Join-Path $inputDir $OutputFile))
}

if (-not $env:GITHUB_TOKEN) {
    Write-Host "No GITHUB_TOKEN found in the environment." -ForegroundColor Yellow
    Write-Host "Create one (no scopes needed for public data) at https://github.com/settings/tokens" -ForegroundColor Yellow
    $secure = Read-Host "Paste a GitHub personal access token" -AsSecureString
    $bstr = [System.Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
    try {
        $env:GITHUB_TOKEN = [System.Runtime.InteropServices.Marshal]::PtrToStringAuto($bstr)
    }
    finally {
        [System.Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr)
    }
}

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "Rust/cargo not found on PATH. Install from https://rustup.rs and re-run."
}

if (-not $SkipBuild) {
    Write-Host "Building release binary (first build will download crates; needs internet)..." -ForegroundColor Cyan
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed." }
}

$exe = Join-Path $PSScriptRoot "target\release\ghlinks.exe"
if (-not (Test-Path $exe)) {
    throw "Built binary not found at $exe. Run without -SkipBuild first."
}

$argsList = @(
    "--input", $InputFile,
    "--output", $OutputFile,
    "--concurrency", $Concurrency,
    "--delay-ms", $DelayMs,
    "--timeout-secs", $TimeoutSecs,
    "--max-retries", $MaxRetries
)
if ($SkipExternal) { $argsList += "--skip-external" }

Write-Host "Running ghlinks against $InputFile ..." -ForegroundColor Cyan
& $exe @argsList

$exitCode = $LASTEXITCODE

if ($exitCode -eq 0) {
    Write-Host "Done. Report written to $OutputFile" -ForegroundColor Green
} else {
    throw "ghlinks exited with code $exitCode"
}
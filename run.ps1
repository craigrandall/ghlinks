#Requires -Version 7.0
<#
.SYNOPSIS
    Builds and runs the ghlinks collector.

.EXAMPLE
    ./run.ps1 -InputFile .\links.txt -OutputFile .\report.json

.EXAMPLE
    ./run.ps1 -InputFile .\links.txt -SkipExternal -Concurrency 5
#>
param(
    [Parameter(Mandatory = $true)]
    [string]$InputFile,

    [string]$OutputFile = "ghlinks-report.json",

    [int]$Concurrency = 3,

    [int]$DelayMs = 250,

    [switch]$SkipExternal,

    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

if ($Concurrency -lt 1) { throw "Concurrency must be at least 1." }
if ($DelayMs -lt 0) { throw "DelayMs cannot be negative." }
if (-not (Test-Path -LiteralPath $InputFile -PathType Leaf)) {
    throw "InputFile must be an existing file: $InputFile"
}
$InputFile = (Resolve-Path -LiteralPath $InputFile).Path
$OutputFile = [System.IO.Path]::GetFullPath($OutputFile)

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
    "--delay-ms", $DelayMs
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

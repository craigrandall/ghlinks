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

# Fail fast on the first error rather than continuing with a partially
# valid state (e.g. building successfully but then running with a bad flag).
$ErrorActionPreference = "Stop"

# Validate CLI-facing values before doing anything expensive (build, token
# prompt). These mirror the constraints the compiled binary itself enforces
# on --concurrency/--delay-ms/--timeout-secs/--max-retries; catching them
# here gives a faster, PowerShell-native error instead of a cargo/binary
# failure several steps later.
if ($Concurrency -lt 1) { throw "Concurrency must be at least 1." }
if ($DelayMs -lt 0) { throw "DelayMs cannot be negative." }
if ($TimeoutSecs -lt 1) { throw "TimeoutSecs must be at least 1." }
if ($MaxRetries -lt 1) { throw "MaxRetries must be at least 1." }
if (-not (Test-Path -LiteralPath $InputFile -PathType Leaf)) {
    throw "InputFile must be an existing file: $InputFile"
}
# Resolve to an absolute path now so every downstream use (output-path
# anchoring, the binary invocation itself) is unambiguous regardless of
# what the working directory is by the time it matters.
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

# Secure entry, not secure storage: -AsSecureString keeps the token out of
# PowerShell's plaintext command history and off-screen while typing, but
# it still ends up as a plaintext env var (GITHUB_TOKEN) for the child
# process below, since that's the only way to hand a token to the compiled
# binary. Don't mistake this block for token-at-rest protection.
if (-not $env:GITHUB_TOKEN) {
    Write-Host "No GITHUB_TOKEN found in the environment." -ForegroundColor Yellow
    Write-Host "Create one (no scopes needed for public data) at https://github.com/settings/tokens" -ForegroundColor Yellow
    $secure = Read-Host "Paste a GitHub personal access token" -AsSecureString
    # SecureString itself can't be handed to a child process's environment
    # directly — it has to be decrypted to a plain string first. The
    # BSTR round-trip is the standard .NET pattern for that; ZeroFreeBSTR
    # in `finally` scrubs the decrypted copy from memory as soon as it's
    # been read into $env:GITHUB_TOKEN, rather than leaving it sitting in
    # an unmanaged buffer for the rest of the script's run.
    $bstr = [System.Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
    try {
        $env:GITHUB_TOKEN = [System.Runtime.InteropServices.Marshal]::PtrToStringAuto($bstr)
    }
    finally {
        [System.Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr)
    }
}

# Checked even when -SkipBuild is set: an old binary can still exist
# without cargo being installed on this machine, but we'd rather fail here
# with a clear message than let a later step fail more confusingly.
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "Rust/cargo not found on PATH. Install from https://rustup.rs and re-run."
}

if (-not $SkipBuild) {
    Write-Host "Building release binary (first build will download crates; needs internet)..." -ForegroundColor Cyan
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed." }
}

# $PSScriptRoot (not the current working directory) anchors this, so the
# script finds its own project's binary regardless of where it was
# invoked from. Windows-only path/extension by design — this wrapper is
# the PowerShell-specific convenience path; macOS/Linux users invoke
# target/release/ghlinks directly (see README).
$exe = Join-Path $PSScriptRoot "target\release\ghlinks.exe"
if (-not (Test-Path $exe)) {
    # Most likely cause: -SkipBuild was passed but no prior build exists
    # at this path yet (e.g. first run, or a `cargo clean` since the last
    # one). The message says so rather than just reporting "not found."
    throw "Built binary not found at $exe. Run without -SkipBuild first."
}

# One-to-one mapping onto the binary's own CLI flags (see `--help` on the
# binary, or README's "Build & run" section) — this wrapper doesn't add or
# rename anything, it just gives these the same names as PowerShell
# parameters and fills in GITHUB_TOKEN via the environment instead of a flag.
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

# $LASTEXITCODE reflects the native binary's exit code, not this script's
# own success — capture it immediately, since any further PowerShell
# cmdlet call between `& $exe` and this line could overwrite it.
$exitCode = $LASTEXITCODE

if ($exitCode -eq 0) {
    Write-Host "Done. Report written to $OutputFile" -ForegroundColor Green
} else {
    # Deliberately re-thrown rather than swallowed, so a caller scripting
    # against this wrapper (e.g. in CI) sees a non-zero exit and a
    # non-zero PowerShell error, not just a printed message.
    throw "ghlinks exited with code $exitCode"
}
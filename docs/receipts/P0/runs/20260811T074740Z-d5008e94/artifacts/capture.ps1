[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$env:CARGO_TERM_COLOR = 'never'
$env:NO_COLOR = '1'

$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
Set-Location -LiteralPath $repositoryRoot
$frozenCommit = '4354a4ec5cefdb2c7b462562991a33686969778e'
$frozenTree = 'eb400714687589e6a7e6a4395b5d43bb6333501a'
$contractPath = Join-Path $repositoryRoot 'docs\rebuild-contract.md'
$captureScriptPath = $MyInvocation.MyCommand.Path

$startedAt = (Get-Date).ToUniversalTime()
$runId = $startedAt.ToString('yyyyMMddTHHmmssZ') + '-' +
    [guid]::NewGuid().ToString('N').Substring(0, 8)
$runsRoot = Join-Path $PSScriptRoot 'runs'
$preexistingRunFiles = [Collections.Generic.Dictionary[string,string]]::new(
    [StringComparer]::Ordinal
)
$runRoot = Join-Path $runsRoot $runId
$commandsRoot = Join-Path $runRoot 'commands'
$artifactsRoot = Join-Path $runRoot 'artifacts'
$targetRoot = Join-Path ([IO.Path]::GetTempPath()) (
    'p0-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
)
$scratchRoot = Join-Path $targetRoot 'capture-scratch'

$utf8NoBom = [Text.UTF8Encoding]::new($false, $true)
$results = [Collections.Generic.List[object]]::new()
$artifacts = [Collections.Generic.List[object]]::new()
$overallStatus = 'pass'
$failedCommand = $null
$failureMessage = $null
$failureStage = 'initialization'
$contractHashStart = $null
$contractHashEnd = $null
$captureScriptHashStart = $null
$captureScriptHashEnd = $null
$targetCacheCleaned = $false
$preexistingEvidenceUnchanged = $false
$secondaryFailures = [Collections.Generic.List[string]]::new()

function Write-Utf8Lf {
    param(
        [Parameter(Mandatory)][string]$Path,
        [AllowEmptyString()][string]$Text
    )

    if ($null -eq $Text) {
        $Text = ''
    }
    $Text = $Text.Replace("`r`n", "`n").Replace("`r", "`n")
    if ($Text.Length -gt 0 -and -not $Text.EndsWith("`n")) {
        $Text += "`n"
    }
    [IO.File]::WriteAllText($Path, $Text, $utf8NoBom)
}

function Get-Sha256 {
    param([Parameter(Mandatory)][string]$Path)

    return (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
}

function Get-TextSha256 {
    param([AllowEmptyString()][string]$Text)

    if ($null -eq $Text) {
        $Text = ''
    }
    $hasher = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = $utf8NoBom.GetBytes($Text)
        $digest = [BitConverter]::ToString($hasher.ComputeHash($bytes))
        return $digest.Replace('-', '').ToLowerInvariant()
    }
    finally {
        $hasher.Dispose()
    }
}

function Protect-Text {
    param([AllowEmptyString()][string]$Text)

    if ($null -eq $Text) {
        $Text = ''
    }
    $rules = @(
        [ordered]@{ value = $repositoryRoot; token = '${REPO}' },
        [ordered]@{ value = $targetRoot; token = '${TEMP}' },
        [ordered]@{ value = $env:CARGO_HOME; token = '${CARGO_HOME}' },
        [ordered]@{ value = $env:RUSTUP_HOME; token = '${RUSTUP_HOME}' },
        [ordered]@{ value = $env:TEMP; token = '${OS_TEMP}' },
        [ordered]@{ value = $env:TMP; token = '${OS_TMP}' },
        [ordered]@{ value = $env:USERPROFILE; token = '${USERPROFILE}' }
    )
    $redactions = [Collections.Generic.List[object]]::new()
    foreach ($rule in $rules) {
        $count = 0
        if ($rule.value) {
            $variants = @($rule.value, $rule.value.Replace('\', '/')) | Select-Object -Unique
            foreach ($variant in $variants) {
                $regex = [regex]::new(
                    [regex]::Escape($variant),
                    [Text.RegularExpressions.RegexOptions]::IgnoreCase
                )
                $matches = $regex.Matches($Text).Count
                if ($matches -gt 0) {
                    $Text = $regex.Replace($Text, $rule.token)
                    $count += $matches
                }
            }
        }
        $redactions.Add([ordered]@{
            token = $rule.token
            replacement_count = $count
        })
    }
    return [pscustomobject]@{
        text = $Text.Replace("`r`n", "`n").Replace("`r", "`n")
        redactions = $redactions
    }
}

function Protect-Argument {
    param([Parameter(Mandatory)][string]$Value)

    return (Protect-Text $Value).text.TrimEnd("`n")
}

function Add-CommandResult {
    param(
        [Parameter(Mandatory)][string]$Id,
        [Parameter(Mandatory)][string[]]$Argv,
        [Parameter(Mandatory)][int]$ExitCode,
        [Parameter(Mandatory)][long]$DurationMs,
        [Parameter(Mandatory)][object]$ProtectedStdout,
        [Parameter(Mandatory)][object]$ProtectedStderr,
        [string]$Source = 'executed'
    )

    $stdoutRelative = "commands/$Id.stdout.txt"
    $stderrRelative = "commands/$Id.stderr.txt"
    $stdoutPath = Join-Path $runRoot $stdoutRelative
    $stderrPath = Join-Path $runRoot $stderrRelative
    Write-Utf8Lf -Path $stdoutPath -Text $ProtectedStdout.text
    Write-Utf8Lf -Path $stderrPath -Text $ProtectedStderr.text
    $safeArgv = @($Argv | ForEach-Object { Protect-Argument $_ })
    $results.Add([ordered]@{
        id = $Id
        source = $Source
        argv = $safeArgv
        cwd = '.'
        exit_code = $ExitCode
        duration_ms = $DurationMs
        status = if ($ExitCode -eq 0) { 'pass' } else { 'fail' }
        stdout = [ordered]@{
            path = $stdoutRelative
            sha256 = Get-Sha256 $stdoutPath
            bytes = (Get-Item -LiteralPath $stdoutPath).Length
            hash_basis = 'normalized_utf8_lf'
            redactions = $ProtectedStdout.redactions
        }
        stderr = [ordered]@{
            path = $stderrRelative
            sha256 = Get-Sha256 $stderrPath
            bytes = (Get-Item -LiteralPath $stderrPath).Length
            hash_basis = 'normalized_utf8_lf'
            redactions = $ProtectedStderr.redactions
        }
    })
}

function Invoke-NativeCapture {
    param(
        [Parameter(Mandatory)][string]$Id,
        [Parameter(Mandatory)][string]$File,
        [Parameter(Mandatory)][string[]]$Arguments
    )

    foreach ($argument in $Arguments) {
        if ($argument -match '[\s"]') {
            throw "$Id has an argument requiring unsupported command-line quoting"
        }
    }
    $stopwatch = [Diagnostics.Stopwatch]::StartNew()
    $stdout = ''
    $stderr = ''
    $exitCode = -1
    $process = $null
    try {
        $startInfo = [Diagnostics.ProcessStartInfo]::new()
        $startInfo.FileName = $File
        $startInfo.Arguments = $Arguments -join ' '
        $startInfo.WorkingDirectory = $repositoryRoot
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true
        $startInfo.StandardOutputEncoding = $utf8NoBom
        $startInfo.StandardErrorEncoding = $utf8NoBom
        $process = [Diagnostics.Process]::new()
        $process.StartInfo = $startInfo
        if (-not $process.Start()) {
            throw "failed to start $File"
        }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        $stdout = $stdoutTask.Result
        $stderr = $stderrTask.Result
        $exitCode = $process.ExitCode
    }
    catch {
        $stderr += $_.Exception.Message
    }
    finally {
        if ($null -ne $process) {
            $process.Dispose()
        }
        $stopwatch.Stop()
    }
    $protectedStdout = Protect-Text $stdout
    $protectedStderr = Protect-Text $stderr
    Add-CommandResult -Id $Id -Argv (@($File) + $Arguments) -ExitCode $exitCode `
        -DurationMs $stopwatch.ElapsedMilliseconds -ProtectedStdout $protectedStdout `
        -ProtectedStderr $protectedStderr
    if ($exitCode -ne 0) {
        $script:failedCommand = $Id
        throw "$Id failed with exit code $exitCode"
    }
}

function Invoke-JsonValidationCapture {
    param([Parameter(Mandatory)][string]$Id)

    $stopwatch = [Diagnostics.Stopwatch]::StartNew()
    $names = [Collections.Generic.List[string]]::new()
    $exitCode = 0
    $stderr = ''
    try {
        Get-ChildItem -LiteralPath . -Filter '*.example.json' -File |
            Sort-Object Name |
            ForEach-Object {
                $json = [IO.File]::ReadAllText($_.FullName, $utf8NoBom)
                [void]($json | ConvertFrom-Json)
                $names.Add($_.Name)
            }
    }
    catch {
        $exitCode = 1
        $stderr = $_.Exception.Message
    }
    finally {
        $stopwatch.Stop()
    }
    $argv = @('in-process', 'parse-sorted-root-example-json', 'strict-utf8')
    Add-CommandResult -Id $Id -Argv $argv -ExitCode $exitCode `
        -DurationMs $stopwatch.ElapsedMilliseconds `
        -ProtectedStdout (Protect-Text ($names -join "`n")) `
        -ProtectedStderr (Protect-Text $stderr) -Source 'executed_in_capture_process'
    if ($exitCode -ne 0) {
        $script:failedCommand = $Id
        throw "$Id failed"
    }
}

function Add-DerivedArtifact {
    param(
        [Parameter(Mandatory)][string]$SourceRelative,
        [Parameter(Mandatory)][string]$DestinationName,
        [Parameter(Mandatory)][string]$DerivedFrom
    )

    $sourcePath = Join-Path $runRoot $SourceRelative
    $destinationRelative = "artifacts/$DestinationName"
    $destinationPath = Join-Path $runRoot $destinationRelative
    [IO.File]::Copy($sourcePath, $destinationPath, $false)
    $sourceHash = Get-Sha256 $sourcePath
    $destinationHash = Get-Sha256 $destinationPath
    if ($sourceHash -ne $destinationHash) {
        throw "copied artifact hash mismatch for $DestinationName"
    }
    $artifacts.Add([ordered]@{
        path = $destinationRelative
        sha256 = $destinationHash
        bytes = (Get-Item -LiteralPath $destinationPath).Length
        hash_basis = 'normalized_utf8_lf'
        derived_from_command = $DerivedFrom
    })
}

function Add-InputSnapshot {
    param(
        [Parameter(Mandatory)][string]$SourcePath,
        [Parameter(Mandatory)][string]$DestinationName,
        [Parameter(Mandatory)][string]$ExpectedSha256,
        [Parameter(Mandatory)][string]$SourceIdentity
    )

    $destinationRelative = "artifacts/$DestinationName"
    $destinationPath = Join-Path $runRoot $destinationRelative
    [IO.File]::Copy($SourcePath, $destinationPath, $false)
    $destinationHash = Get-Sha256 $destinationPath
    if ($destinationHash -ne $ExpectedSha256) {
        throw "input snapshot hash mismatch for $DestinationName"
    }
    $artifacts.Add([ordered]@{
        path = $destinationRelative
        sha256 = $destinationHash
        bytes = (Get-Item -LiteralPath $destinationPath).Length
        hash_basis = 'raw_file_bytes'
        derived_from_command = $null
        source_identity = $SourceIdentity
    })
}

function Write-RunSeal {
    $sealPath = Join-Path $runRoot 'SHA256SUMS'
    $lines = [Collections.Generic.List[string]]::new()
    Get-ChildItem -Recurse -File -LiteralPath $runRoot |
        Where-Object { $_.FullName -ne $sealPath } |
        ForEach-Object {
            $relative = $_.FullName.Substring($runRoot.Length + 1).Replace('\', '/')
            [pscustomobject]@{ path = $relative; full_path = $_.FullName }
        } |
        Sort-Object path |
        ForEach-Object {
            $lines.Add("$(Get-Sha256 $_.full_path)  $($_.path)")
    }
    Write-Utf8Lf -Path $sealPath -Text ($lines -join "`n")
    $verified = 0
    foreach ($line in [IO.File]::ReadAllLines($sealPath, $utf8NoBom)) {
        if ($line -notmatch '^([0-9a-f]{64})  (.+)$') {
            throw 'invalid SHA256SUMS line after write'
        }
        $expected = $Matches[1]
        $relative = $Matches[2]
        $sealedPath = Join-Path $runRoot $relative.Replace('/', '\')
        if (-not (Test-Path -LiteralPath $sealedPath) -or
            (Get-Sha256 $sealedPath) -ne $expected) {
            throw "sealed file changed during final verification: $relative"
        }
        $verified++
    }
    $coveredFiles = @(
        Get-ChildItem -Recurse -File -LiteralPath $runRoot |
            Where-Object { $_.FullName -ne $sealPath }
    ).Count
    if ($verified -ne $lines.Count -or $verified -ne $coveredFiles) {
        throw 'SHA256SUMS coverage changed during final verification'
    }
    return [ordered]@{
        path = 'SHA256SUMS'
        sha256 = Get-Sha256 $sealPath
        entries = $lines.Count
        hash_basis = 'raw_file_bytes'
    }
}

function Assert-PreexistingEvidenceUnchanged {
    $observed = [Collections.Generic.Dictionary[string,string]]::new(
        [StringComparer]::Ordinal
    )
    if (Test-Path -LiteralPath $runsRoot) {
        Get-ChildItem -Recurse -File -LiteralPath $runsRoot |
            Where-Object { -not $_.FullName.StartsWith(
                    $runRoot + '\',
                    [StringComparison]::OrdinalIgnoreCase
                ) } |
            Sort-Object FullName |
            ForEach-Object {
                $repoRelative = $_.FullName.Substring($repositoryRoot.Length + 1)
                $repoRelative = $repoRelative.Replace('\', '/')
                $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash
                $observed.Add($repoRelative, $hash.ToLowerInvariant())
            }
    }
    if ($observed.Count -ne $preexistingRunFiles.Count) {
        throw 'pre-existing receipt evidence file set changed during capture'
    }
    foreach ($entry in $preexistingRunFiles.GetEnumerator()) {
        if (-not $observed.ContainsKey($entry.Key) -or
            $observed[$entry.Key] -ne $entry.Value) {
            throw "pre-existing receipt evidence changed: $($entry.Key)"
        }
    }
    $script:preexistingEvidenceUnchanged = $true
}

try {
    $failureStage = 'initialization'
    if (Test-Path -LiteralPath $runsRoot) {
        Get-ChildItem -Recurse -File -LiteralPath $runsRoot |
            Sort-Object FullName |
            ForEach-Object {
                $repoRelative = $_.FullName.Substring($repositoryRoot.Length + 1)
                $repoRelative = $repoRelative.Replace('\', '/')
                $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash
                $preexistingRunFiles.Add($repoRelative, $hash.ToLowerInvariant())
            }
    }
    if (Test-Path -LiteralPath $runRoot) {
        throw "refusing to overwrite evidence run $runId"
    }
    [void](New-Item -ItemType Directory -Path $commandsRoot)
    [void](New-Item -ItemType Directory -Path $artifactsRoot)
    [void](New-Item -ItemType Directory -Path $targetRoot)
    [void](New-Item -ItemType Directory -Path $scratchRoot)
    $env:CARGO_TARGET_DIR = Join-Path $targetRoot 'cargo-target'

    $failureStage = 'input_identity_start'
    $contractHashStart = Get-Sha256 $contractPath
    $captureScriptHashStart = Get-Sha256 $captureScriptPath
    $failureStage = 'commands'
    Invoke-NativeCapture -Id 'C01' -File 'git' -Arguments @(
        'rev-parse', '--is-inside-work-tree'
    )
    Invoke-NativeCapture -Id 'C02' -File 'git' -Arguments @(
        'rev-parse', '--verify', 'HEAD'
    )
    Invoke-NativeCapture -Id 'C03' -File 'git' -Arguments @(
        'rev-parse', 'HEAD^{tree}'
    )
    Invoke-NativeCapture -Id 'C04' -File 'git' -Arguments @(
        'branch', '--show-current'
    )
    Invoke-NativeCapture -Id 'C05' -File 'git' -Arguments @(
        'status', '--porcelain=v2', '--branch', '--untracked-files=all'
    )
    Invoke-NativeCapture -Id 'C06' -File 'rustc' -Arguments @('-Vv')
    Invoke-NativeCapture -Id 'C07' -File 'cargo' -Arguments @('-V')
    Invoke-NativeCapture -Id 'C08' -File 'git' -Arguments @(
        'ls-tree', '-r', '--full-tree', 'HEAD'
    )
    Invoke-NativeCapture -Id 'C09' -File 'git' -Arguments @('ls-files')
    Invoke-JsonValidationCapture -Id 'C10'
    Invoke-NativeCapture -Id 'C11' -File 'cargo' -Arguments @(
        'run', '--quiet', '--locked', '--offline', '--', '--help'
    )
    Invoke-NativeCapture -Id 'C12' -File 'cargo' -Arguments @(
        'run', '--quiet', '--locked', '--offline', '--', 'plan', '--help'
    )
    Invoke-NativeCapture -Id 'C13' -File 'cargo' -Arguments @(
        'run', '--quiet', '--locked', '--offline', '--', 'curate', '--help'
    )
    Invoke-NativeCapture -Id 'C14' -File 'cargo' -Arguments @(
        'run', '--quiet', '--locked', '--offline', '--', 'train-tokenizer', '--help'
    )
    Invoke-NativeCapture -Id 'C15' -File 'cargo' -Arguments @(
        'run', '--quiet', '--locked', '--offline', '--', 'tokenize', '--help'
    )
    Invoke-NativeCapture -Id 'C16' -File 'cargo' -Arguments @(
        'run', '--quiet', '--locked', '--offline', '--', 'train', '--help'
    )
    Invoke-NativeCapture -Id 'C17' -File 'cargo' -Arguments @(
        'fmt', '--all', '--', '--check'
    )
    Invoke-NativeCapture -Id 'C18' -File 'cargo' -Arguments @(
        'clippy', '--locked', '--offline', '--all-targets', '--features',
        'cpu-reference', '--', '-D', 'warnings'
    )
    Invoke-NativeCapture -Id 'C19' -File 'cargo' -Arguments @(
        'test', '--locked', '--offline', '--features', 'cpu-reference', '--', '--list'
    )
    Invoke-NativeCapture -Id 'C20' -File 'cargo' -Arguments @(
        'test', '--locked', '--offline', '--features', 'cpu-reference', '--',
        '--test-threads=1'
    )
    Invoke-NativeCapture -Id 'C21' -File 'cargo' -Arguments @(
        'run', '--quiet', '--locked', '--offline', '--', 'plan'
    )
    Invoke-NativeCapture -Id 'C22' -File 'cargo' -Arguments @(
        'run', '--quiet', '--locked', '--offline', '--', 'plan', '--gqa-135m'
    )
    Invoke-NativeCapture -Id 'C23' -File 'git' -Arguments @(
        'rev-parse', '--verify', 'HEAD'
    )
    Invoke-NativeCapture -Id 'C24' -File 'git' -Arguments @(
        'diff', '--exit-code', '--', 'src', 'Cargo.toml', 'Cargo.lock', 'build.rs'
    )
    Invoke-NativeCapture -Id 'C25' -File 'git' -Arguments @(
        'diff', '--cached', '--exit-code', '--', 'src', 'Cargo.toml', 'Cargo.lock',
        'build.rs'
    )
    Invoke-NativeCapture -Id 'C26' -File 'git' -Arguments @('diff', '--check')
    Invoke-NativeCapture -Id 'C29' -File 'git' -Arguments @(
        'diff', '--cached', '--check'
    )
    Invoke-NativeCapture -Id 'C27' -File 'git' -Arguments @(
        'status', '--porcelain=v2', '--branch', '--untracked-files=all'
    )
    Invoke-NativeCapture -Id 'C28' -File 'git' -Arguments @(
        'status', '--short', '--untracked-files=all'
    )

    $failureStage = 'frozen_source_assertions'
    $insideWorktree = (
        Get-Content -Raw -Encoding UTF8 -LiteralPath (
            Join-Path $commandsRoot 'C01.stdout.txt'
        )
    ).Trim()
    $headBefore = (
        Get-Content -Raw -Encoding UTF8 -LiteralPath (
            Join-Path $commandsRoot 'C02.stdout.txt'
        )
    ).Trim()
    $treeBefore = (
        Get-Content -Raw -Encoding UTF8 -LiteralPath (
            Join-Path $commandsRoot 'C03.stdout.txt'
        )
    ).Trim()
    $branchBefore = (
        Get-Content -Raw -Encoding UTF8 -LiteralPath (
            Join-Path $commandsRoot 'C04.stdout.txt'
        )
    ).Trim()
    $headAfter = (
        Get-Content -Raw -Encoding UTF8 -LiteralPath (
            Join-Path $commandsRoot 'C23.stdout.txt'
        )
    ).Trim()
    if ($insideWorktree -ne 'true') {
        throw 'capture did not run inside a Git worktree'
    }
    if ($headBefore -ne $frozenCommit -or $headAfter -ne $frozenCommit) {
        throw 'HEAD does not match the frozen Phase 0 commit'
    }
    if ($treeBefore -ne $frozenTree) {
        throw 'HEAD tree does not match the frozen Phase 0 tree'
    }
    if ($branchBefore -ne 'main') {
        throw 'Phase 0 capture is not on the observed main branch'
    }

    $failureStage = 'write_scope_assertions'
    $statusLines = Get-Content -Encoding UTF8 -LiteralPath (
        Join-Path $commandsRoot 'C28.stdout.txt'
    )
    $unexpectedPaths = [Collections.Generic.List[string]]::new()
    foreach ($line in $statusLines) {
        if ($line.Length -lt 4) {
            continue
        }
        $indexState = $line.Substring(0, 2)
        $path = $line.Substring(3).Replace('\', '/')
        $allowed = $path -eq 'docs/rebuild-contract.md' -or
            $path -eq 'docs/receipts/P0.md' -or
            $path -eq 'docs/receipts/P0/evidence.json' -or
            $path -eq 'docs/receipts/P0/capture.ps1' -or
            $path.StartsWith("docs/receipts/P0/runs/$runId/") -or
            $preexistingRunFiles.ContainsKey($path)
        if (-not $allowed -or $path.Contains(' -> ') -or $indexState -ne '??') {
            $unexpectedPaths.Add("$indexState $path")
        }
    }
    if ($unexpectedPaths.Count -gt 0) {
        throw "unexpected Phase 0 paths: $($unexpectedPaths -join ', ')"
    }

    $failureStage = 'preexisting_evidence_immutability'
    Assert-PreexistingEvidenceUnchanged

    Add-DerivedArtifact -SourceRelative 'commands/C08.stdout.txt' `
        -DestinationName 'tracked-tree.txt' -DerivedFrom 'C08'
    Add-DerivedArtifact -SourceRelative 'commands/C09.stdout.txt' `
        -DestinationName 'tracked-files.txt' -DerivedFrom 'C09'
    Add-DerivedArtifact -SourceRelative 'commands/C19.stdout.txt' `
        -DestinationName 'test-list.txt' -DerivedFrom 'C19'
    Add-DerivedArtifact -SourceRelative 'commands/C21.stdout.txt' `
        -DestinationName 'plan-124m.json' -DerivedFrom 'C21'
    Add-DerivedArtifact -SourceRelative 'commands/C22.stdout.txt' `
        -DestinationName 'plan-135m.json' -DerivedFrom 'C22'
    Add-InputSnapshot -SourcePath $contractPath -DestinationName 'rebuild-contract.md' `
        -ExpectedSha256 $contractHashStart -SourceIdentity 'contract_start'
    Add-InputSnapshot -SourcePath $captureScriptPath -DestinationName 'capture.ps1' `
        -ExpectedSha256 $captureScriptHashStart -SourceIdentity 'capture_script_start'

    $failureStage = 'reference_assertions'
    $plan124 = Get-Content -Raw -Encoding UTF8 -LiteralPath (
        Join-Path $artifactsRoot 'plan-124m.json'
    ) | ConvertFrom-Json
    $plan135 = Get-Content -Raw -Encoding UTF8 -LiteralPath (
        Join-Path $artifactsRoot 'plan-135m.json'
    ) | ConvertFrom-Json
    if ($plan124.parameters -ne 124668672 -or $plan124.model.d_ff -ne 2048) {
        throw '124M reference plan changed'
    }
    if ($plan135.parameters -ne 135285504 -or $plan135.model.d_ff -ne 2432) {
        throw '135M canonical plan changed'
    }
    if ([Math]::Abs(
            $plan124.exact_minimum_tokens_per_second_for_eight_hours -
            69444.44444444444
        ) -gt 0.000000001) {
        throw 'eight-hour arithmetic floor changed'
    }
    if ($plan124.acceptance_gate -notmatch 'closed' -or
        $plan135.acceptance_gate -notmatch 'closed') {
        throw 'reference production gate is not fail-closed'
    }
    $testOutput = Get-Content -Raw -Encoding UTF8 -LiteralPath (
        Join-Path $commandsRoot 'C20.stdout.txt'
    )
    if ($testOutput -notmatch '22 passed; 0 failed') {
        throw 'reference CPU test count changed'
    }

    $failureStage = 'input_identity_end'
    $contractHashEnd = Get-Sha256 $contractPath
    $captureScriptHashEnd = Get-Sha256 $captureScriptPath
    if ($contractHashStart -ne $contractHashEnd) {
        throw 'rebuild contract changed during capture'
    }
    if ($captureScriptHashStart -ne $captureScriptHashEnd) {
        throw 'capture script changed during capture'
    }

    $failureStage = 'final_head_assertion'
    Invoke-NativeCapture -Id 'C30' -File 'git' -Arguments @(
        'rev-parse', '--verify', 'HEAD'
    )
    $sealedHead = (
        Get-Content -Raw -Encoding UTF8 -LiteralPath (
            Join-Path $commandsRoot 'C30.stdout.txt'
        )
    ).Trim()
    if ($sealedHead -ne $frozenCommit) {
        throw 'HEAD changed before evidence sealing'
    }

    $failureStage = 'temporary_target_cleanup'
    $tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    $targetFull = [IO.Path]::GetFullPath($targetRoot)
    if (-not $targetFull.StartsWith($tempBase, [StringComparison]::OrdinalIgnoreCase) -or
        -not ([IO.Path]::GetFileName($targetFull)).StartsWith('p0-')) {
        throw 'refusing unsafe temporary target cleanup'
    }
    Remove-Item -LiteralPath $targetFull -Recurse -Force
    $targetCacheCleaned = -not (Test-Path -LiteralPath $targetFull)
    if (-not $targetCacheCleaned) {
        throw 'temporary target cleanup did not complete'
    }
    $failureStage = 'complete'
}
catch {
    $overallStatus = 'fail'
    $failureMessage = (Protect-Text $_.Exception.Message).text.Trim()
}
finally {
    try {
        if (Test-Path -LiteralPath $contractPath) {
            $contractHashEnd = Get-Sha256 $contractPath
        }
        if (Test-Path -LiteralPath $captureScriptPath) {
            $captureScriptHashEnd = Get-Sha256 $captureScriptPath
        }
    }
    catch {
        if ($overallStatus -eq 'pass') {
            $overallStatus = 'fail'
            $failureStage = 'final_input_identity'
            $failureMessage = (Protect-Text $_.Exception.Message).text.Trim()
        } else {
            $secondaryFailures.Add(
                'final input identity read: ' + (Protect-Text $_.Exception.Message).text.Trim()
            )
        }
    }
    $finalIdentityMatches = (
        $null -ne $contractHashStart -and
        $contractHashStart -eq $contractHashEnd -and
        $null -ne $captureScriptHashStart -and
        $captureScriptHashStart -eq $captureScriptHashEnd
    )
    if (-not $finalIdentityMatches) {
        if ($overallStatus -eq 'pass') {
            $overallStatus = 'fail'
            $failureStage = 'final_input_identity'
            $failureMessage = 'contract or capture script changed before sealing'
        } else {
            $secondaryFailures.Add('contract or capture script changed before sealing')
        }
    }
    try {
        Assert-PreexistingEvidenceUnchanged
    }
    catch {
        if ($overallStatus -eq 'pass') {
            $overallStatus = 'fail'
            $failureStage = 'final_preexisting_evidence_immutability'
            $failureMessage = (Protect-Text $_.Exception.Message).text.Trim()
        } else {
            $secondaryFailures.Add(
                'pre-existing evidence: ' + (Protect-Text $_.Exception.Message).text.Trim()
            )
        }
    }
    if (-not $targetCacheCleaned -and (Test-Path -LiteralPath $targetRoot)) {
        try {
            $tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
            $targetFull = [IO.Path]::GetFullPath($targetRoot)
            if (-not $targetFull.StartsWith(
                    $tempBase,
                    [StringComparison]::OrdinalIgnoreCase
                ) -or -not ([IO.Path]::GetFileName($targetFull)).StartsWith(
                    'p0-'
                )) {
                throw 'refusing unsafe fallback temporary target cleanup'
            }
            Remove-Item -LiteralPath $targetFull -Recurse -Force
            $targetCacheCleaned = -not (Test-Path -LiteralPath $targetFull)
        }
        catch {
            if ($overallStatus -eq 'pass') {
                $overallStatus = 'fail'
                $failureStage = 'temporary_target_cleanup'
                $failureMessage = (Protect-Text $_.Exception.Message).text.Trim()
            } else {
                $secondaryFailures.Add(
                    'temporary target cleanup: ' + (
                        (Protect-Text $_.Exception.Message).text.Trim()
                    )
                )
            }
        }
    }
    $priorInventoryLines = @(
        $preexistingRunFiles.GetEnumerator() |
            Sort-Object Key |
            ForEach-Object { "$($_.Value)  $($_.Key)" }
    )
    $priorInventoryText = $priorInventoryLines -join "`n"
    if ($priorInventoryText.Length -gt 0) {
        $priorInventoryText += "`n"
    }
    $finishedAt = (Get-Date).ToUniversalTime()
    $summary = [ordered]@{
        schema = 'python-slm-p0-capture-v4'
        run_id = $runId
        overall_status = $overallStatus
        failed_command = $failedCommand
        failure_stage = if ($overallStatus -eq 'pass') { $null } else { $failureStage }
        failure_message = $failureMessage
        secondary_failures = @($secondaryFailures)
        started_at_utc = $startedAt.ToString('o')
        finished_at_utc = $finishedAt.ToString('o')
        environment = [ordered]@{
            powershell_version = $PSVersionTable.PSVersion.ToString()
            clr_version = [Environment]::Version.ToString()
            process_is_64_bit = [Environment]::Is64BitProcess
            execution_policy_process = (Get-ExecutionPolicy -Scope Process).ToString()
        }
        cargo_offline = $true
        temporary_target = [ordered]@{
            path = '${TEMP}'
            cargo_target_dir_was_new = $true
            cleaned = $targetCacheCleaned
        }
        allowed_write_roots = @(
            '${REPO}/docs/receipts/P0/' + "runs/$runId",
            '${TEMP} (created, then removed)',
            '${CARGO_HOME} or ${USERPROFILE}/.cargo (existing offline cache; read/write effects not confined)',
            '${RUSTUP_HOME} or ${USERPROFILE}/.rustup (existing toolchain; read/write effects not confined)'
        )
        initial_observation = [ordered]@{
            source = 'interactive pre-write capture; not a scripted command result'
            eligible_for_automated_pass = $false
            command = @(
                'git', 'status', '--porcelain=v2', '--branch', '--untracked-files=all'
            )
            captured_at_utc = '2026-08-11T06:53:09.5751955Z'
            head_commit = $frozenCommit
            head_tree = $frozenTree
            branch = 'main'
            worktree_clean = $true
        }
        preexisting_receipt_evidence = [ordered]@{
            files = $preexistingRunFiles.Count
            inventory_sha256 = Get-TextSha256 $priorInventoryText
            unchanged = $preexistingEvidenceUnchanged
            eligible_for_this_run = $false
            disposition = 'preserved superseded evidence; not cited by this run'
        }
        input_identity = [ordered]@{
            contract_start_sha256 = $contractHashStart
            contract_end_sha256 = $contractHashEnd
            capture_script_start_sha256 = $captureScriptHashStart
            capture_script_end_sha256 = $captureScriptHashEnd
            hashes_match = $finalIdentityMatches
        }
        rebuild_contract = [ordered]@{
            path = 'docs/rebuild-contract.md'
            sha256 = $contractHashStart
            hash_basis = 'raw_file_bytes'
            status = 'awaiting_review'
        }
        commands = $results
        artifacts = $artifacts
        concurrency_precondition = 'no concurrent writer may mutate the repository during sealing'
    }
    $summaryPath = Join-Path $runRoot 'command-results.json'
    try {
        Write-Utf8Lf -Path $summaryPath -Text ($summary | ConvertTo-Json -Depth 12)
        [void](Get-Content -Raw -Encoding UTF8 -LiteralPath $summaryPath |
            ConvertFrom-Json)
        $seal = Write-RunSeal
    }
    catch {
        $overallStatus = 'fail'
        $failureStage = 'finalization'
        $failureMessage = (Protect-Text $_.Exception.Message).text.Trim()
        $summary['overall_status'] = $overallStatus
        $summary['failure_stage'] = $failureStage
        $summary['failure_message'] = $failureMessage
        $summary['secondary_failures'] = @($secondaryFailures)
        try {
            Write-Utf8Lf -Path $summaryPath -Text ($summary | ConvertTo-Json -Depth 12)
            [void](Get-Content -Raw -Encoding UTF8 -LiteralPath $summaryPath |
                ConvertFrom-Json)
            $seal = Write-RunSeal
        }
        catch {
            $seal = $null
            $retryMessage = 'finalization retry: ' + (
                (Protect-Text $_.Exception.Message).text.Trim()
            )
            $secondaryFailures.Add($retryMessage)
            $failureMessage += '; ' + $retryMessage
            $summary['failure_message'] = $failureMessage
            $summary['secondary_failures'] = @($secondaryFailures)
            try {
                Write-Utf8Lf -Path $summaryPath -Text (
                    $summary | ConvertTo-Json -Depth 12
                )
            }
            catch {
                [Console]::Error.WriteLine(
                    "Phase 0 could not rewrite failure summary for run $runId"
                )
            }
        }
    }
}

if ($overallStatus -ne 'pass') {
    [Console]::Error.WriteLine("Phase 0 capture failed; run summary: $runId")
    exit 1
}
Write-Output "Captured $($results.Count) passing commands in run $runId."
Write-Output "Run seal: $($seal.sha256)"
Write-Output 'Temporary target: ${TEMP} (cleaned)'

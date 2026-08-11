Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:Utf8NoBom = [Text.UTF8Encoding]::new($false, $true)
$script:Lf = [string][char]10
$script:Cr = [string][char]13
$script:LegacyFailureEvidenceSha256 = @{
    '20260811T135205572Z-b4054fbd19f3447cbc6cb13e' = '0e3102f47d1671bc7807dd4302aa5e214db8369cf0b20095a0131d34ba9157c6'
    '20260811T135349352Z-b7b1c0230b47473ba22ce6c0' = 'acc8a773b452e9edaac6868d1515b2270438ad393a88b576ef080234eab26e82'
    '20260811T135551718Z-bac8f20fcdc54147811a1266' = '8f695108d1df57c75060761db4c93da7fd863c48cf5ea06d8e3194964d6a5974'
    '20260811T135746406Z-aef03b68c7fb4bb5b6b75e17' = '09100c6bc76da55e7db1f742e91588101ba38ade50e6d00180c3bd57e35ee485'
    '20260811T135924951Z-21d92bf2c9f14bdb8c57bbc2' = '5be7bc1d0ac832af984438535cd240832508bca17549e3916726f92e3422a7c3'
    '20260811T140125365Z-bcbbe06ec66245b4941cc2b9' = '27b69d1bcea97d896ad45764a1a0dd9cd7039adc40b20ca20aa4af1e9f44d5ef'
    '20260811T140306815Z-3af3710134354c378097aa9a' = 'c0496697aad115ebcd2e0075b9adc18f09978a56d000418279576a38698e3bf3'
    '20260811T140631147Z-fc46ebcc30184ee09fca08bd' = 'ead8f22e937b1331033ad0fc888129c2aac9fc15cafb1277448181694d22d3c1'
}

function ConvertTo-P1ACommandLine {
    [CmdletBinding()]
    param([Parameter(Mandatory)][AllowEmptyCollection()][AllowEmptyString()][string[]]$Argument)

    $rendered = [Collections.Generic.List[string]]::new()
    foreach ($itemValue in $Argument) {
        $item = if ($null -eq $itemValue) { '' } else { $itemValue }
        if ($item -match '[\r\n]') {
            throw 'command-line arguments must be single-line'
        }
        if ($item.Length -gt 0 -and $item -notmatch '[\s"]') {
            $rendered.Add($item)
            continue
        }
        $builder = [Text.StringBuilder]::new()
        [void]$builder.Append('"')
        $slashes = 0
        foreach ($character in $item.ToCharArray()) {
            if ($character -eq '\') {
                $slashes++
                continue
            }
            if ($character -eq '"') {
                [void]$builder.Append(('\' * (($slashes * 2) + 1)))
                [void]$builder.Append('"')
                $slashes = 0
                continue
            }
            if ($slashes -gt 0) {
                [void]$builder.Append(('\' * $slashes))
                $slashes = 0
            }
            [void]$builder.Append($character)
        }
        if ($slashes -gt 0) {
            [void]$builder.Append(('\' * ($slashes * 2)))
        }
        [void]$builder.Append('"')
        $rendered.Add($builder.ToString())
    }
    return ($rendered -join ' ')
}

function Test-P1APathWithin {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$Root
    )

    try {
        $fullPath = [IO.Path]::GetFullPath($Path).TrimEnd('\', '/')
        $fullRoot = [IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    }
    catch {
        return $false
    }
    if ([string]::Equals($fullPath, $fullRoot, [StringComparison]::OrdinalIgnoreCase)) {
        return $true
    }
    return $fullPath.StartsWith(
        $fullRoot + [IO.Path]::DirectorySeparatorChar,
        [StringComparison]::OrdinalIgnoreCase
    )
}

function ConvertTo-P1ANormalizedPath {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$BasePath
    )

    $fullPath = [IO.Path]::GetFullPath($Path)
    $fullBase = [IO.Path]::GetFullPath($BasePath).TrimEnd('\', '/')
    if (-not (Test-P1APathWithin -Path $fullPath -Root $fullBase)) {
        throw "path is outside the requested base: $Path"
    }
    if ([string]::Equals($fullPath.TrimEnd('\', '/'), $fullBase, [StringComparison]::OrdinalIgnoreCase)) {
        return '.'
    }
    return $fullPath.Substring($fullBase.Length + 1).Replace('\', '/')
}

function Resolve-P1AOutputRoot {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$RepositoryRoot,
        [Parameter(Mandatory)][string]$OutputRoot,
        [ValidateSet('P1A', 'P1B')][string]$PhaseId = 'P1A'
    )

    $repository = [IO.Path]::GetFullPath($RepositoryRoot).TrimEnd('\', '/')
    $candidate = if ([IO.Path]::IsPathRooted($OutputRoot)) {
        [IO.Path]::GetFullPath($OutputRoot)
    }
    else {
        [IO.Path]::GetFullPath((Join-Path $repository $OutputRoot))
    }
    $allowed = [IO.Path]::GetFullPath((Join-Path $repository "docs\receipts\$PhaseId"))
    if (-not [string]::Equals(
            $candidate.TrimEnd('\', '/'),
            $allowed.TrimEnd('\', '/'),
            [StringComparison]::OrdinalIgnoreCase
        )) {
        throw "$PhaseId output root must be the repository docs/receipts/$PhaseId directory"
    }
    if (Test-Path -LiteralPath $candidate -PathType Leaf) {
        throw 'P1A output root resolves to a file, not a directory'
    }
    if (-not (Test-P1APathWithin -Path $candidate -Root $repository)) {
        throw 'P1A output root escapes repository containment'
    }
    $cursor = $candidate
    while ($cursor -and (Test-P1APathWithin -Path $cursor -Root $repository)) {
        if (Test-Path -LiteralPath $cursor) {
            $item = Get-Item -Force -LiteralPath $cursor
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "P1A output ancestor is a reparse point: $cursor"
            }
        }
        if ([string]::Equals($cursor.TrimEnd('\', '/'), $repository, [StringComparison]::OrdinalIgnoreCase)) {
            break
        }
        $cursor = Split-Path -Parent $cursor
    }
    return $candidate
}

function Write-P1AUtf8LfFile {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Path,
        [AllowEmptyString()][string]$Text,
        [switch]$CreateNew
    )

    $value = if ($null -eq $Text) { '' } else { $Text }
    $normalized = $value.Replace($script:Cr + $script:Lf, $script:Lf).Replace($script:Cr, $script:Lf)
    if ($normalized.Length -gt 0 -and -not $normalized.EndsWith($script:Lf)) {
        $normalized += $script:Lf
    }
    $mode = if ($CreateNew) { [IO.FileMode]::CreateNew } else { [IO.FileMode]::Create }
    $stream = [IO.FileStream]::new($Path, $mode, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $bytes = $script:Utf8NoBom.GetBytes($normalized)
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
    }
    finally {
        $stream.Dispose()
    }
}

function Write-P1AJsonFile {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)]$Value,
        [switch]$CreateNew
    )

    Write-P1AUtf8LfFile -Path $Path -Text ($Value | ConvertTo-Json -Depth 32) -CreateNew:$CreateNew
}

function Get-P1ASha256 {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Path)

    return (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
}

function Get-P1ATextSha256 {
    param([AllowEmptyString()][string]$Text)

    $value = if ($null -eq $Text) { '' } else { $Text }
    $hasher = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString(
                $hasher.ComputeHash($script:Utf8NoBom.GetBytes($value))
            )).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $hasher.Dispose()
    }
}

function New-P1ARunId {
    [CmdletBinding()]
    param()

    $stamp = (Get-Date).ToUniversalTime().ToString('yyyyMMddTHHmmssfffZ')
    return $stamp + '-' + [guid]::NewGuid().ToString('N').Substring(0, 24)
}

function ConvertFrom-P1ARustcVersion {
    [CmdletBinding()]
    param([Parameter(Mandatory)][AllowEmptyString()][string]$Text)

    if ([string]::IsNullOrWhiteSpace($Text)) {
        throw 'rustc version output is empty'
    }
    $values = [ordered]@{}
    foreach ($line in ($Text -split '\r?\n')) {
        $separator = $line.IndexOf(':')
        if ($separator -le 0) {
            continue
        }
        $values[$line.Substring(0, $separator).Trim().ToLowerInvariant()] =
            $line.Substring($separator + 1).Trim()
    }
    foreach ($required in @('release', 'host', 'commit-hash', 'commit-date', 'llvm version')) {
        if (-not $values.Contains($required) -or
            [string]::IsNullOrWhiteSpace([string]$values[$required])) {
            throw "could not parse required rustc version field: $required"
        }
    }
    return [pscustomobject][ordered]@{
        release = [string]$values['release']
        host = [string]$values['host']
        commit_hash = [string]$values['commit-hash']
        commit_date = [string]$values['commit-date']
        llvm_version = [string]$values['llvm version']
    }
}

function Test-P1AVersionAtLeast {
    param(
        [Parameter(Mandatory)][string]$Actual,
        [Parameter(Mandatory)][string]$Minimum
    )
    try {
        return ([version]$Actual -ge [version]$Minimum)
    }
    catch {
        return $false
    }
}

function Get-P1AForbiddenCargoTreeTokens {
    param([AllowEmptyString()][string]$Text = '')

    $matches = [Collections.Generic.List[string]]::new()
    foreach ($line in @($Text -split '\r?\n')) {
        foreach ($match in [regex]::Matches(
                $line,
                '(?i)[A-Za-z0-9_.+-]*(?:cuda|cudnn|cublas|curand|cusparse|nvrtc|nvjitlink)[A-Za-z0-9_.+-]*'
            )) {
            $matches.Add('cargo-tree-token:' + $match.Value.ToLowerInvariant())
        }
    }
    return @($matches | Sort-Object -Unique)
}

function Select-P1AVs2022Instance {
    [CmdletBinding()]
    param([Parameter(Mandatory)][AllowEmptyCollection()][object[]]$Instances)

    $candidates = @($Instances | ForEach-Object { $_ })
    $eligible = @(
        $candidates | Where-Object {
            try {
                $version = [version]$_.installationVersion
            }
            catch {
                $version = $null
            }
            $complete = $_.PSObject.Properties['isComplete']
            $launchable = $_.PSObject.Properties['isLaunchable']
            $reboot = $_.PSObject.Properties['isRebootRequired']
            $version -and $version.Major -eq 17 -and
                $null -ne $complete -and [bool]$complete.Value -and
                $null -ne $launchable -and [bool]$launchable.Value -and
                $null -ne $reboot -and -not [bool]$reboot.Value
        } | Sort-Object @{
            Expression = { [version]$_.installationVersion }
            Descending = $true
        }, @{
            Expression = {
                if ($_.PSObject.Properties['instanceId']) { [string]$_.instanceId } else { '' }
            }
            Descending = $false
        }
    )
    if ($eligible.Count -eq 0) {
        throw '[VS_INSTANCE_NOT_FOUND] no complete, launchable Visual Studio 2022 (17.x) instance was found'
    }
    return $eligible[0]
}

function Protect-P1AText {
    [CmdletBinding()]
    param(
        [AllowEmptyString()][string]$Text,
        [Parameter(Mandatory)][string]$RepositoryRoot,
        [string[]]$ExtraRoots = @()
    )

    $value = if ($null -eq $Text) { '' } else { $Text }
    $rules = [Collections.Generic.List[object]]::new()
    $rules.Add([pscustomobject]@{
        Value = [IO.Path]::GetFullPath($RepositoryRoot).TrimEnd('\', '/')
        Token = '$' + '{REPO}'
    })
    $index = 0
    foreach ($root in $ExtraRoots) {
        if ([string]::IsNullOrWhiteSpace($root)) {
            continue
        }
        $fullRoot = [IO.Path]::GetFullPath($root).TrimEnd('\', '/')
        $tokenName = if ($env:USERPROFILE -and
            [string]::Equals($fullRoot, [IO.Path]::GetFullPath($env:USERPROFILE).TrimEnd('\', '/'), [StringComparison]::OrdinalIgnoreCase)) {
            'USERPROFILE'
        }
        elseif ([string]::Equals(
                $fullRoot,
                [IO.Path]::GetFullPath($(if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' })).TrimEnd('\', '/'),
                [StringComparison]::OrdinalIgnoreCase
            )) {
            'CARGO_HOME'
        }
        elseif ([string]::Equals(
                $fullRoot,
                [IO.Path]::GetFullPath($(if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE '.rustup' })).TrimEnd('\', '/'),
                [StringComparison]::OrdinalIgnoreCase
            )) {
            'RUSTUP_HOME'
        }
        elseif ($env:SystemRoot -and
            [string]::Equals($fullRoot, [IO.Path]::GetFullPath($env:SystemRoot).TrimEnd('\', '/'), [StringComparison]::OrdinalIgnoreCase)) {
            'WINDOWS'
        }
        elseif ($fullRoot -match '(?i)[\\/]Microsoft Visual Studio[\\/](?:2022|17)[\\/]') {
            'VS_INSTALL'
        }
        elseif ($fullRoot -match '(?i)[\\/]Windows Kits[\\/]') {
            'WINDOWS_KITS'
        }
        elseif ($fullRoot -match '(?i)[\\/]docs[\\/]receipts[\\/]P1A$') {
            'OUTPUT_ROOT'
        }
        elseif ((Test-P1APathWithin -Path $fullRoot -Root ([IO.Path]::GetTempPath()))) {
            'TEMP'
        }
        else {
            $index++
            'ROOT_' + $index.ToString('00')
        }
        $rules.Add([pscustomobject]@{
            Value = $fullRoot
            Token = '$' + '{' + $tokenName + '}'
        })
    }
    foreach ($rule in @($rules | Sort-Object { $_.Value.Length } -Descending)) {
        foreach ($variant in @($rule.Value, $rule.Value.Replace('\', '/')) | Select-Object -Unique) {
            if (-not [string]::IsNullOrWhiteSpace($variant)) {
                $value = [regex]::Replace(
                    $value,
                    [regex]::Escape($variant),
                    [Text.RegularExpressions.MatchEvaluator]{ param($match) $rule.Token },
                    [Text.RegularExpressions.RegexOptions]::IgnoreCase
                )
            }
        }
    }
    foreach ($identity in @($env:USERNAME, $env:COMPUTERNAME) | Where-Object {
            -not [string]::IsNullOrWhiteSpace([string]$_)
        } | Sort-Object -Unique) {
        $value = [regex]::Replace(
            $value,
            '(?i)(?<![A-Za-z0-9_.-])' + [regex]::Escape([string]$identity) + '(?![A-Za-z0-9_.-])',
            '${REDACTED_IDENTITY}'
        )
    }
    $value = [regex]::Replace(
        $value,
        '(?i)(?<key>"?(?:authorization|api[_-]?key|access[_-]?token|password|credential|secret)"?\s*[:=]\s*)(?<secret>[^\s,;}]+)',
        [Text.RegularExpressions.MatchEvaluator]{ param($match) $match.Groups['key'].Value + '${REDACTED_SECRET}' }
    )
    $value = [regex]::Replace(
        $value,
        '(?i)(?<scheme>https?://)[^/@\s:]+:[^/@\s]+@',
        [Text.RegularExpressions.MatchEvaluator]{ param($match) $match.Groups['scheme'].Value + '${REDACTED_USERINFO}@' }
    )
    return $value.Replace($script:Cr + $script:Lf, $script:Lf).Replace($script:Cr, $script:Lf)
}

function Invoke-P1AProcessLegacy {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [Parameter(Mandatory)][AllowEmptyCollection()][string[]]$ArgumentList,
        [Parameter(Mandatory)][string]$WorkingDirectory,
        [hashtable]$Environment = @{},
        [ValidateRange(1, 86400)][int]$TimeoutSeconds = 120
    )

    $command = Get-Command -Name $FilePath -CommandType Application -ErrorAction SilentlyContinue
    if (-not (Test-Path -LiteralPath $FilePath -PathType Leaf) -and $null -eq $command) {
        throw "process executable not found: $FilePath"
    }
    if (-not (Test-Path -LiteralPath $WorkingDirectory -PathType Container)) {
        throw "process working directory not found: $WorkingDirectory"
    }
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = if (Test-Path -LiteralPath $FilePath -PathType Leaf) {
        [IO.Path]::GetFullPath($FilePath)
    }
    else { $command.Source }
    $startInfo.Arguments = ConvertTo-P1ACommandLine -Argument $ArgumentList
    $startInfo.WorkingDirectory = [IO.Path]::GetFullPath($WorkingDirectory)
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($name in $Environment.Keys) {
        if ($name -match '[=\x00]') { throw "invalid environment variable name: $name" }
        if ($null -eq $Environment[$name]) {
            [void]$startInfo.EnvironmentVariables.Remove([string]$name)
        }
        else {
            $startInfo.EnvironmentVariables[[string]$name] = [string]$Environment[$name]
        }
    }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    $watch = [Diagnostics.Stopwatch]::StartNew()
    try {
        if (-not $process.Start()) { throw "failed to start process: $FilePath" }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $completed = $process.WaitForExit($TimeoutSeconds * 1000)
        if (-not $completed) {
            try { $process.Kill() } catch { }
            [void]$process.WaitForExit(5000)
        }
        else { $process.WaitForExit() }
        $watch.Stop()
        return [pscustomobject][ordered]@{
            exit_code = if ($completed) { [int]$process.ExitCode } else { 124 }
            stdout = [string]$stdoutTask.Result
            stderr = [string]$stderrTask.Result
            duration_ms = [int64]$watch.ElapsedMilliseconds
            timed_out = -not $completed
        }
    }
    finally {
        $watch.Stop()
        $process.Dispose()
    }
}

function New-P1ASealLegacy {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$RunRoot)

    $root = [IO.Path]::GetFullPath($RunRoot)
    if (-not (Test-Path -LiteralPath $root -PathType Container)) {
        throw "run root not found: $root"
    }
    $sealPath = Join-Path $root 'SHA256SUMS'
    if (Test-Path -LiteralPath $sealPath) { throw 'refusing to replace an existing run seal' }
    $files = @(Get-ChildItem -LiteralPath $root -File -Recurse | Where-Object {
            $_.FullName -ne $sealPath -and $_.Name -ne 'evidence.json'
        } | Sort-Object { ConvertTo-P1ANormalizedPath -Path $_.FullName -BasePath $root })
    if ($files.Count -eq 0) { throw 'cannot seal an empty run' }
    $lines = foreach ($file in $files) {
        $relative = ConvertTo-P1ANormalizedPath -Path $file.FullName -BasePath $root
        "$(Get-P1ASha256 -Path $file.FullName)  $relative"
    }
    Write-P1AUtf8LfFile -Path $sealPath -Text ($lines -join $script:Lf) -CreateNew
    if (-not (Test-P1ASeal -RunRoot $root)) { throw 'newly written run seal did not verify' }
    return [pscustomobject][ordered]@{
        path = 'SHA256SUMS'
        sha256 = Get-P1ASha256 -Path $sealPath
        entries = $files.Count
        coverage_rule = 'all_run_files_except_evidence_and_seal'
    }
}

function Test-P1ASealLegacy {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$RunRoot)

    try {
        $root = [IO.Path]::GetFullPath($RunRoot)
        $sealPath = Join-Path $root 'SHA256SUMS'
        if (-not (Test-Path -LiteralPath $sealPath -PathType Leaf)) { return $false }
        $lines = @([IO.File]::ReadAllLines($sealPath, $script:Utf8NoBom))
        if ($lines.Count -eq 0) { return $false }
        $covered = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
        $previous = $null
        foreach ($line in $lines) {
            if ($line -notmatch '^(?<hash>[0-9a-f]{64})  (?<path>[^:\\]+(?:/[^:\\]+)*)$') { return $false }
            $expectedHash = $Matches['hash']
            $relative = $Matches['path']
            if ($relative -eq 'SHA256SUMS' -or $relative -eq 'evidence.json' -or
                $relative -match '(^|/)\.\.?(/|$)') { return $false }
            if ($null -ne $previous -and [string]::CompareOrdinal($previous, $relative) -gt 0) { return $false }
            if (-not $covered.Add($relative)) { return $false }
            $fullPath = [IO.Path]::GetFullPath((Join-Path $root $relative.Replace('/', '\')))
            if (-not (Test-P1APathWithin -Path $fullPath -Root $root) -or
                -not (Test-Path -LiteralPath $fullPath -PathType Leaf)) { return $false }
            if ((Get-P1ASha256 -Path $fullPath) -cne $expectedHash) { return $false }
            $previous = $relative
        }
        $actual = @(Get-ChildItem -LiteralPath $root -File -Recurse | Where-Object {
                $_.FullName -ne $sealPath -and $_.Name -ne 'evidence.json'
            } | ForEach-Object { ConvertTo-P1ANormalizedPath -Path $_.FullName -BasePath $root })
        if ($actual.Count -ne $covered.Count) { return $false }
        foreach ($relative in $actual) { if (-not $covered.Contains($relative)) { return $false } }
        return $true
    }
    catch { return $false }
}

function Publish-P1AAcceptanceLegacy {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$OutputRoot,
        [Parameter(Mandatory)][string]$RunRoot,
        [Parameter(Mandatory)][string]$EvidencePath,
        [Parameter(Mandatory)]$SourceIdentity
    )

    $root = [IO.Path]::GetFullPath($OutputRoot)
    $run = [IO.Path]::GetFullPath($RunRoot)
    $runsRoot = Join-Path $root 'runs'
    if (-not (Test-P1APathWithin -Path $run -Root $runsRoot)) {
        throw 'acceptance run is outside the P1A runs directory'
    }
    if (-not (Test-P1ASeal -RunRoot $run)) { throw 'acceptance run seal is invalid' }
    $evidence = Get-Content -Raw -LiteralPath $EvidencePath | ConvertFrom-Json
    if ($evidence.phase_id -ne 'P1A' -or $evidence.status -ne 'PASS') {
        throw 'only passing P1A evidence can be accepted'
    }
    $environmentPath = Join-Path $run 'artifacts\environment.json'
    if (-not (Test-Path -LiteralPath $environmentPath -PathType Leaf)) {
        throw 'accepted run has no environment manifest'
    }
    $acceptanceRoot = Join-Path $root 'acceptances'
    if (-not (Test-Path -LiteralPath $acceptanceRoot)) {
        [void](New-Item -ItemType Directory -Path $acceptanceRoot -Force)
    }
    $existing = @(Get-ChildItem -LiteralPath $acceptanceRoot -File -Filter '*.json' |
        Where-Object { $_.BaseName -match '^\d{8}$' })
    $sequence = if ($existing.Count -eq 0) { 1 } else {
        [int](($existing | ForEach-Object { [int]$_.BaseName } | Measure-Object -Maximum).Maximum) + 1
    }
    $acceptanceName = $sequence.ToString('00000000') + '.json'
    $acceptancePath = Join-Path $acceptanceRoot $acceptanceName
    $pointerPath = Join-Path $root 'evidence.json'
    $previousHash = $null
    if (Test-Path -LiteralPath $pointerPath -PathType Leaf) {
        $oldPointer = Get-Content -Raw -LiteralPath $pointerPath | ConvertFrom-Json
        $oldAcceptance = Join-Path $root ([string]$oldPointer.acceptance_path).Replace('/', '\')
        if (-not (Test-P1APathWithin -Path $oldAcceptance -Root $acceptanceRoot) -or
            -not (Test-Path -LiteralPath $oldAcceptance -PathType Leaf)) {
            throw 'existing pointer references an invalid acceptance'
        }
        if ((Get-P1ASha256 -Path $oldAcceptance) -cne [string]$oldPointer.acceptance_sha256) {
            throw 'existing pointer acceptance hash mismatch'
        }
        $previousHash = [string]$oldPointer.acceptance_sha256
    }
    $sourceHash = if ($SourceIdentity -is [string]) { [string]$SourceIdentity }
        elseif ($SourceIdentity.PSObject.Properties['sha256']) { [string]$SourceIdentity.sha256 }
        else { [string]$evidence.source_identity_sha256 }
    if ($sourceHash -notmatch '^[0-9a-f]{64}$') { throw 'invalid source identity SHA-256' }
    $runRelative = 'runs/' + (Split-Path -Leaf $run)
    $sealPath = Join-Path $run 'SHA256SUMS'
    $acceptance = [ordered]@{
        schema = 'python-slm-phase-acceptance-v1'
        phase_id = 'P1A'
        sequence = $sequence
        status = 'PASS'
        acceptance_kind = 'automatic_machine_qualification'
        required_approvals = @()
        run_path = $runRelative
        run_evidence_sha256 = Get-P1ASha256 -Path $EvidencePath
        seal_path = "$runRelative/SHA256SUMS"
        seal_sha256 = Get-P1ASha256 -Path $sealPath
        environment_path = "$runRelative/artifacts/environment.json"
        environment_sha256 = Get-P1ASha256 -Path $environmentPath
        source_identity_sha256 = $sourceHash
        previous_acceptance_sha256 = $previousHash
        created_at = [DateTime]::UtcNow.ToString('o')
    }
    Write-P1AJsonFile -Path $acceptancePath -Value $acceptance -CreateNew
    $pointer = [ordered]@{
        schema = 'python-slm-phase-evidence-pointer-v1'
        phase_id = 'P1A'
        acceptance_path = 'acceptances/' + $acceptanceName
        acceptance_sha256 = Get-P1ASha256 -Path $acceptancePath
        updated_at = [DateTime]::UtcNow.ToString('o')
    }
    $temporaryPointer = Join-Path $root ('.evidence-' + [guid]::NewGuid().ToString('N') + '.json')
    Write-P1AJsonFile -Path $temporaryPointer -Value $pointer -CreateNew
    if (Test-Path -LiteralPath $pointerPath) {
        [IO.File]::Replace($temporaryPointer, $pointerPath, $null, $true)
    }
    else { [IO.File]::Move($temporaryPointer, $pointerPath) }
    $selected = Get-Content -Raw -LiteralPath $pointerPath | ConvertFrom-Json
    if ([string]$selected.acceptance_sha256 -cne [string]$pointer.acceptance_sha256) {
        throw 'atomic evidence pointer verification failed'
    }
    return [pscustomobject][ordered]@{
        acceptance_path = $acceptancePath
        pointer_path = $pointerPath
        acceptance = $acceptance
        pointer = $pointer
    }
}

function Get-P1ACanonicalEnvironment {
    $environment = [Collections.Generic.Dictionary[string,string]]::new(
        [StringComparer]::OrdinalIgnoreCase
    )
    foreach ($entry in [Environment]::GetEnvironmentVariables().GetEnumerator()) {
        $name = [string]$entry.Key
        $value = [string]$entry.Value
        if ($environment.ContainsKey($name) -and $environment[$name] -cne $value) {
            throw "process environment contains conflicting case variants for $name"
        }
        $environment[$name] = $value
    }
    return $environment
}

function Invoke-P1AProcessLegacy2 {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [AllowEmptyCollection()][string[]]$ArgumentList = @(),
        [Parameter(Mandatory)][string]$WorkingDirectory,
        [hashtable]$Environment = @{},
        [ValidateRange(1, 3600)][int]$TimeoutSeconds = 120
    )

    if (-not (Test-Path -LiteralPath $FilePath -PathType Leaf)) {
        throw "process executable was not found: $FilePath"
    }
    if (-not (Test-Path -LiteralPath $WorkingDirectory -PathType Container)) {
        throw "process working directory was not found: $WorkingDirectory"
    }
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $resolvedExecutable = [IO.Path]::GetFullPath($FilePath)
    $temporaryStem = Join-Path ([IO.Path]::GetTempPath()) `
        ('python-slm-process-' + [guid]::NewGuid().ToString('N'))
    $wrapperPath = $temporaryStem + '.cmd'
    $helperPath = $null
    $planPath = $null
    $wrapperLines = [Collections.Generic.List[string]]::new()
    $wrapperLines.Add('@echo off')
    $wrapperLines.Add('setlocal DisableDelayedExpansion')
    foreach ($entry in @($Environment.GetEnumerator() | Sort-Object Key)) {
        $name = [string]$entry.Key
        if ([string]::IsNullOrWhiteSpace($name) -or $name.IndexOf('=') -ge 0 -or
            $name.IndexOf([char]0) -ge 0) {
            throw "[CHILD_ENVIRONMENT_NAME_INVALID] invalid child environment name"
        }
        if ($null -eq $entry.Value) {
            $wrapperLines.Add("set `"$name=`"")
            continue
        }
        $value = [string]$entry.Value
        if ($value -match '[\r\n"]') {
            throw "child environment value cannot be represented safely: $name"
        }
        $wrapperLines.Add(('set "{0}={1}"' -f $name, $value.Replace('%', '%%')))
    }
    $targetArguments = @($resolvedExecutable) + @($ArgumentList)
    $quotedTarget = foreach ($argument in $targetArguments) {
        if ($argument -match '[\r\n"]') {
            throw 'process argument cannot be represented safely in the child wrapper'
        }
        $escaped = $argument.Replace('%', '%%')
        $trailing = [regex]::Match($escaped, '\\+$')
        if ($trailing.Success) {
            $escaped = $escaped.Substring(0, $escaped.Length - $trailing.Length) +
                ('\' * ($trailing.Length * 2))
        }
        '"' + $escaped + '"'
    }
    $wrapperLines.Add(($quotedTarget -join ' '))
    $wrapperLines.Add('exit /b %errorlevel%')
    [IO.File]::WriteAllText($wrapperPath, ($wrapperLines -join "`r`n") + "`r`n", [Text.Encoding]::ASCII)
    $startInfo.FileName = Join-Path $env:SystemRoot 'System32\cmd.exe'
    $startInfo.Arguments = ConvertTo-P1ACommandLine -Argument @('/d', '/c', 'call', $wrapperPath)
    $startInfo.WorkingDirectory = [IO.Path]::GetFullPath($WorkingDirectory)
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = $script:Utf8NoBom
    $startInfo.StandardErrorEncoding = $script:Utf8NoBom

    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    $stopwatch = [Diagnostics.Stopwatch]::StartNew()
    try {
        if (-not $process.Start()) {
            throw "failed to start process: $FilePath"
        }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $completed = $process.WaitForExit($TimeoutSeconds * 1000)
        $timedOut = -not $completed
        if ($timedOut) {
            try {
                $taskkill = Join-Path $env:SystemRoot 'System32\taskkill.exe'
                if (Test-Path -LiteralPath $taskkill -PathType Leaf) {
                    $killInfo = [Diagnostics.ProcessStartInfo]::new()
                    $killInfo.FileName = $taskkill
                    $killInfo.Arguments = "/PID $($process.Id) /T /F"
                    $killInfo.UseShellExecute = $false
                    $killInfo.CreateNoWindow = $true
                    $killer = [Diagnostics.Process]::Start($killInfo)
                    if ($null -ne $killer) {
                        [void]$killer.WaitForExit(10000)
                        $killer.Dispose()
                    }
                }
                if (-not $process.HasExited) {
                    $process.Kill()
                }
                [void]$process.WaitForExit(10000)
            }
            catch {
                throw "process timed out and its process tree could not be terminated: $FilePath"
            }
        }
        else {
            $process.WaitForExit()
        }
        $stopwatch.Stop()
        return [pscustomobject][ordered]@{
            file_path = $startInfo.FileName
            arguments = @($ArgumentList)
            exit_code = if ($timedOut) { -1 } else { $process.ExitCode }
            duration_ms = $stopwatch.ElapsedMilliseconds
            stdout = $stdoutTask.Result
            stderr = $stderrTask.Result
            timed_out = $timedOut
        }
    }
    finally {
        if ($stopwatch.IsRunning) {
            $stopwatch.Stop()
        }
        $process.Dispose()
        foreach ($temporaryProcessFile in @($wrapperPath, $helperPath, $planPath)) {
            if ($null -ne $temporaryProcessFile -and (Test-Path -LiteralPath $temporaryProcessFile)) {
                Remove-Item -LiteralPath $temporaryProcessFile -Force -ErrorAction SilentlyContinue
            }
        }
    }
}

function Invoke-P1AProcess {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [AllowEmptyCollection()][string[]]$ArgumentList = @(),
        [Parameter(Mandatory)][string]$WorkingDirectory,
        [hashtable]$Environment = @{},
        [ValidateRange(1, 3600)][int]$TimeoutSeconds = 120
    )

    if (-not (Test-Path -LiteralPath $FilePath -PathType Leaf)) {
        throw "process executable was not found: $FilePath"
    }
    if (-not (Test-Path -LiteralPath $WorkingDirectory -PathType Container)) {
        throw "process working directory was not found: $WorkingDirectory"
    }
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = [IO.Path]::GetFullPath($FilePath)
    $startInfo.Arguments = ConvertTo-P1ACommandLine -Argument $ArgumentList
    $startInfo.WorkingDirectory = [IO.Path]::GetFullPath($WorkingDirectory)
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = $script:Utf8NoBom
    $startInfo.StandardErrorEncoding = $script:Utf8NoBom

    # Windows PowerShell 5.1 can expose both Path and PATH in the parent
    # environment. The public ProcessStartInfo getters then throw while creating
    # their case-insensitive dictionaries. Build the child environment ourselves
    # and install the documented backing type before Process.Start().
    $canonical = Get-P1ACanonicalEnvironment
    foreach ($entry in $Environment.GetEnumerator()) {
        $name = [string]$entry.Key
        if ([string]::IsNullOrWhiteSpace($name) -or $name.IndexOf('=') -ge 0 -or
            $name.IndexOf([char]0) -ge 0) {
            throw "invalid child environment name: $name"
        }
        if ($null -eq $entry.Value) {
            [void]$canonical.Remove($name)
            continue
        }
        $value = [string]$entry.Value
        if ($value.IndexOf([char]0) -ge 0) {
            throw "child environment contains a NUL value: $name"
        }
        $canonical[$name] = $value
    }
    $childEnvironment = [Collections.Specialized.StringDictionary]::new()
    foreach ($entry in $canonical.GetEnumerator()) {
        $childEnvironment[[string]$entry.Key] = [string]$entry.Value
    }
    $environmentField = $startInfo.GetType().GetField(
        'environmentVariables',
        [Reflection.BindingFlags]'Instance,NonPublic'
    )
    if ($null -eq $environmentField -or
        $environmentField.FieldType -ne [Collections.Specialized.StringDictionary]) {
        throw 'ProcessStartInfo child-environment backing field is unsupported'
    }
    $environmentField.SetValue($startInfo, $childEnvironment)

    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    $stopwatch = [Diagnostics.Stopwatch]::StartNew()
    try {
        if (-not $process.Start()) {
            throw "failed to start process: $FilePath"
        }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $completed = $process.WaitForExit($TimeoutSeconds * 1000)
        $timedOut = -not $completed
        if ($timedOut) {
            $taskkill = Join-Path $env:SystemRoot 'System32\taskkill.exe'
            if (-not (Test-Path -LiteralPath $taskkill -PathType Leaf)) {
                throw 'process timed out and taskkill.exe is unavailable'
            }
            $killInfo = [Diagnostics.ProcessStartInfo]::new()
            $killInfo.FileName = $taskkill
            $killInfo.Arguments = "/PID $($process.Id) /T /F"
            $killInfo.UseShellExecute = $false
            $killInfo.CreateNoWindow = $true
            $killer = [Diagnostics.Process]::Start($killInfo)
            if ($null -eq $killer -or -not $killer.WaitForExit(10000) -or $killer.ExitCode -ne 0) {
                if ($null -ne $killer) { $killer.Dispose() }
                throw 'process timed out and its child tree could not be terminated'
            }
            $killer.Dispose()
            if (-not $process.HasExited) {
                $process.Kill()
            }
            if (-not $process.WaitForExit(10000) -or -not $process.HasExited) {
                throw 'process timed out and its root process did not terminate'
            }
        }
        else {
            $process.WaitForExit()
        }
        $stopwatch.Stop()
        return [pscustomobject][ordered]@{
            file_path = $startInfo.FileName
            arguments = @($ArgumentList)
            exit_code = if ($timedOut) { -1 } else { [int]$process.ExitCode }
            duration_ms = [int64]$stopwatch.ElapsedMilliseconds
            stdout = [string]$stdoutTask.Result
            stderr = [string]$stderrTask.Result
            timed_out = $timedOut
        }
    }
    finally {
        if ($stopwatch.IsRunning) {
            $stopwatch.Stop()
        }
        $process.Dispose()
    }
}

function New-P1ASeal {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$RunRoot)

    $root = [IO.Path]::GetFullPath($RunRoot)
    $sealPath = Join-Path $root 'SHA256SUMS'
    if (Test-Path -LiteralPath $sealPath) {
        throw 'refusing to replace an existing run seal'
    }
    $entries = [Collections.Generic.List[string]]::new()
    Get-ChildItem -LiteralPath $root -Recurse -File |
        Where-Object {
            -not [string]::Equals($_.FullName, $sealPath, [StringComparison]::OrdinalIgnoreCase)
        } |
        ForEach-Object {
            [pscustomobject]@{
                Path = ConvertTo-P1ANormalizedPath -Path $_.FullName -BasePath $root
                FullName = $_.FullName
            }
        } |
        Sort-Object Path |
        ForEach-Object {
            $entries.Add("$(Get-P1ASha256 -Path $_.FullName)  $($_.Path)")
        }
    if ($entries.Count -eq 0) { throw 'cannot seal an empty run' }
    Write-P1AUtf8LfFile -Path $sealPath -Text ($entries -join $script:Lf) -CreateNew
    if (-not (Test-P1ASeal -RunRoot $root)) { throw 'new run seal failed verification' }
    return [pscustomobject][ordered]@{
        path = 'SHA256SUMS'
        entries = $entries.Count
        coverage_rule = 'all_run_files_except_seal'
    }
}

function Test-P1ASeal {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$RunRoot)

    try {
        $root = [IO.Path]::GetFullPath($RunRoot)
        $sealPath = Join-Path $root 'SHA256SUMS'
        $evidencePath = Join-Path $root 'evidence.json'
        if (-not (Test-Path -LiteralPath $sealPath -PathType Leaf)) {
            return $false
        }
        $sealLines = @([IO.File]::ReadAllLines($sealPath, $script:Utf8NoBom))
        if ($sealLines.Count -eq 0 -or
            ($sealLines.Count -eq 1 -and [string]::IsNullOrWhiteSpace($sealLines[0]))) {
            return $false
        }
        $legacyEvidenceExclusion = $false
        if (Test-Path -LiteralPath $evidencePath -PathType Leaf) {
            try {
                $evidence = [IO.File]::ReadAllText($evidencePath, $script:Utf8NoBom) |
                    ConvertFrom-Json
                $sealProperty = $evidence.PSObject.Properties['seal']
                if ($null -ne $sealProperty -and $null -ne $sealProperty.Value) {
                    $coverageProperty = $sealProperty.Value.PSObject.Properties['coverage_rule']
                    $legacyEvidenceExclusion = $null -ne $coverageProperty -and
                        [string]$coverageProperty.Value -ceq
                            'all_run_files_except_root_evidence_and_seal'
                    if ($legacyEvidenceExclusion) {
                        $runIdProperty = $evidence.PSObject.Properties['run_id']
                        $statusProperty = $evidence.PSObject.Properties['status']
                        $legacyRunId = if ($null -eq $runIdProperty) {
                            ''
                        }
                        else {
                            [string]$runIdProperty.Value
                        }
                        if ($null -eq $statusProperty -or
                            [string]$statusProperty.Value -cne 'FAIL' -or
                            -not $script:LegacyFailureEvidenceSha256.ContainsKey($legacyRunId) -or
                            (Get-P1ASha256 -Path $evidencePath) -cne
                                [string]$script:LegacyFailureEvidenceSha256[$legacyRunId]) {
                            return $false
                        }
                    }
                }
            }
            catch {
                return $false
            }
        }
        $expectedPaths = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
        $previousPath = $null
        foreach ($line in $sealLines) {
            if ($line -notmatch '^([0-9a-f]{64})  ([^:\\]+(?:/[^:\\]+)*)$') {
                return $false
            }
            $hash = $Matches[1]
            $relative = $Matches[2]
            if ($relative -match '(^|/)\.\.?(/|$)' -or
                [IO.Path]::IsPathRooted($relative) -or
                -not $expectedPaths.Add($relative)) {
                return $false
            }
            if ($null -ne $previousPath -and
                [string]::CompareOrdinal($previousPath, $relative) -ge 0) {
                return $false
            }
            $previousPath = $relative
            $path = [IO.Path]::GetFullPath((Join-Path $root $relative.Replace('/', '\')))
            if (-not (Test-P1APathWithin -Path $path -Root $root) -or
                -not (Test-Path -LiteralPath $path -PathType Leaf) -or
                (Get-P1ASha256 -Path $path) -cne $hash) {
                return $false
            }
        }
        $actual = @(
            Get-ChildItem -LiteralPath $root -Recurse -File |
                Where-Object {
                    -not [string]::Equals($_.FullName, $sealPath, [StringComparison]::OrdinalIgnoreCase) -and
                        (-not $legacyEvidenceExclusion -or
                            -not [string]::Equals(
                                $_.FullName,
                                $evidencePath,
                                [StringComparison]::OrdinalIgnoreCase
                            ))
                } |
                ForEach-Object {
                    ConvertTo-P1ANormalizedPath -Path $_.FullName -BasePath $root
                }
        )
        if ($actual.Count -ne $expectedPaths.Count) {
            return $false
        }
        foreach ($relative in $actual) {
            if (-not $expectedPaths.Contains($relative)) {
                return $false
            }
        }
        return $true
    }
    catch {
        return $false
    }
}

function Assert-P1AClosedObject {
    param(
        [Parameter(Mandatory)]$Value,
        [Parameter(Mandatory)][string[]]$Fields,
        [Parameter(Mandatory)][string]$Name
    )

    if ($null -eq $Value) {
        throw "$Name is null"
    }
    $actual = if ($Value -is [Collections.IDictionary]) {
        @($Value.Keys | ForEach-Object { [string]$_ } | Sort-Object)
    }
    else {
        @($Value.PSObject.Properties.Name | Sort-Object)
    }
    $expected = @($Fields | Sort-Object)
    if (($actual -join $script:Lf) -cne ($expected -join $script:Lf)) {
        throw "$Name fields do not match the closed schema"
    }
}

function Assert-P1ASha256Value {
    param([AllowNull()][string]$Value, [Parameter(Mandatory)][string]$Name)
    if ($Value -cnotmatch '^[0-9a-f]{64}$') {
        throw "$Name is not a canonical SHA-256"
    }
}

function Resolve-P1ARunFile {
    param(
        [Parameter(Mandatory)][string]$RunRoot,
        [Parameter(Mandatory)][string]$RelativePath,
        [Parameter(Mandatory)][string]$Name
    )

    if ($RelativePath -notmatch '^[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)*$' -or
        $RelativePath -match '(^|/)\.\.?(/|$)' -or
        [IO.Path]::IsPathRooted($RelativePath)) {
        throw "$Name is not a contained canonical relative path"
    }
    $full = [IO.Path]::GetFullPath((Join-Path $RunRoot $RelativePath.Replace('/', '\')))
    if (-not (Test-P1APathWithin -Path $full -Root $RunRoot) -or
        -not (Test-Path -LiteralPath $full -PathType Leaf)) {
        throw "$Name does not resolve to a run file"
    }
    $cursor = $full
    while (Test-P1APathWithin -Path $cursor -Root $RunRoot) {
        $item = Get-Item -Force -LiteralPath $cursor
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "$Name resolves through a reparse point"
        }
        if ([string]::Equals(
                $cursor.TrimEnd('\', '/'),
                [IO.Path]::GetFullPath($RunRoot).TrimEnd('\', '/'),
                [StringComparison]::OrdinalIgnoreCase
            )) {
            break
        }
        $cursor = Split-Path -Parent $cursor
    }
    return $full
}

function Assert-P1AFileReference {
    param(
        [Parameter(Mandatory)]$Reference,
        [Parameter(Mandatory)][string]$RunRoot,
        [Parameter(Mandatory)][string]$Name
    )

    Assert-P1AClosedObject -Value $Reference -Fields @('path', 'sha256', 'bytes') -Name $Name
    Assert-P1ASha256Value -Value ([string]$Reference.sha256) -Name "$Name.sha256"
    $path = Resolve-P1ARunFile -RunRoot $RunRoot -RelativePath ([string]$Reference.path) -Name "$Name.path"
    if ((Get-P1ASha256 -Path $path) -cne [string]$Reference.sha256 -or
        [int64](Get-Item -LiteralPath $path).Length -ne [int64]$Reference.bytes) {
        throw "$Name hash or byte count does not match its run file"
    }
    return $path
}

function Assert-P1AToolDescriptor {
    param([Parameter(Mandatory)]$Value, [Parameter(Mandatory)][string]$Name)
    Assert-P1AClosedObject -Value $Value -Fields @('name', 'version', 'path', 'sha256') -Name $Name
    Assert-P1ASha256Value -Value ([string]$Value.sha256) -Name "$Name.sha256"
    if ([string]::IsNullOrWhiteSpace([string]$Value.version) -or
        [string]$Value.path -notmatch '^\$\{[A-Z0-9_]+\}(?:/[A-Za-z0-9_.+() -]+)*$') {
        throw "$Name contains an unversioned or non-tokenized tool identity"
    }
}

function Assert-P1APassRun {
    param(
        [Parameter(Mandatory)][string]$RunRoot,
        [Parameter(Mandatory)][string]$RunId,
        [Parameter(Mandatory)][string]$SourceIdentitySha256
    )

    if ($RunId -cnotmatch '^[0-9]{8}T[0-9]{9}Z-[0-9a-f]{24}$' -or
        (Split-Path -Leaf ([IO.Path]::GetFullPath($RunRoot))) -cne $RunId) {
        throw 'accepted run ID is not canonical'
    }
    if (-not (Test-Path -LiteralPath $RunRoot -PathType Container) -or
        (((Get-Item -Force -LiteralPath $RunRoot).Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)) {
        throw 'accepted run root is missing or is a reparse point'
    }
    Assert-P1ASha256Value -Value $SourceIdentitySha256 -Name 'source identity'
    if (-not (Test-P1ASeal -RunRoot $RunRoot)) {
        throw 'cannot publish acceptance for a run with an invalid seal'
    }

    $evidencePath = Join-Path $RunRoot 'evidence.json'
    $environmentPath = Join-Path $RunRoot 'artifacts\environment.json'
    $sourcePath = Join-Path $RunRoot 'artifacts\source-identity.json'
    $sealPath = Join-Path $RunRoot 'SHA256SUMS'
    foreach ($path in @($evidencePath, $environmentPath, $sourcePath, $sealPath)) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw 'accepted run is missing a required evidence artifact'
        }
    }
    if ((Get-P1ASha256 -Path $sourcePath) -cne $SourceIdentitySha256) {
        throw 'accepted source identity hash does not match the run artifact'
    }

    $evidence = [IO.File]::ReadAllText($evidencePath, $script:Utf8NoBom) | ConvertFrom-Json
    Assert-P1AClosedObject -Value $evidence -Fields @(
        'schema', 'phase_id', 'run_id', 'mode', 'status', 'started_at', 'finished_at',
        'duration_ms', 'invocation', 'source_identity_sha256', 'p0_dependency',
        'environment', 'commands', 'gates', 'errors', 'cleanup', 'seal'
    ) -Name 'run evidence'
    if ($evidence.schema -cne 'python-slm-phase-evidence-v1' -or
        $evidence.phase_id -cne 'P1A' -or $evidence.run_id -cne $RunId -or
        $evidence.mode -cne 'Cpu' -or $evidence.status -cne 'PASS') {
        throw 'only a schema-valid PASS P1A Cpu run can be accepted'
    }
    if ([string]$evidence.source_identity_sha256 -cne $SourceIdentitySha256) {
        throw 'run evidence source identity does not match the accepted source'
    }
    $parsedTime = [DateTime]::MinValue
    if (-not [DateTime]::TryParse([string]$evidence.started_at, [ref]$parsedTime) -or
        -not [DateTime]::TryParse([string]$evidence.finished_at, [ref]$parsedTime) -or
        [int64]$evidence.duration_ms -lt 0) {
        throw 'run evidence timestamps or duration are invalid'
    }

    Assert-P1AClosedObject -Value $evidence.invocation -Fields @('argv', 'cwd') -Name 'run invocation'
    $expectedArgv = @(
        'powershell', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
        'scripts/verify-env.ps1', '-Mode', 'Cpu', '-OutputRoot', 'docs/receipts/P1A'
    )
    if ($evidence.invocation.cwd -cne '${REPO}' -or
        (@($evidence.invocation.argv) -join [char]0) -cne ($expectedArgv -join [char]0)) {
        throw 'run evidence does not record the exact normative P1A invocation'
    }

    $expectedP0 = [ordered]@{
        status = 'PASS'
        receipt_commit = '86fb1e4cc68efeb651e5362c4aca85c2827d8e4d'
        receipt_sha256 = 'f08c6a41658ff287e238d6a96c4f2c874975964202c3eeced2bc0bc21f308904'
        contract_sha256 = 'fc2c60b52fdd7c524e0da06bb03972a4d523c21ad5536cba536185435bd44ad4'
        decision_ledger_sha256 = '8349d8a3e06d96d6921889de5534715e7b2f7439caf7e06558a97652a8890c8d'
    }
    Assert-P1AClosedObject -Value $evidence.p0_dependency -Fields @($expectedP0.Keys) -Name 'P0 dependency'
    foreach ($field in $expectedP0.Keys) {
        if ([string]$evidence.p0_dependency.$field -cne [string]$expectedP0[$field]) {
            throw "P0 dependency field is not approved: $field"
        }
    }

    if (@($evidence.errors).Count -ne 0) {
        throw 'a PASS run cannot contain errors'
    }
    Assert-P1AClosedObject -Value $evidence.cleanup -Fields @('attempted', 'temporary_root_removed') -Name 'run cleanup'
    if (-not [bool]$evidence.cleanup.attempted -or -not [bool]$evidence.cleanup.temporary_root_removed) {
        throw 'a PASS run must complete temporary cleanup'
    }

    $gateNames = @(
        'p0', 'input_stability', 'toolchain', 'native_probe', 'cpu_graph', 'clean_build',
        'quality_gate', 'python_isolation', 'cuda_isolation', 'redaction', 'cleanup'
    )
    Assert-P1AClosedObject -Value $evidence.gates -Fields $gateNames -Name 'run gates'
    foreach ($gateName in $gateNames) {
        $gate = $evidence.gates.$gateName
        Assert-P1AClosedObject -Value $gate -Fields @('status', 'detail') -Name "gate $gateName"
        if ($gate.status -cne 'PASS' -or [string]::IsNullOrWhiteSpace([string]$gate.detail)) {
            throw "gate did not pass: $gateName"
        }
    }

    $commands = @($evidence.commands)
    if ($commands.Count -eq 0) {
        throw 'a PASS run must contain command evidence'
    }
    $commandIds = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $commandById = @{}
    $commandPositions = @{}
    for ($commandIndex = 0; $commandIndex -lt $commands.Count; $commandIndex++) {
        $command = $commands[$commandIndex]
        Assert-P1AClosedObject -Value $command -Fields @(
            'id', 'argv', 'cwd', 'environment_delta_names', 'configuration_sha256',
            'exit_code', 'duration_ms', 'status', 'stdout', 'stderr'
        ) -Name 'command evidence'
        $expectedCommandId = 'C' + ($commandIndex + 1).ToString('00')
        if ([string]$command.id -cne $expectedCommandId -or
            -not $commandIds.Add([string]$command.id) -or
            $command.cwd -cne '${REPO}' -or $command.status -cne 'PASS' -or
            [int]$command.exit_code -ne 0 -or [int64]$command.duration_ms -lt 0 -or
            @($command.argv).Count -eq 0) {
            throw 'command evidence is incomplete, duplicated, or failed'
        }
        Assert-P1ASha256Value -Value ([string]$command.configuration_sha256) -Name 'command configuration'
        $stdoutPath = Assert-P1AFileReference -Reference $command.stdout -RunRoot $RunRoot -Name 'command stdout'
        $stderrPath = Assert-P1AFileReference -Reference $command.stderr -RunRoot $RunRoot -Name 'command stderr'
        if ([string]$command.stdout.path -cne "commands/$($command.id).stdout.txt" -or
            [string]$command.stderr.path -cne "commands/$($command.id).stderr.txt") {
            throw 'command transcript paths do not match their command ID'
        }
        [void]$stdoutPath
        [void]$stderrPath
        $commandPositions[[string]$command.id] = $commandIndex
        $commandById[[string]$command.id] = $command
    }

    $environmentFile = Assert-P1AFileReference -Reference $evidence.environment -RunRoot $RunRoot -Name 'environment manifest'
    if ([string]$evidence.environment.path -cne 'artifacts/environment.json') {
        throw 'environment manifest path is not canonical'
    }
    $environment = [IO.File]::ReadAllText($environmentFile, $script:Utf8NoBom) | ConvertFrom-Json
    Assert-P1AClosedObject -Value $environment -Fields @(
        'schema', 'phase_id', 'run_id', 'mode', 'status', 'p0', 'source', 'host',
        'rust', 'visual_studio', 'windows_sdk', 'isolation', 'native_probe',
        'quality_gate', 'cleanup'
    ) -Name 'environment manifest'
    if ($environment.schema -cne 'python-slm-environment-manifest-v1' -or
        $environment.phase_id -cne 'P1A' -or $environment.run_id -cne $RunId -or
        $environment.mode -cne 'Cpu' -or $environment.status -cne 'PASS') {
        throw 'environment manifest identity does not match the accepted run'
    }
    Assert-P1AClosedObject -Value $environment.p0 -Fields @('receipt_commit', 'receipt_sha256', 'contract_sha256', 'decision_ledger_sha256') -Name 'environment P0 identity'
    foreach ($field in @('receipt_commit', 'receipt_sha256', 'contract_sha256', 'decision_ledger_sha256')) {
        if ([string]$environment.p0.$field -cne [string]$expectedP0[$field]) {
            throw "environment P0 identity mismatch: $field"
        }
    }

    Assert-P1AClosedObject -Value $environment.source -Fields @(
        'head', 'dirty', 'input_manifest_sha256', 'cargo_lock_sha256',
        'verifier_sha256', 'schema_bundle_sha256'
    ) -Name 'environment source identity'
    Assert-P1AClosedObject -Value $environment.host -Fields @(
        'os_version', 'os_architecture', 'process_architecture',
        'powershell_edition', 'powershell_version'
    ) -Name 'environment host identity'
    Assert-P1AClosedObject -Value $environment.rust -Fields @(
        'rustc_version', 'commit_hash', 'commit_date', 'host', 'llvm_version',
        'rustc', 'cargo_version', 'cargo', 'rustup'
    ) -Name 'environment Rust identity'
    Assert-P1AClosedObject -Value $environment.visual_studio -Fields @(
        'discovery_method', 'query', 'instance_id', 'product_id', 'product_version',
        'installation_version', 'complete', 'launchable', 'reboot_required',
        'developer_environment_arguments', 'tools'
    ) -Name 'environment Visual Studio identity'
    Assert-P1AClosedObject -Value $environment.windows_sdk -Fields @(
        'version', 'ucrt_version', 'root', 'validated_paths', 'tools'
    ) -Name 'environment Windows SDK identity'
    Assert-P1AClosedObject -Value $environment.isolation -Fields @(
        'target_absent_before', 'target_outside_repository', 'offline',
        'incremental_disabled', 'wrappers_disabled', 'cargo_config_absent',
        'cleared_environment_names', 'canaries', 'canary_hits',
        'forbidden_feature_matches', 'forbidden_artifacts', 'forbidden_imports',
        'python_invoked', 'cuda_invoked'
    ) -Name 'environment isolation evidence'
    Assert-P1AClosedObject -Value $environment.native_probe -Fields @(
        'c_source_sha256', 'rust_source_sha256', 'command_ids', 'result', 'pe_imports'
    ) -Name 'environment native probe'
    Assert-P1AClosedObject -Value $environment.quality_gate -Fields @(
        'command_ids', 'result'
    ) -Name 'environment quality gate'
    Assert-P1AClosedObject -Value $environment.cleanup -Fields @(
        'temporary_root_removed', 'repository_target_unchanged',
        'parent_environment_unchanged', 'inputs_unchanged'
    ) -Name 'environment cleanup'

    $source = [IO.File]::ReadAllText($sourcePath, $script:Utf8NoBom) | ConvertFrom-Json
    if ([string]$environment.source.head -cne [string]$source.head -or
        [string]$environment.source.input_manifest_sha256 -cne [string]$source.input_manifest_sha256) {
        throw 'environment and source identities do not reconcile'
    }
    Assert-P1ASha256Value -Value ([string]$environment.source.cargo_lock_sha256) -Name 'Cargo.lock identity'
    Assert-P1ASha256Value -Value ([string]$environment.source.verifier_sha256) -Name 'verifier identity'
    Assert-P1ASha256Value -Value ([string]$environment.source.schema_bundle_sha256) -Name 'schema bundle identity'
    if ($environment.rust.host -cne 'x86_64-pc-windows-msvc' -or
        -not (Test-P1AVersionAtLeast -Actual ([string]$environment.rust.rustc_version) -Minimum '1.96.0')) {
        throw 'environment Rust identity is not qualified'
    }
    if ($environment.host.os_architecture -cne 'AMD64' -or
        $environment.host.process_architecture -cne 'AMD64' -or
        $environment.host.powershell_edition -cne 'Desktop' -or
        [string]$environment.host.powershell_version -notmatch '^5\.1(?:\.|$)') {
        throw 'environment host identity is not Windows PowerShell 5.1 AMD64'
    }
    Assert-P1AToolDescriptor -Value $environment.rust.rustc -Name 'rustc tool'
    Assert-P1AToolDescriptor -Value $environment.rust.cargo -Name 'Cargo tool'
    if ($null -ne $environment.rust.rustup) {
        Assert-P1AClosedObject -Value $environment.rust.rustup -Fields @(
            'version', 'active_toolchain', 'installed_target', 'components'
        ) -Name 'rustup identity'
        if ([string]::IsNullOrWhiteSpace([string]$environment.rust.rustup.version) -or
            [string]::IsNullOrWhiteSpace([string]$environment.rust.rustup.active_toolchain) -or
            -not [bool]$environment.rust.rustup.installed_target) {
            throw 'rustup identity is incomplete'
        }
    }

    if (-not [bool]$environment.visual_studio.complete -or
        -not [bool]$environment.visual_studio.launchable -or
        [bool]$environment.visual_studio.reboot_required -or
        ([version][string]$environment.visual_studio.installation_version).Major -ne 17) {
        throw 'Visual Studio identity is not a qualified VS2022 instance'
    }
    $expectedVsQuery = @(
        '-version', '[17.0,18.0)', '-products', '*', '-requires',
        'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-format', 'json', '-utf8'
    )
    $expectedVsArguments = @('-arch=amd64', '-host_arch=amd64', '-startdir=none')
    if ($environment.visual_studio.discovery_method -cne 'vswhere' -or
        (@($environment.visual_studio.query) -join [char]0) -cne ($expectedVsQuery -join [char]0) -or
        (@($environment.visual_studio.developer_environment_arguments) -join [char]0) -cne
            ($expectedVsArguments -join [char]0)) {
        throw 'Visual Studio discovery or developer-environment arguments are not canonical'
    }
    $vsNames = @($environment.visual_studio.tools | ForEach-Object {
        Assert-P1AToolDescriptor -Value $_ -Name 'Visual Studio tool'
        [string]$_.name
    })
    $expectedVsNames = @('cl', 'link', 'lib', 'dumpbin')
    if (($vsNames -join [char]0) -cne ($expectedVsNames -join [char]0)) {
        throw 'Visual Studio tool identities are incomplete, duplicated, or out of order'
    }
    $sdkNames = @($environment.windows_sdk.tools | ForEach-Object {
        Assert-P1AToolDescriptor -Value $_ -Name 'Windows SDK tool'
        [string]$_.name
    })
    if (($sdkNames -join [char]0) -cne ((@('rc', 'mt')) -join [char]0)) {
        throw 'Windows SDK tool identities are incomplete, duplicated, or out of order'
    }
    $expectedSdkPaths = @(
        ('${WINDOWS_KITS}/Include/' + [string]$environment.windows_sdk.version + '/um/Windows.h')
        ('${WINDOWS_KITS}/Include/' + [string]$environment.windows_sdk.ucrt_version + '/ucrt/stdlib.h')
        ('${WINDOWS_KITS}/Lib/' + [string]$environment.windows_sdk.version + '/um/x64/kernel32.lib')
        ('${WINDOWS_KITS}/Lib/' + [string]$environment.windows_sdk.ucrt_version + '/ucrt/x64/ucrt.lib')
        ('${WINDOWS_KITS}/bin/' + [string]$environment.windows_sdk.version + '/x64/rc.exe')
        ('${WINDOWS_KITS}/bin/' + [string]$environment.windows_sdk.version + '/x64/mt.exe')
    )
    if ($environment.windows_sdk.root -cne '${WINDOWS_KITS}' -or
        (@($environment.windows_sdk.validated_paths) -join [char]0) -cne
            ($expectedSdkPaths -join [char]0)) {
        throw 'Windows SDK file validation identity is incomplete or noncanonical'
    }

    foreach ($field in @(
            'target_absent_before', 'target_outside_repository', 'offline',
            'incremental_disabled', 'wrappers_disabled', 'cargo_config_absent'
        )) {
        if (-not [bool]$environment.isolation.$field) {
            throw "CPU isolation assertion is false: $field"
        }
    }
    if (@($environment.isolation.canary_hits).Count -ne 0 -or
        @($environment.isolation.forbidden_feature_matches).Count -ne 0 -or
        @($environment.isolation.forbidden_artifacts).Count -ne 0 -or
        @($environment.isolation.forbidden_imports).Count -ne 0 -or
        [bool]$environment.isolation.python_invoked -or [bool]$environment.isolation.cuda_invoked) {
        throw 'CPU isolation evidence contains a forbidden hit'
    }
    if ($environment.native_probe.result -cne 'PASS' -or
        $environment.quality_gate.result -cne 'PASS') {
        throw 'native probe or quality gate did not pass'
    }
    if (@($environment.native_probe.command_ids).Count -ne 6 -or
        @($environment.quality_gate.command_ids).Count -ne 4) {
        throw 'native probe or quality-gate command coverage is incomplete'
    }
    foreach ($commandId in @($environment.native_probe.command_ids) + @($environment.quality_gate.command_ids)) {
        if (-not $commandIds.Contains([string]$commandId)) {
            throw "environment references an unknown command ID: $commandId"
        }
    }
    foreach ($group in @(
            , @($environment.native_probe.command_ids)
            , @($environment.quality_gate.command_ids)
        )) {
        $priorPosition = -1
        foreach ($commandId in @($group)) {
            $position = [int]$commandPositions[[string]$commandId]
            if ($position -ne ($priorPosition + 1) -and $priorPosition -ge 0) {
                throw 'environment command IDs are not in canonical consecutive order'
            }
            $priorPosition = $position
        }
    }
    $nativeCommands = @($environment.native_probe.command_ids | ForEach-Object {
            $commandById[[string]$_]
        })
    $nativeExecutableNames = @($nativeCommands | ForEach-Object {
            [IO.Path]::GetFileName([string]$_.argv[0]).ToLowerInvariant()
        })
    if (($nativeExecutableNames -join [char]0) -cne
        ((@('cl.exe', 'cl.exe', 'lib.exe', 'rustc.exe', 'native_probe.exe', 'dumpbin.exe')) -join [char]0) -or
        (@($nativeCommands[0].argv) -join ' ') -notmatch '(?i)tool_canary\.c' -or
        (@($nativeCommands[1].argv) -join ' ') -notmatch '(?i)native_probe\.c' -or
        (@($nativeCommands[2].argv) -join ' ') -notmatch '(?i)native_probe\.lib' -or
        (@($nativeCommands[3].argv) -join ' ') -notmatch '(?i)--edition=2024.*--target\s+x86_64-pc-windows-msvc' -or
        @($nativeCommands[4].argv).Count -ne 1 -or
        (@($nativeCommands[5].argv) -join ' ') -notmatch '(?i)^dumpbin\.exe\s+/DEPENDENTS\s+') {
        throw 'native probe command evidence does not match the qualified ABI sequence'
    }
    $qualityCommands = @($environment.quality_gate.command_ids | ForEach-Object {
            $commandById[[string]$_]
        })
    $expectedQualityArgv = @(
        , @('cargo.exe', 'test', '--locked', '--offline', '--no-default-features',
            '--features', 'cpu-reference', '--target', 'x86_64-pc-windows-msvc',
            '--no-run', '--message-format=json-render-diagnostics')
        , @('cargo.exe', 'fmt', '--all', '--', '--check')
        , @('cargo.exe', 'clippy', '--locked', '--all-targets', '--features',
            'cpu-reference', '--', '-D', 'warnings')
        , @('cargo.exe', 'test', '--locked', '--features', 'cpu-reference')
    )
    for ($qualityIndex = 0; $qualityIndex -lt $expectedQualityArgv.Count; $qualityIndex++) {
        if ((@($qualityCommands[$qualityIndex].argv) -join [char]0) -cne
            (@($expectedQualityArgv[$qualityIndex]) -join [char]0)) {
            throw 'quality-gate command evidence does not match the exact approved argv sequence'
        }
    }
    foreach ($field in @(
            'temporary_root_removed', 'repository_target_unchanged',
            'parent_environment_unchanged', 'inputs_unchanged'
        )) {
        if (-not [bool]$environment.cleanup.$field) {
            throw "environment cleanup assertion is false: $field"
        }
    }

    Assert-P1AClosedObject -Value $evidence.seal -Fields @('path', 'entries', 'coverage_rule') -Name 'run seal reference'
    if ($evidence.seal.path -cne 'SHA256SUMS' -or
        $evidence.seal.coverage_rule -cne 'all_run_files_except_seal' -or
        [int]$evidence.seal.entries -ne @([IO.File]::ReadAllLines($sealPath, $script:Utf8NoBom)).Count) {
        throw 'run seal reference does not match the immutable run'
    }

    return [pscustomobject][ordered]@{
        evidence_path = $evidencePath
        evidence_sha256 = Get-P1ASha256 -Path $evidencePath
        environment_path = $environmentPath
        environment_sha256 = Get-P1ASha256 -Path $environmentPath
        seal_path = $sealPath
        seal_sha256 = Get-P1ASha256 -Path $sealPath
    }
}

function Assert-P1AAcceptanceObject {
    param(
        [Parameter(Mandatory)]$Acceptance,
        [Parameter(Mandatory)][int]$Sequence,
        [AllowNull()][string]$PreviousHash,
        [Parameter(Mandatory)][string]$OutputRoot
    )

    Assert-P1AClosedObject -Value $Acceptance -Fields @(
        'schema', 'phase_id', 'sequence', 'status', 'acceptance_kind',
        'required_approvals', 'run_path', 'run_evidence_sha256', 'seal_path',
        'seal_sha256', 'environment_path', 'environment_sha256',
        'source_identity_sha256', 'previous_acceptance_sha256', 'created_at'
    ) -Name 'acceptance record'
    if ($Acceptance.schema -cne 'python-slm-phase-acceptance-v1' -or
        $Acceptance.phase_id -cne 'P1A' -or [int]$Acceptance.sequence -ne $Sequence -or
        $Acceptance.status -cne 'PASS' -or
        $Acceptance.acceptance_kind -cne 'automatic_machine_qualification' -or
        @($Acceptance.required_approvals).Count -ne 0 -or
        [string]$Acceptance.previous_acceptance_sha256 -cne [string]$PreviousHash) {
        throw 'acceptance record does not match the automatic qualification chain'
    }
    if ([string]$Acceptance.run_path -cnotmatch '^runs/(?<run>[0-9]{8}T[0-9]{9}Z-[0-9a-f]{24})$') {
        throw 'acceptance run path is not canonical'
    }
    $runId = $Matches['run']
    $runRoot = Join-Path (Join-Path $OutputRoot 'runs') $runId
    $validated = Assert-P1APassRun -RunRoot $runRoot -RunId $runId `
        -SourceIdentitySha256 ([string]$Acceptance.source_identity_sha256)
    if ([string]$Acceptance.run_evidence_sha256 -cne $validated.evidence_sha256 -or
        [string]$Acceptance.seal_sha256 -cne $validated.seal_sha256 -or
        [string]$Acceptance.environment_sha256 -cne $validated.environment_sha256 -or
        [string]$Acceptance.seal_path -cne "runs/$runId/SHA256SUMS" -or
        [string]$Acceptance.environment_path -cne "runs/$runId/artifacts/environment.json") {
        throw 'acceptance record hashes or paths do not match its run'
    }
}

function Publish-P1AAcceptance {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$OutputRoot,
        [Parameter(Mandatory)][string]$RunId,
        [Parameter(Mandatory)][string]$SourceIdentitySha256
    )

    $root = [IO.Path]::GetFullPath($OutputRoot)
    if ($RunId -cnotmatch '^[0-9]{8}T[0-9]{9}Z-[0-9a-f]{24}$') {
        throw 'acceptance run ID is not canonical'
    }
    $runsRoot = Join-Path $root 'runs'
    $runRoot = [IO.Path]::GetFullPath((Join-Path $runsRoot $RunId))
    if (-not (Test-P1APathWithin -Path $runRoot -Root $runsRoot)) {
        throw 'acceptance run is outside the runs directory'
    }
    $validatedRun = Assert-P1APassRun -RunRoot $runRoot -RunId $RunId `
        -SourceIdentitySha256 $SourceIdentitySha256

    $mutexNameHash = Get-P1ATextSha256 -Text $root.ToLowerInvariant()
    $mutex = [Threading.Mutex]::new($false, "Local\python-slm-p1a-$mutexNameHash")
    $locked = $false
    $publicationCommitted = $false
    try {
        $locked = $mutex.WaitOne([TimeSpan]::FromSeconds(30))
        if (-not $locked) {
            throw 'timed out waiting for the P1A publication lock'
        }
        # Revalidate after acquiring the cross-process lock so no run or chain
        # bytes can change between validation and publication.
        $validatedRun = Assert-P1APassRun -RunRoot $runRoot -RunId $RunId `
            -SourceIdentitySha256 $SourceIdentitySha256

        $acceptanceRoot = Join-Path $root 'acceptances'
        [void][IO.Directory]::CreateDirectory($acceptanceRoot)
        if ((((Get-Item -Force -LiteralPath $acceptanceRoot).Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)) {
            throw 'acceptance directory is a reparse point'
        }
        $allAcceptanceFiles = @(Get-ChildItem -LiteralPath $acceptanceRoot -File -ErrorAction Stop | Sort-Object Name)
        foreach ($file in $allAcceptanceFiles) {
            if ($file.Name -cnotmatch '^[0-9]{8}\.json$') {
                throw 'acceptance directory contains a noncanonical file'
            }
        }
        $previousHash = $null
        for ($index = 0; $index -lt $allAcceptanceFiles.Count; $index++) {
            $expectedSequence = $index + 1
            if ($allAcceptanceFiles[$index].BaseName -cne $expectedSequence.ToString('00000000')) {
                throw 'acceptance sequence contains a gap or duplicate'
            }
            $prior = [IO.File]::ReadAllText($allAcceptanceFiles[$index].FullName, $script:Utf8NoBom) | ConvertFrom-Json
            Assert-P1AAcceptanceObject -Acceptance $prior -Sequence $expectedSequence `
                -PreviousHash $previousHash -OutputRoot $root
            $previousHash = Get-P1ASha256 -Path $allAcceptanceFiles[$index].FullName
        }

        $pointerPath = Join-Path $root 'evidence.json'
        if (Test-Path -LiteralPath $pointerPath -PathType Leaf) {
            if ($allAcceptanceFiles.Count -eq 0) {
                throw 'selected pointer exists without an acceptance chain'
            }
            $oldPointer = [IO.File]::ReadAllText($pointerPath, $script:Utf8NoBom) | ConvertFrom-Json
            Assert-P1AClosedObject -Value $oldPointer -Fields @(
                'schema', 'phase_id', 'acceptance_path', 'acceptance_sha256', 'updated_at'
            ) -Name 'selected pointer'
            $selectedRelative = [string]$oldPointer.acceptance_path
            if ($selectedRelative -cnotmatch '^acceptances/(?<name>[0-9]{8}\.json)$') {
                throw 'selected pointer acceptance path is not canonical'
            }
            $selectedExisting = Join-Path $acceptanceRoot $Matches['name']
            if ($oldPointer.schema -cne 'python-slm-phase-evidence-pointer-v1' -or
                $oldPointer.phase_id -cne 'P1A' -or
                -not (Test-Path -LiteralPath $selectedExisting -PathType Leaf) -or
                [string]$oldPointer.acceptance_sha256 -cne (Get-P1ASha256 -Path $selectedExisting)) {
                throw 'selected pointer does not match a valid acceptance in the immutable chain'
            }
        }

        $sequence = $allAcceptanceFiles.Count + 1
        $acceptanceRelative = 'acceptances/' + $sequence.ToString('00000000') + '.json'
        $acceptancePath = Join-Path $root $acceptanceRelative.Replace('/', '\')
        $acceptance = [ordered]@{
            schema = 'python-slm-phase-acceptance-v1'
            phase_id = 'P1A'
            sequence = $sequence
            status = 'PASS'
            acceptance_kind = 'automatic_machine_qualification'
            required_approvals = @()
            run_path = "runs/$RunId"
            run_evidence_sha256 = $validatedRun.evidence_sha256
            seal_path = "runs/$RunId/SHA256SUMS"
            seal_sha256 = $validatedRun.seal_sha256
            environment_path = "runs/$RunId/artifacts/environment.json"
            environment_sha256 = $validatedRun.environment_sha256
            source_identity_sha256 = $SourceIdentitySha256
            previous_acceptance_sha256 = $previousHash
            created_at = [DateTime]::UtcNow.ToString('o')
        }
        Write-P1AJsonFile -Path $acceptancePath -Value $acceptance -CreateNew
        $acceptanceHash = Get-P1ASha256 -Path $acceptancePath
        Assert-P1AAcceptanceObject -Acceptance $acceptance -Sequence $sequence `
            -PreviousHash $previousHash -OutputRoot $root

        $pointer = [ordered]@{
            schema = 'python-slm-phase-evidence-pointer-v1'
            phase_id = 'P1A'
            acceptance_path = $acceptanceRelative
            acceptance_sha256 = $acceptanceHash
            updated_at = [DateTime]::UtcNow.ToString('o')
        }
        $temporaryPointer = Join-Path $root ('.evidence.' + $RunId + '.tmp')
        Write-P1AJsonFile -Path $temporaryPointer -Value $pointer -CreateNew
        $backupPointer = $null
        try {
            if (Test-Path -LiteralPath $pointerPath -PathType Leaf) {
                $backupPointer = Join-Path $root ('.evidence.' + $RunId + '.bak')
                [IO.File]::Replace($temporaryPointer, $pointerPath, $backupPointer, $true)
            }
            else {
                [IO.File]::Move($temporaryPointer, $pointerPath)
            }
        }
        catch {
            if (Test-Path -LiteralPath $temporaryPointer -PathType Leaf) {
                Remove-Item -LiteralPath $temporaryPointer -Force -ErrorAction SilentlyContinue
            }
            throw
        }

        try {
            $verifiedPointer = [IO.File]::ReadAllText($pointerPath, $script:Utf8NoBom) | ConvertFrom-Json
            Assert-P1AClosedObject -Value $verifiedPointer -Fields @(
                'schema', 'phase_id', 'acceptance_path', 'acceptance_sha256', 'updated_at'
            ) -Name 'published pointer'
            $selectedPath = Join-Path $root ([string]$verifiedPointer.acceptance_path).Replace('/', '\')
            if ($verifiedPointer.schema -cne 'python-slm-phase-evidence-pointer-v1' -or
                $verifiedPointer.phase_id -cne 'P1A' -or
                [string]$verifiedPointer.acceptance_path -cne $acceptanceRelative -or
                [string]$verifiedPointer.acceptance_sha256 -cne $acceptanceHash -or
                -not (Test-P1APathWithin -Path $selectedPath -Root $acceptanceRoot) -or
                (Get-P1ASha256 -Path $selectedPath) -cne $acceptanceHash) {
                throw 'published P1A pointer failed complete chain verification'
            }
            $selectedAcceptance = [IO.File]::ReadAllText($selectedPath, $script:Utf8NoBom) | ConvertFrom-Json
            Assert-P1AAcceptanceObject -Acceptance $selectedAcceptance -Sequence $sequence `
                -PreviousHash $previousHash -OutputRoot $root
            if ($null -ne $backupPointer -and (Test-Path -LiteralPath $backupPointer -PathType Leaf)) {
                Remove-Item -LiteralPath $backupPointer -Force -ErrorAction Stop
            }
            $publicationCommitted = $true
        }
        catch {
            if ($null -ne $backupPointer -and (Test-Path -LiteralPath $backupPointer -PathType Leaf)) {
                $failedPointerBackup = Join-Path $root ('.evidence.' + $RunId + '.failed')
                [IO.File]::Replace($backupPointer, $pointerPath, $failedPointerBackup, $true)
                if (Test-Path -LiteralPath $failedPointerBackup -PathType Leaf) {
                    Remove-Item -LiteralPath $failedPointerBackup -Force
                }
            }
            elseif (Test-Path -LiteralPath $pointerPath -PathType Leaf) {
                Remove-Item -LiteralPath $pointerPath -Force
            }
            throw
        }
        return [pscustomobject][ordered]@{
            sequence = $sequence
            acceptance_path = $acceptanceRelative
            acceptance_sha256 = $acceptanceHash
            pointer_path = 'evidence.json'
        }
    }
    finally {
        if ($locked) {
            try { [void]$mutex.ReleaseMutex() } catch {
                if (-not $publicationCommitted) {
                    # Do not replace the primary publication error with a lock-cleanup error.
                }
            }
        }
        try { $mutex.Dispose() } catch { }
    }
}

function New-P1AFailureException {
    param(
        [Parameter(Mandatory)][string]$Code,
        [Parameter(Mandatory)][ValidateRange(1, 5)][int]$Category,
        [Parameter(Mandatory)][string]$Message,
        [Parameter(Mandatory)][string]$Remediation
    )

    $exception = [InvalidOperationException]::new($Message)
    $exception.Data['P1ACode'] = $Code
    $exception.Data['P1ACategory'] = $Category
    $exception.Data['P1ARemediation'] = $Remediation
    return $exception
}

function Throw-P1AFailure {
    param(
        [Parameter(Mandatory)][string]$Code,
        [Parameter(Mandatory)][ValidateRange(1, 5)][int]$Category,
        [Parameter(Mandatory)][string]$Message,
        [Parameter(Mandatory)][string]$Remediation
    )
    throw (New-P1AFailureException -Code $Code -Category $Category -Message $Message -Remediation $Remediation)
}

function Get-P1AEnvironmentIdentity {
    $lines = [Collections.Generic.List[string]]::new()
    foreach ($entry in (Get-P1ACanonicalEnvironment).GetEnumerator() | Sort-Object Key) {
        $lines.Add($entry.Key + '=' + $entry.Value)
    }
    return Get-P1ATextSha256 -Text ($lines -join $script:Lf)
}

function Get-P1ADirectoryIdentity {
    param([Parameter(Mandatory)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Container)) {
        return Get-P1ATextSha256 -Text 'absent'
    }
    $root = [IO.Path]::GetFullPath($Path)
    $lines = @(
        Get-ChildItem -LiteralPath $root -Recurse -Force -File -ErrorAction Stop |
            ForEach-Object {
                $relative = ConvertTo-P1ANormalizedPath -Path $_.FullName -BasePath $root
                "$relative|$($_.Length)|$($_.LastWriteTimeUtc.Ticks)"
            } |
            Sort-Object
    )
    return Get-P1ATextSha256 -Text ($lines -join $script:Lf)
}

function Resolve-P1AExecutable {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][Collections.IDictionary]$Environment
    )

    $pathValue = [string]$Environment['Path']
    if ([string]::IsNullOrWhiteSpace($pathValue)) {
        return $null
    }
    foreach ($directory in $pathValue.Split(';')) {
        if ([string]::IsNullOrWhiteSpace($directory)) {
            continue
        }
        $candidate = Join-Path $directory $Name
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            return [IO.Path]::GetFullPath($candidate)
        }
    }
    return $null
}

function ConvertTo-P1ARolePath {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][object[]]$RoleRoots
    )

    $full = [IO.Path]::GetFullPath($Path)
    foreach ($role in @($RoleRoots | Sort-Object { $_.Path.Length } -Descending)) {
        if ([string]::IsNullOrWhiteSpace([string]$role.Path)) {
            continue
        }
        if (Test-P1APathWithin -Path $full -Root $role.Path) {
            $relative = ConvertTo-P1ANormalizedPath -Path $full -BasePath $role.Path
            if ($relative -eq '.') {
                return '$' + '{' + $role.Name + '}'
            }
            return '$' + '{' + $role.Name + '}/' + $relative
        }
    }
    throw "tool path has no approved normalization role: $Path"
}

function Save-P1ACommandResult {
    param(
        [Parameter(Mandatory)][hashtable]$Context,
        [Parameter(Mandatory)][string]$Id,
        [Parameter(Mandatory)][string]$DisplayFile,
        [Parameter(Mandatory)][string[]]$Arguments,
        [Parameter(Mandatory)]$Result,
        [string[]]$EnvironmentDeltaNames = @()
    )

    $safeOut = Protect-P1AText -Text ([string]$Result.stdout) -RepositoryRoot $Context.RepositoryRoot -ExtraRoots $Context.RedactionRoots
    $safeErr = Protect-P1AText -Text ([string]$Result.stderr) -RepositoryRoot $Context.RepositoryRoot -ExtraRoots $Context.RedactionRoots
    $safeArgv = @(
        @($DisplayFile) + $Arguments | ForEach-Object {
            Protect-P1AText -Text ([string]$_) -RepositoryRoot $Context.RepositoryRoot -ExtraRoots $Context.RedactionRoots
        }
    )
    foreach ($candidate in @($safeOut, $safeErr) + $safeArgv) {
        if ($candidate -match '(?i)(?:[a-z]:[\\/]|\\\\[^\\])') {
            Throw-P1AFailure -Code 'REDACTION_FAILED' -Category 3 -Message "absolute path remained after redaction for command $Id" -Remediation 'Extend the verifier role-root map without recording machine-local paths.'
        }
    }
    $stdoutRelative = "commands/$Id.stdout.txt"
    $stderrRelative = "commands/$Id.stderr.txt"
    $stdoutPath = Join-Path $Context.RunRoot $stdoutRelative.Replace('/', '\')
    $stderrPath = Join-Path $Context.RunRoot $stderrRelative.Replace('/', '\')
    Write-P1AUtf8LfFile -Path $stdoutPath -Text $safeOut -CreateNew
    Write-P1AUtf8LfFile -Path $stderrPath -Text $safeErr -CreateNew
    $record = [ordered]@{
        id = $Id
        argv = $safeArgv
        cwd = '$' + '{REPO}'
        environment_delta_names = @($EnvironmentDeltaNames | Sort-Object -Unique)
        configuration_sha256 = Get-P1ATextSha256 -Text (([ordered]@{
                argv = $safeArgv
                cwd = '$' + '{REPO}'
                environment_delta_names = @($EnvironmentDeltaNames | Sort-Object -Unique)
            } | ConvertTo-Json -Depth 8 -Compress))
        exit_code = [int]$Result.exit_code
        duration_ms = [int64]$Result.duration_ms
        status = if ([int]$Result.exit_code -eq 0 -and -not [bool]$Result.timed_out) { 'PASS' } else { 'FAIL' }
        stdout = [ordered]@{
            path = $stdoutRelative
            sha256 = Get-P1ASha256 -Path $stdoutPath
            bytes = (Get-Item -LiteralPath $stdoutPath).Length
        }
        stderr = [ordered]@{
            path = $stderrRelative
            sha256 = Get-P1ASha256 -Path $stderrPath
            bytes = (Get-Item -LiteralPath $stderrPath).Length
        }
    }
    $Context.Commands.Add($record)
    return $record
}

function Invoke-P1ARecorded {
    param(
        [Parameter(Mandatory)][hashtable]$Context,
        [Parameter(Mandatory)][string]$Id,
        [Parameter(Mandatory)][string]$FilePath,
        [Parameter(Mandatory)][string[]]$Arguments,
        [Parameter(Mandatory)][Collections.IDictionary]$Environment,
        [int]$TimeoutSeconds = 120,
        [string[]]$EnvironmentDeltaNames = @(),
        [switch]$AllowFailure
    )

    $overlay = @{}
    foreach ($entry in $Environment.GetEnumerator()) {
        $overlay[[string]$entry.Key] = $entry.Value
    }
    $result = Invoke-P1AProcess -FilePath $FilePath -ArgumentList $Arguments -WorkingDirectory $Context.RepositoryRoot -Environment $overlay -TimeoutSeconds $TimeoutSeconds
    [void](Save-P1ACommandResult -Context $Context -Id $Id -DisplayFile $FilePath -Arguments $Arguments -Result $result -EnvironmentDeltaNames $EnvironmentDeltaNames)
    if (-not $AllowFailure -and ([bool]$result.timed_out -or [int]$result.exit_code -ne 0)) {
        $code = if ([bool]$result.timed_out) { 'COMMAND_TIMEOUT' } else { 'COMMAND_FAILED' }
        Throw-P1AFailure -Code $code -Category 5 -Message "qualification command failed: $Id" -Remediation "Inspect commands/$Id.stderr.txt in the immutable failed run."
    }
    return $result
}

function Get-P1AInputManifestLegacy {
    param(
        [Parameter(Mandatory)][string]$RepositoryRoot,
        [Parameter(Mandatory)][string]$OutputRoot,
        [Parameter(Mandatory)][string]$GitPath
    )

    $result = Invoke-P1AProcess -FilePath $GitPath -ArgumentList @(
        'ls-files', '--cached', '--others', '--exclude-standard'
    ) -WorkingDirectory $RepositoryRoot -Environment @{} -TimeoutSeconds 120
    if ($result.exit_code -ne 0) {
        throw 'git ls-files failed while building the P1A input manifest'
    }
    $lines = [Collections.Generic.List[string]]::new()
    foreach ($relative in @($result.stdout -split '\r?\n' | Where-Object { $_ })) {
        $normalized = $relative.Replace('\', '/')
        if ($normalized.StartsWith('docs/receipts/P1A/', [StringComparison]::OrdinalIgnoreCase)) {
            continue
        }
        $path = [IO.Path]::GetFullPath((Join-Path $RepositoryRoot $relative))
        if (-not (Test-P1APathWithin -Path $path -Root $RepositoryRoot) -or
            -not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "input manifest path is invalid: $relative"
        }
        $lines.Add("$(Get-P1ASha256 -Path $path)  $normalized")
    }
    $ordered = @($lines | Sort-Object)
    return [pscustomobject][ordered]@{
        sha256 = Get-P1ATextSha256 -Text ($ordered -join $script:Lf)
        entries = $ordered.Count
    }
}

function Test-P1AP0Dependency {
    param(
        [Parameter(Mandatory)][hashtable]$Context,
        [Parameter(Mandatory)][string]$GitPath,
        [Parameter(Mandatory)][Collections.IDictionary]$Environment
    )

    $baseline = 'b1ebb455cdae94bbb9fc54f246cdf2758eedf1d1'
    $receiptCommit = '86fb1e4cc68efeb651e5362c4aca85c2827d8e4d'
    $receiptHash = 'f08c6a41658ff287e238d6a96c4f2c874975964202c3eeced2bc0bc21f308904'
    $contractHash = 'fc2c60b52fdd7c524e0da06bb03972a4d523c21ad5536cba536185435bd44ad4'
    $ledgerHash = '8349d8a3e06d96d6921889de5534715e7b2f7439caf7e06558a97652a8890c8d'
    $sealed = @(
        'docs/rebuild-contract.md',
        'docs/receipts/P0/capture.ps1',
        'docs/receipts/P0/evidence.json',
        'docs/receipts/P0/runs'
    )
    $c01 = Invoke-P1ARecorded -Context $Context -Id 'C01' -FilePath $GitPath -Arguments (@('diff', '--exit-code', $baseline, '--') + $sealed) -Environment $Environment
    $c02 = Invoke-P1ARecorded -Context $Context -Id 'C02' -FilePath $GitPath -Arguments (@('status', '--porcelain=v1', '--untracked-files=all', '--') + $sealed) -Environment $Environment
    if (-not [string]::IsNullOrWhiteSpace($c02.stdout)) {
        Throw-P1AFailure -Code 'P0_DEPENDENCY_INVALID' -Category 3 -Message 'sealed P0 paths are dirty' -Remediation 'Restore the approved P0 bytes before running P1A.'
    }
    [void](Invoke-P1ARecorded -Context $Context -Id 'C03' -FilePath $GitPath -Arguments @('merge-base', '--is-ancestor', $receiptCommit, 'HEAD') -Environment $Environment)
    [void](Invoke-P1ARecorded -Context $Context -Id 'C04' -FilePath $GitPath -Arguments @('diff', '--exit-code', $receiptCommit, '--', 'docs/receipts/P0.md') -Environment $Environment)

    $receiptPath = Join-Path $Context.RepositoryRoot 'docs\receipts\P0.md'
    $contractPath = Join-Path $Context.RepositoryRoot 'docs\rebuild-contract.md'
    if ((Get-P1ASha256 -Path $receiptPath) -cne $receiptHash -or
        (Get-P1ASha256 -Path $contractPath) -cne $contractHash) {
        Throw-P1AFailure -Code 'P0_DEPENDENCY_INVALID' -Category 3 -Message 'approved P0 receipt or contract hash changed' -Remediation 'Restore the approved Phase 0 files.'
    }
    $receipt = [IO.File]::ReadAllText($receiptPath, $script:Utf8NoBom)
    foreach ($requiredPattern in @(
            '(?m)^Status:[ \t]+\*\*PASS\*\*[ \t]*$',
            '(?m)^Technical approval:[ \t]+\*\*APPROVED\*\*[ \t]*$',
            '(?m)^Data-governance approval:[ \t]+\*\*APPROVED\*\*[ \t]*$',
            [regex]::Escape($ledgerHash)
        )) {
        if ($receipt -notmatch $requiredPattern) {
            Throw-P1AFailure -Code 'P0_DEPENDENCY_INVALID' -Category 3 -Message 'approved P0 receipt is incomplete or contradictory' -Remediation 'Restore the signed Phase 0 receipt.'
        }
    }
    $runRoot = Join-Path $Context.RepositoryRoot 'docs\receipts\P0\runs\20260811T074740Z-d5008e94'
    $sealPath = Join-Path $runRoot 'SHA256SUMS'
    foreach ($line in [IO.File]::ReadAllLines($sealPath, $script:Utf8NoBom)) {
        if ($line -notmatch '^([0-9a-f]{64})  (.+)$') {
            throw 'invalid sealed P0 manifest line'
        }
        $path = Join-Path $runRoot $Matches[2].Replace('/', '\')
        if ((Get-P1ASha256 -Path $path) -cne $Matches[1]) {
            Throw-P1AFailure -Code 'P0_DEPENDENCY_INVALID' -Category 3 -Message 'P0 machine-evidence seal failed' -Remediation 'Restore the approved immutable P0 run.'
        }
    }
    return [ordered]@{
        status = 'PASS'
        receipt_commit = $receiptCommit
        receipt_sha256 = $receiptHash
        contract_sha256 = $contractHash
        decision_ledger_sha256 = $ledgerHash
    }
}

function Add-P1AError {
    param(
        [Parameter(Mandatory)][Collections.IList]$Errors,
        [Parameter(Mandatory)][string]$Code,
        [Parameter(Mandatory)][string]$Message,
        [Parameter(Mandatory)][string]$Remediation,
        [ValidateRange(1, 5)][int]$Category = 2
    )
    [void]$Errors.Add([ordered]@{
        code = $Code
        category = $Category
        message = $Message
        remediation = $Remediation
    })
}

function Get-P1AErrorCategory {
    param([Parameter(Mandatory)][string]$Code)
    if ($Code -match '^(P0_|INPUT_|CARGO_LOCK_|CARGO_CONFIG_|REPOSITORY_|PARENT_|REDACTION_|UNSAFE_|CHILD_ENVIRONMENT_)') {
        return 3
    }
    if ($Code -match '^(UNSUPPORTED_|TOOL_|RUST_|CARGO_VERSION_|VS_|VSWHERE_|VSDEVCMD_|MSVC_TOOL_|WINDOWS_SDK_|GIT_|TARGET_NOT_CLEAN|TEMP_PATH_)') {
        return 4
    }
    if ($Code -match '^(MODE_NOT_IMPLEMENTED|COMMAND_|NATIVE_|CANARY_|CPU_|CLEAN_|CARGO_METADATA_|CARGO_TREE_|CARGO_MESSAGE_|CARGO_ARTIFACT_|CUDA_|PYTHON_|PE_|EVIDENCE_|TEMP_CLEANUP_)') {
        return 5
    }
    return 1
}

function New-P1AGates {
    $gates = [ordered]@{}
    foreach ($name in @('p0', 'input_stability', 'toolchain', 'native_probe', 'cpu_graph',
            'clean_build', 'quality_gate', 'python_isolation', 'cuda_isolation',
            'redaction', 'cleanup')) {
        $gates[$name] = [ordered]@{ status = 'NOT_RUN'; detail = 'not reached' }
    }
    return $gates
}

function Set-P1AGate {
    param([Collections.IDictionary]$Gates, [string]$Name, [string]$Status, [string]$Detail)
    $Gates[$Name] = [ordered]@{ status = $Status; detail = $Detail }
}

function Invoke-P1ARecordedCommand {
    param(
        [Parameter(Mandatory)][hashtable]$Context,
        [Parameter(Mandatory)][string]$FilePath,
        [string[]]$ArgumentList = @(),
        [hashtable]$Environment = @{},
        [int]$TimeoutSeconds = 120,
        [AllowNull()][string]$RecordedStdout,
        [AllowNull()][string]$RecordedStderr,
        [switch]$RecordDigestOnly
    )
    $Context.CommandNumber = [int]$Context.CommandNumber + 1
    $id = 'C' + ([int]$Context.CommandNumber).ToString('00')
    $result = Invoke-P1AProcess -FilePath $FilePath -ArgumentList $ArgumentList `
        -WorkingDirectory $Context.RepositoryRoot -Environment $Environment `
        -TimeoutSeconds $TimeoutSeconds
    $stdoutValue = if ($PSBoundParameters.ContainsKey('RecordedStdout')) { $RecordedStdout }
        elseif ($RecordDigestOnly) {
            'raw stdout omitted; sha256=' + (Get-P1ATextSha256 -Text ([string]$result.stdout)) +
                '; utf8_bytes=' + $script:Utf8NoBom.GetByteCount([string]$result.stdout)
        }
        else { $result.stdout }
    $stderrValue = if ($PSBoundParameters.ContainsKey('RecordedStderr')) { $RecordedStderr }
        elseif ($RecordDigestOnly) {
            'raw stderr omitted; sha256=' + (Get-P1ATextSha256 -Text ([string]$result.stderr)) +
                '; utf8_bytes=' + $script:Utf8NoBom.GetByteCount([string]$result.stderr)
        }
        else { $result.stderr }
    $protectedOut = Protect-P1AText -Text ([string]$stdoutValue) `
        -RepositoryRoot $Context.RepositoryRoot -ExtraRoots $Context.RedactionRoots
    $protectedErr = Protect-P1AText -Text ([string]$stderrValue) `
        -RepositoryRoot $Context.RepositoryRoot -ExtraRoots $Context.RedactionRoots
    $stdoutPath = Join-Path $Context.CommandsRoot "$id.stdout.txt"
    $stderrPath = Join-Path $Context.CommandsRoot "$id.stderr.txt"
    Write-P1AUtf8LfFile -Path $stdoutPath -Text $protectedOut -CreateNew
    Write-P1AUtf8LfFile -Path $stderrPath -Text $protectedErr -CreateNew
    $displayArgs = @((Split-Path -Leaf $FilePath)) + @($ArgumentList | ForEach-Object {
        Protect-P1AText -Text ([string]$_) -RepositoryRoot $Context.RepositoryRoot `
            -ExtraRoots $Context.RedactionRoots
    })
    $environmentPolicyPattern = '^(?i:Path|PATHEXT|SystemRoot|WINDIR|ComSpec|TEMP|TMP|USERPROFILE|HOME|INCLUDE|LIB|LIBPATH|CL|_CL_|LINK|_LINK_|VS.*|VC.*|WindowsSDK.*|UCRT.*|UniversalCRTSdkDir|CARGO_.*|RUST.*|CUDA.*|CUDNN.*|NVCC.*|SCCACHE.*|NO_COLOR|CC_.*|AR_.*|P1A_CANARY_DIR)$'
    $environmentDeltaNames = @($Environment.Keys | ForEach-Object { [string]$_ } |
        Where-Object { $_ -match $environmentPolicyPattern } | Sort-Object -Unique)
    $effectiveEnvironment = Get-P1ACanonicalEnvironment
    foreach ($entry in $Environment.GetEnumerator()) {
        $name = [string]$entry.Key
        if ($null -eq $entry.Value) {
            [void]$effectiveEnvironment.Remove($name)
        }
        else {
            $effectiveEnvironment[$name] = [string]$entry.Value
        }
    }
    $effectiveBuildEnvironment = [ordered]@{}
    foreach ($name in @($effectiveEnvironment.Keys | Where-Object {
                [string]$_ -match $environmentPolicyPattern
            } | Sort-Object)) {
        $value = [string]$effectiveEnvironment[$name]
        $effectiveBuildEnvironment[[string]$name] = if ([string]$name -match '(?i:token|secret|password|credential|api[_-]?key)') {
            '<PRESENT>'
        }
        else {
            Protect-P1AText -Text $value -RepositoryRoot $Context.RepositoryRoot `
                -ExtraRoots $Context.RedactionRoots
        }
    }
    $record = [ordered]@{
        id = $id
        argv = $displayArgs
        cwd = '${REPO}'
        environment_delta_names = $environmentDeltaNames
        configuration_sha256 = Get-P1ATextSha256 -Text (([ordered]@{
                argv = $displayArgs
                cwd = '${REPO}'
                environment_delta_names = $environmentDeltaNames
                effective_build_environment = $effectiveBuildEnvironment
            } | ConvertTo-Json -Depth 8 -Compress))
        exit_code = [int]$result.exit_code
        duration_ms = [int64]$result.duration_ms
        status = if ([int]$result.exit_code -eq 0 -and -not [bool]$result.timed_out) { 'PASS' } else { 'FAIL' }
        stdout = [ordered]@{
            path = "commands/$id.stdout.txt"
            sha256 = Get-P1ASha256 -Path $stdoutPath
            bytes = [int64](Get-Item -LiteralPath $stdoutPath).Length
        }
        stderr = [ordered]@{
            path = "commands/$id.stderr.txt"
            sha256 = Get-P1ASha256 -Path $stderrPath
            bytes = [int64](Get-Item -LiteralPath $stderrPath).Length
        }
    }
    [void]$Context.Commands.Add($record)
    return [pscustomobject]@{ id = $id; result = $result; record = $record }
}

function Assert-P1ACommand {
    param($Command, [string]$Code, [string]$Message)
    if ($Command.record.status -ne 'PASS') {
        if ($Command.result.timed_out) { throw "[$Code] $Message (command timed out)" }
        throw "[$Code] $Message (exit $($Command.result.exit_code))"
    }
}

function Get-P1APathCommand {
    param([string]$Name, [hashtable]$Environment)
    $pathValue = if ($Environment.ContainsKey('Path')) { [string]$Environment['Path'] } else { [string]$env:Path }
    foreach ($directory in $pathValue.Split(';', [StringSplitOptions]::RemoveEmptyEntries)) {
        foreach ($candidateName in @($Name, "$Name.exe", "$Name.cmd", "$Name.bat")) {
            $candidate = Join-Path $directory $candidateName
            if (Test-Path -LiteralPath $candidate -PathType Leaf) { return [IO.Path]::GetFullPath($candidate) }
        }
    }
    return $null
}

function Get-P1AApplicationPath {
    param([Parameter(Mandatory)][string]$Name)

    $matches = @(Get-Command -Name $Name -CommandType Application -ErrorAction SilentlyContinue)
    if ($matches.Count -eq 0 -or [string]::IsNullOrWhiteSpace([string]$matches[0].Source)) {
        throw "[TOOL_NOT_FOUND] application was not resolved from PATH: $Name"
    }
    $path = [IO.Path]::GetFullPath([string]$matches[0].Source)
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "[TOOL_NOT_FOUND] resolved application is not a file: $Name"
    }
    return $path
}

function ConvertTo-P1ATokenizedToolPath {
    param([string]$Path, [hashtable]$Roots)
    $full = [IO.Path]::GetFullPath($Path)
    foreach ($name in @($Roots.Keys | Sort-Object)) {
        $root = [string]$Roots[$name]
        if (-not [string]::IsNullOrWhiteSpace($root) -and (Test-P1APathWithin -Path $full -Root $root)) {
            $relative = ConvertTo-P1ANormalizedPath -Path $full -BasePath $root
            if ($relative -eq '.') { return '${' + $name + '}' }
            return '${' + $name + '}/' + $relative
        }
    }
    throw "tool path is outside every approved normalization root: $(Split-Path -Leaf $full)"
}

function New-P1AToolDescriptor {
    param([string]$Name, [string]$Version, [string]$Path, [hashtable]$Roots)
    return [ordered]@{
        name = $Name
        version = if ([string]::IsNullOrWhiteSpace($Version)) { 'unknown' } else { $Version }
        path = ConvertTo-P1ATokenizedToolPath -Path $Path -Roots $Roots
        sha256 = Get-P1ASha256 -Path $Path
    }
}

function Get-P1AInputManifest {
    param([string]$RepositoryRoot, [string]$ScriptPath)
    $git = Get-P1AApplicationPath -Name 'git.exe'
    $listed = Invoke-P1AProcess -FilePath $git -ArgumentList @(
        'ls-files', '-z', '--cached', '--others', '--exclude-standard'
    ) -WorkingDirectory $RepositoryRoot -Environment @{} -TimeoutSeconds 120
    if ($listed.timed_out -or $listed.exit_code -ne 0) {
        throw 'git ls-files failed while building the P1A input manifest'
    }
    $paths = [Collections.Generic.List[string]]::new()
    foreach ($relative in @($listed.stdout -split [char]0 | Where-Object { $_ })) {
        $normalized = $relative.Replace('\', '/')
        if ($normalized.StartsWith('docs/receipts/P1A/', [StringComparison]::OrdinalIgnoreCase)) {
            continue
        }
        $path = [IO.Path]::GetFullPath((Join-Path $RepositoryRoot $relative))
        if (-not (Test-P1APathWithin -Path $path -Root $RepositoryRoot) -or
            -not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "input manifest path is invalid: $relative"
        }
        $paths.Add($path)
    }
    foreach ($required in @($ScriptPath, $MyInvocation.MyCommand.Module.Path)) {
        $full = [IO.Path]::GetFullPath($required)
        if (-not $paths.Contains($full)) {
            $paths.Add($full)
        }
    }
    $entries = @($paths | Sort-Object -Unique | ForEach-Object {
        [ordered]@{
            path = ConvertTo-P1ANormalizedPath -Path $_ -BasePath $RepositoryRoot
            sha256 = Get-P1ASha256 -Path $_
            bytes = [int64](Get-Item -LiteralPath $_).Length
        }
    })
    $text = $entries | ConvertTo-Json -Depth 8 -Compress
    return [pscustomobject]@{ entries = $entries; sha256 = Get-P1ATextSha256 -Text $text }
}

function Get-P1ADirectoryFingerprint {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) { return 'absent' }
    $lines = @(Get-ChildItem -LiteralPath $Path -File -Recurse | Sort-Object FullName | ForEach-Object {
        (ConvertTo-P1ANormalizedPath -Path $_.FullName -BasePath $Path) + '|' + $_.Length + '|' + $_.LastWriteTimeUtc.Ticks
    })
    return Get-P1ATextSha256 -Text ($lines -join $script:Lf)
}

function Get-P1ARepositoryFingerprint {
    param(
        [Parameter(Mandatory)][string]$RepositoryRoot,
        [Parameter(Mandatory)][string]$OutputRoot
    )

    $repository = [IO.Path]::GetFullPath($RepositoryRoot)
    $gitRoot = Join-Path $repository '.git'
    $targetRoot = Join-Path $repository 'target'
    $output = [IO.Path]::GetFullPath($OutputRoot)
    $values = [Collections.Generic.List[string]]::new()
    $pending = [Collections.Generic.Stack[string]]::new()
    $pending.Push($repository)
    while ($pending.Count -gt 0) {
        $directory = $pending.Pop()
        foreach ($item in Get-ChildItem -LiteralPath $directory -Force -ErrorAction Stop) {
            if ((Test-P1APathWithin -Path $item.FullName -Root $gitRoot) -or
                (Test-P1APathWithin -Path $item.FullName -Root $targetRoot) -or
                (Test-P1APathWithin -Path $item.FullName -Root $output)) {
                continue
            }
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "repository fingerprint encountered a reparse point: $($item.Name)"
            }
            if ($item.PSIsContainer) {
                $pending.Push($item.FullName)
            }
            else {
                $values.Add(
                    (ConvertTo-P1ANormalizedPath -Path $item.FullName -BasePath $repository) +
                        '|' + (Get-P1ASha256 -Path $item.FullName)
                )
            }
        }
    }
    $lines = @($values | Sort-Object)
    return Get-P1ATextSha256 -Text ($lines -join $script:Lf)
}

function Get-P1AEnvironmentFingerprint {
    $lines = @([Environment]::GetEnvironmentVariables().GetEnumerator() | Sort-Object Key | ForEach-Object {
        ([string]$_.Key).ToUpperInvariant() + '=' + [string]$_.Value
    })
    return Get-P1ATextSha256 -Text ($lines -join $script:Lf)
}

function Complete-P1AFailureRun {
    param(
        [Parameter(Mandatory)][string]$RunRoot,
        [Parameter(Mandatory)][string]$RunId,
        [Parameter(Mandatory)][DateTime]$Started,
        [Parameter(Mandatory)][Collections.IList]$Errors,
        [Parameter(Mandatory)][Collections.IList]$Commands,
        [Parameter(Mandatory)][Collections.IDictionary]$Gates,
        [Parameter(Mandatory)][Collections.IDictionary]$P0,
        [Parameter(Mandatory)][bool]$CleanupAttempted,
        [Parameter(Mandatory)][bool]$TemporaryRemoved
    )

    $artifactsRoot = Join-Path $RunRoot 'artifacts'
    [void][IO.Directory]::CreateDirectory($artifactsRoot)
    $sealPath = Join-Path $RunRoot 'SHA256SUMS'
    $sealExists = Test-Path -LiteralPath $sealPath -PathType Leaf
    if ($sealExists -and -not (Test-P1ASeal -RunRoot $RunRoot)) {
        throw 'existing failed-run seal is invalid'
    }
    $sourcePath = Join-Path $artifactsRoot 'source-identity.json'
    if (-not $sealExists -and -not (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
        Write-P1AJsonFile -Path $sourcePath -Value ([ordered]@{
                schema = 'python-slm-source-identity-v1'
                phase_id = 'P1A'
                mode = 'Cpu'
                status = 'INCOMPLETE'
            }) -CreateNew
    }
    $failureSummaryPath = Join-Path $artifactsRoot 'failure-summary.json'
    if (-not $sealExists -and -not (Test-Path -LiteralPath $failureSummaryPath -PathType Leaf)) {
        Write-P1AJsonFile -Path $failureSummaryPath -Value ([ordered]@{
                schema = 'python-slm-failure-summary-v1'
                phase_id = 'P1A'
                run_id = $RunId
                status = 'FAIL'
                errors = @($Errors)
            }) -CreateNew
    }
    if (-not (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
        throw 'sealed run is missing its source identity'
    }
    $evidencePath = Join-Path $RunRoot 'evidence.json'
    if ($sealExists -and -not (Test-Path -LiteralPath $evidencePath -PathType Leaf)) {
        throw 'sealed run is missing its terminal evidence'
    }
    if (-not (Test-Path -LiteralPath $evidencePath -PathType Leaf)) {
        $sealEntries = @(
            Get-ChildItem -LiteralPath $RunRoot -Recurse -File |
                Where-Object {
                    -not [string]::Equals(
                        $_.FullName,
                        $sealPath,
                        [StringComparison]::OrdinalIgnoreCase
                    )
                }
        ).Count + 1
        $sealReference = [ordered]@{
            path = 'SHA256SUMS'
            entries = $sealEntries
            coverage_rule = 'all_run_files_except_seal'
        }
        $finished = [DateTime]::UtcNow
        Write-P1AJsonFile -Path $evidencePath -Value ([ordered]@{
                schema = 'python-slm-phase-evidence-v1'
                phase_id = 'P1A'
                run_id = $RunId
                mode = 'Cpu'
                status = 'FAIL'
                started_at = $Started.ToString('o')
                finished_at = $finished.ToString('o')
                duration_ms = [int64]($finished - $Started).TotalMilliseconds
                invocation = [ordered]@{
                    argv = @(
                        'powershell', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
                        'scripts/verify-env.ps1', '-Mode', 'Cpu', '-OutputRoot',
                        'docs/receipts/P1A'
                    )
                    cwd = '${REPO}'
                }
                source_identity_sha256 = Get-P1ASha256 -Path $sourcePath
                p0_dependency = $P0
                environment = $null
                commands = @($Commands)
                gates = $Gates
                errors = @($Errors)
                cleanup = [ordered]@{
                    attempted = $CleanupAttempted
                    temporary_root_removed = $TemporaryRemoved
                }
                seal = $sealReference
            }) -CreateNew
    }
    if (-not $sealExists) {
        [void](New-P1ASeal -RunRoot $RunRoot)
    }
    if (-not (Test-P1ASeal -RunRoot $RunRoot)) {
        throw 'failed-run evidence does not match its seal'
    }
    return [pscustomobject][ordered]@{
        status = 'FAIL'
        phase_id = 'P1A'
        run_id = $RunId
        evidence_path = "docs/receipts/P1A/runs/$RunId/evidence.json"
        errors = @($Errors)
    }
}

function New-P1AModeNotImplementedRun {
    param(
        [Parameter(Mandatory)][string]$OutputRoot,
        [Parameter(Mandatory)][string]$RepositoryRoot
    )

    $started = [DateTime]::UtcNow
    [void][IO.Directory]::CreateDirectory((Join-Path $OutputRoot 'runs'))
    $runId = New-P1ARunId
    $runRoot = Join-Path (Join-Path $OutputRoot 'runs') $runId
    $artifactsRoot = Join-Path $runRoot 'artifacts'
    [void][IO.Directory]::CreateDirectory($artifactsRoot)
    $errorValue = [ordered]@{
        code = 'MODE_NOT_IMPLEMENTED'
        category = 5
        message = 'Cuda mode is reserved for Phase 1B and is not implemented.'
        remediation = 'Complete and approve P1A before implementing the P1B CUDA extension.'
    }
    $sourcePath = Join-Path $artifactsRoot 'source-identity.json'
    Write-P1AJsonFile -Path $sourcePath -Value ([ordered]@{
            schema = 'python-slm-source-identity-v1'
            phase_id = 'P1B'
            mode = 'Cuda'
            repository = '${REPO}'
            status = 'NOT_EVALUATED'
        }) -CreateNew
    Write-P1AJsonFile -Path (Join-Path $artifactsRoot 'failure-summary.json') -Value ([ordered]@{
            schema = 'python-slm-failure-summary-v1'
            phase_id = 'P1B'
            run_id = $runId
            status = 'FAIL'
            errors = @($errorValue)
        }) -CreateNew
    $sealReference = [ordered]@{
        path = 'SHA256SUMS'
        entries = @(Get-ChildItem -LiteralPath $runRoot -Recurse -File).Count + 1
        coverage_rule = 'all_run_files_except_seal'
    }
    $finished = [DateTime]::UtcNow
    $evidence = [ordered]@{
        schema = 'python-slm-phase-evidence-v1'
        phase_id = 'P1B'
        run_id = $runId
        mode = 'Cuda'
        status = 'FAIL'
        started_at = $started.ToString('o')
        finished_at = $finished.ToString('o')
        duration_ms = [int64]($finished - $started).TotalMilliseconds
        invocation = [ordered]@{
            argv = @(
                'powershell', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
                'scripts/verify-env.ps1', '-Mode', 'Cuda', '-OutputRoot', 'docs/receipts/P1B'
            )
            cwd = '${REPO}'
        }
        source_identity_sha256 = Get-P1ASha256 -Path $sourcePath
        p0_dependency = [ordered]@{
            status = 'FAIL'
            receipt_commit = '86fb1e4cc68efeb651e5362c4aca85c2827d8e4d'
            receipt_sha256 = 'f08c6a41658ff287e238d6a96c4f2c874975964202c3eeced2bc0bc21f308904'
            contract_sha256 = 'fc2c60b52fdd7c524e0da06bb03972a4d523c21ad5536cba536185435bd44ad4'
            decision_ledger_sha256 = '8349d8a3e06d96d6921889de5534715e7b2f7439caf7e06558a97652a8890c8d'
        }
        environment = $null
        commands = @()
        gates = New-P1AGates
        errors = @($errorValue)
        cleanup = [ordered]@{ attempted = $false; temporary_root_removed = $true }
        seal = $sealReference
    }
    $evidencePath = Join-Path $runRoot 'evidence.json'
    Write-P1AJsonFile -Path $evidencePath -Value $evidence -CreateNew
    [void](New-P1ASeal -RunRoot $runRoot)
    if (-not (Test-P1ASeal -RunRoot $runRoot)) {
        throw '[EVIDENCE_SEAL_FAILED] sealed Cuda failure run did not verify'
    }
    return [pscustomobject][ordered]@{
        status = 'FAIL'
        phase_id = 'P1B'
        run_id = $runId
        evidence_path = "docs/receipts/P1B/runs/$runId/evidence.json"
        errors = @($errorValue)
    }
}

function Invoke-P1AVerification {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Mode,
        [Parameter(Mandatory)][string]$OutputRoot,
        [string]$RepositoryRoot,
        [string]$ScriptPath
    )

    $started = [DateTime]::UtcNow
    if ([string]::IsNullOrWhiteSpace($ScriptPath)) {
        if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
            throw '[REPOSITORY_ROOT_REQUIRED] RepositoryRoot or ScriptPath is required'
        }
        $ScriptPath = Join-Path $RepositoryRoot 'scripts\verify-env.ps1'
    }
    $ScriptPath = [IO.Path]::GetFullPath($ScriptPath)
    if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
        $RepositoryRoot = [IO.Path]::GetFullPath((Join-Path (Split-Path -Parent $ScriptPath) '..'))
    }
    $repository = [IO.Path]::GetFullPath($RepositoryRoot).TrimEnd('\', '/')
    if ($Mode -cne 'Cpu' -and $Mode -cne 'Cuda') {
        Throw-P1AFailure -Code 'MODE_INVALID' -Category 2 `
            -Message "Mode must be Cpu or Cuda; got $Mode" `
            -Remediation 'Use -Mode Cpu for P1A or -Mode Cuda for the sealed P1B placeholder.'
    }
    $phaseId = if ($Mode -ceq 'Cuda') { 'P1B' } else { 'P1A' }
    $output = Resolve-P1AOutputRoot -RepositoryRoot $repository -OutputRoot $OutputRoot -PhaseId $phaseId
    if ($Mode -ceq 'Cuda') {
        return New-P1AModeNotImplementedRun -OutputRoot $output -RepositoryRoot $repository
    }
    [void][IO.Directory]::CreateDirectory((Join-Path $output 'runs'))
    $runId = New-P1ARunId
    $runRoot = Join-Path (Join-Path $output 'runs') $runId
    [void](New-Item -ItemType Directory -Path $runRoot)
    $commandsRoot = Join-Path $runRoot 'commands'
    $artifactsRoot = Join-Path $runRoot 'artifacts'
    [void](New-Item -ItemType Directory -Path $commandsRoot)
    [void](New-Item -ItemType Directory -Path $artifactsRoot)

    $commands = [Collections.ArrayList]::new()
    $errors = [Collections.ArrayList]::new()
    $gates = New-P1AGates
    $activeGate = 'p0'
    $status = 'FAIL'
    $environmentRef = $null
    $environmentManifest = $null
    $cargoLockStartHash = $null
    $cleanupAttempted = $false
    $temporaryRemoved = $true
    $temporaryRoot = $null
    $temporaryBase = $null
    $context = $null
    $p0 = [ordered]@{
        status = 'FAIL'
        receipt_commit = '86fb1e4cc68efeb651e5362c4aca85c2827d8e4d'
        receipt_sha256 = 'f08c6a41658ff287e238d6a96c4f2c874975964202c3eeced2bc0bc21f308904'
        contract_sha256 = 'fc2c60b52fdd7c524e0da06bb03972a4d523c21ad5536cba536185435bd44ad4'
        decision_ledger_sha256 = '8349d8a3e06d96d6921889de5534715e7b2f7439caf7e06558a97652a8890c8d'
    }
    try {
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or
        -not [Environment]::Is64BitProcess -or $env:PROCESSOR_ARCHITECTURE -cne 'AMD64') {
        throw '[UNSUPPORTED_HOST] P1A requires 64-bit Windows on AMD64'
    }
    if ($PSVersionTable.PSVersion.Major -ne 5 -or $PSVersionTable.PSVersion.Minor -ne 1) {
        throw '[UNSUPPORTED_POWERSHELL] P1A requires Windows PowerShell 5.1'
    }
    $temporaryBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\', '/')
    $temporaryRoot = Join-Path $temporaryBase ('python-slm-p1a-' + $runId)
    if (-not (Test-P1APathWithin -Path $temporaryRoot -Root $temporaryBase)) {
        throw '[UNSAFE_TEMP_PATH] generated temporary path escaped the system temporary root'
    }
    if (Test-P1APathWithin -Path $temporaryRoot -Root $repository) {
        throw '[UNSAFE_TEMP_PATH] P1A temporary root must be outside the repository'
    }
    if (Test-Path -LiteralPath $temporaryRoot) {
        throw '[TEMP_PATH_NOT_FRESH] generated temporary root already exists'
    }
    [void](New-Item -ItemType Directory -Path $temporaryRoot)
    $targetRoot = Join-Path $temporaryRoot 'cargo-target'
    $canaryRoot = Join-Path $temporaryRoot 'canaries'
    [void](New-Item -ItemType Directory -Path $canaryRoot)

    $context = @{
        RepositoryRoot = $repository
        RunRoot = $runRoot
        CommandsRoot = $commandsRoot
        Commands = $commands
        CommandNumber = 0
        RedactionRoots = @(
            $temporaryRoot, $output, $env:USERPROFILE,
            $(if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }),
            $(if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE '.rustup' }),
            $env:SystemRoot, $env:WINDIR, $env:TEMP, $env:TMP
        ) |
            Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    }
    $temporaryRemoved = $false
    $inputStart = Get-P1AInputManifest -RepositoryRoot $repository -ScriptPath $ScriptPath
    $repositoryTarget = Join-Path $repository 'target'
    $targetBefore = Get-P1ADirectoryFingerprint -Path $repositoryTarget
    $repositoryBefore = Get-P1ARepositoryFingerprint -RepositoryRoot $repository -OutputRoot $output
    $parentEnvironmentBefore = Get-P1AEnvironmentFingerprint
    $sourceIdentityPath = Join-Path $artifactsRoot 'source-identity.json'
    $schemaFiles = @(Get-ChildItem -LiteralPath (Join-Path $repository 'docs\schemas') -File |
        Sort-Object Name | ForEach-Object { $_.Name + '  ' + (Get-P1ASha256 $_.FullName) })
    $schemaBundleSha = Get-P1ATextSha256 -Text ($schemaFiles -join $script:Lf)
    $verifierFiles = @(
        $ScriptPath,
        $MyInvocation.MyCommand.Module.Path,
        (Join-Path $repository 'scripts\tests\verify-env.tests.ps1')
    )
    $verifierBundleSha = Get-P1ATextSha256 -Text ((@($verifierFiles | ForEach-Object {
                    (ConvertTo-P1ANormalizedPath -Path $_ -BasePath $repository) + '  ' +
                    (Get-P1ASha256 -Path $_)
                } | Sort-Object)) -join $script:Lf)

    try {
        $git = Get-P1AApplicationPath -Name 'git.exe'
        $headCommand = Invoke-P1ARecordedCommand -Context $context -FilePath $git `
            -ArgumentList @('rev-parse', 'HEAD')
        Assert-P1ACommand $headCommand 'GIT_HEAD_FAILED' 'could not resolve repository HEAD'
        $head = $headCommand.result.stdout.Trim()
        if ($head -notmatch '^[0-9a-f]{40}$') { throw '[GIT_HEAD_FAILED] repository HEAD was not a full Git object ID' }
        $statusCommand = Invoke-P1ARecordedCommand -Context $context -FilePath $git `
            -ArgumentList @('status', '--porcelain=v1', '--untracked-files=all', '--', '.', ':(exclude)docs/receipts/P1A') `
            -RecordDigestOnly
        Assert-P1ACommand $statusCommand 'GIT_STATUS_FAILED' 'could not inspect repository status'

        $baseline = 'b1ebb455cdae94bbb9fc54f246cdf2758eedf1d1'
        $sealed = @('docs/rebuild-contract.md', 'docs/receipts/P0/capture.ps1',
            'docs/receipts/P0/evidence.json', 'docs/receipts/P0/runs')
        $p0Diff = Invoke-P1ARecordedCommand -Context $context -FilePath $git `
            -ArgumentList (@('diff', '--exit-code', $baseline, '--') + $sealed) -RecordDigestOnly
        Assert-P1ACommand $p0Diff 'P0_SEALED_BYTES_CHANGED' 'sealed Phase 0 bytes differ from their baseline'
        $p0Status = Invoke-P1ARecordedCommand -Context $context -FilePath $git `
            -ArgumentList (@('status', '--porcelain=v1', '--untracked-files=all', '--') + $sealed) -RecordDigestOnly
        Assert-P1ACommand $p0Status 'P0_SEALED_PATH_DIRTY' 'could not inspect sealed Phase 0 paths'
        if (-not [string]::IsNullOrWhiteSpace($p0Status.result.stdout)) {
            throw '[P0_SEALED_PATH_DIRTY] sealed Phase 0 paths are dirty or contain additions'
        }
        $ancestor = Invoke-P1ARecordedCommand -Context $context -FilePath $git `
            -ArgumentList @('merge-base', '--is-ancestor', $p0.receipt_commit, 'HEAD')
        Assert-P1ACommand $ancestor 'P0_RECEIPT_NOT_ANCESTOR' 'signed P0 receipt commit is not an ancestor of HEAD'
        $receiptDiff = Invoke-P1ARecordedCommand -Context $context -FilePath $git `
            -ArgumentList @('diff', '--exit-code', $p0.receipt_commit, '--', 'docs/receipts/P0.md') -RecordDigestOnly
        Assert-P1ACommand $receiptDiff 'P0_RECEIPT_CHANGED' 'signed P0 receipt differs from its approval commit'
        $receiptStatus = Invoke-P1ARecordedCommand -Context $context -FilePath $git `
            -ArgumentList @('status', '--porcelain=v1', '--untracked-files=all', '--', 'docs/receipts/P0.md') -RecordDigestOnly
        Assert-P1ACommand $receiptStatus 'P0_RECEIPT_DIRTY' 'could not inspect signed P0 receipt status'
        if (-not [string]::IsNullOrWhiteSpace($receiptStatus.result.stdout)) { throw '[P0_RECEIPT_DIRTY] signed P0 receipt is dirty' }
        $receiptPath = Join-Path $repository 'docs\receipts\P0.md'
        $contractPath = Join-Path $repository 'docs\rebuild-contract.md'
        if ((Get-P1ASha256 $receiptPath) -cne $p0.receipt_sha256 -or
            (Get-P1ASha256 $contractPath) -cne $p0.contract_sha256) {
            throw '[P0_HASH_MISMATCH] signed P0 receipt or contract hash mismatch'
        }
        $receipt = Get-Content -Raw -LiteralPath $receiptPath
        if ([regex]::Matches($receipt, '(?m)^Status:[^\r\n]*$').Count -ne 1 -or
            $receipt -notmatch '(?m)^Status:[ \t]+\*\*PASS\*\*[ \t]*$') {
            throw '[P0_APPROVAL_INVALID] P0 receipt has no unique PASS status'
        }
        foreach ($summary in @('Technical approval', 'Data-governance approval')) {
            if ([regex]::Matches($receipt, '(?m)^' + [regex]::Escape($summary) + ':[^\r\n]*$').Count -ne 1 -or
                $receipt -notmatch ('(?m)^' + [regex]::Escape($summary) + ':[ \t]+\*\*APPROVED\*\*[ \t]*$')) {
                throw "[P0_APPROVAL_INVALID] missing or contradictory P0 summary: $summary"
            }
        }
        $p0Run = Join-Path $repository 'docs\receipts\P0\runs\20260811T074740Z-d5008e94'
        foreach ($line in Get-Content -LiteralPath (Join-Path $p0Run 'SHA256SUMS')) {
            $parts = $line -split '  ', 2
            if ($parts.Count -ne 2) { throw '[P0_SEAL_INVALID] malformed P0 seal line' }
            $sealedPath = [IO.Path]::GetFullPath((Join-Path $p0Run $parts[1]))
            if (-not (Test-P1APathWithin -Path $sealedPath -Root $p0Run) -or
                (Get-P1ASha256 $sealedPath) -cne $parts[0]) { throw '[P0_SEAL_INVALID] P0 run seal mismatch' }
        }
        $p0.status = 'PASS'
        Set-P1AGate $gates p0 PASS 'pinned P0 receipt, ancestry, sealed bytes, approvals, and run seal verified'
        $cargoLockStartHash = Get-P1ASha256 -Path (Join-Path $repository 'Cargo.lock')
        if ($cargoLockStartHash -cne 'c0a5e1afe02e252a22cca8bf07ab37fb2a25844210d2d5ce2e1e6082e497a28c') {
            throw '[CARGO_LOCK_IDENTITY_MISMATCH] Cargo.lock differs from the P0-pinned identity'
        }

        $activeGate = 'toolchain'
        $rustc = Get-P1AApplicationPath -Name 'rustc.exe'
        $cargo = Get-P1AApplicationPath -Name 'cargo.exe'
        $rustcVersionCommand = Invoke-P1ARecordedCommand -Context $context -FilePath $rustc -ArgumentList @('-Vv')
        Assert-P1ACommand $rustcVersionCommand 'RUSTC_FAILED' 'rustc -Vv failed'
        $rust = ConvertFrom-P1ARustcVersion -Text $rustcVersionCommand.result.stdout
        if ($rust.host -cne 'x86_64-pc-windows-msvc' -or
            -not (Test-P1AVersionAtLeast -Actual $rust.release -Minimum '1.96.0')) {
            throw '[RUST_TOOLCHAIN_MISMATCH] Rust 1.96 or newer with x86_64-pc-windows-msvc host is required'
        }
        $cargoVersionCommand = Invoke-P1ARecordedCommand -Context $context -FilePath $cargo -ArgumentList @('-Vv')
        Assert-P1ACommand $cargoVersionCommand 'CARGO_FAILED' 'cargo -Vv failed'
        $cargoMatch = [regex]::Match($cargoVersionCommand.result.stdout, '(?m)^release:\s*(?<v>\d+\.\d+\.\d+)')
        if (-not $cargoMatch.Success) { $cargoMatch = [regex]::Match($cargoVersionCommand.result.stdout, '^cargo\s+(?<v>\d+\.\d+\.\d+)') }
        if (-not $cargoMatch.Success) {
            throw '[CARGO_VERSION_INVALID] Cargo version output could not be parsed'
        }
        $targetLib = Invoke-P1ARecordedCommand -Context $context -FilePath $rustc `
            -ArgumentList @('--print', 'target-libdir', '--target', 'x86_64-pc-windows-msvc')
        Assert-P1ACommand $targetLib 'RUST_TARGET_MISSING' 'MSVC Rust target is not installed'

        $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
        if (-not (Test-Path -LiteralPath $vswhere -PathType Leaf)) { throw '[VSWHERE_NOT_FOUND] supported Visual Studio discovery tool was not found' }
        $vsQuery = @('-version', '[17.0,18.0)', '-products', '*', '-requires',
            'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-format', 'json', '-utf8')
        $vsCommand = Invoke-P1ARecordedCommand -Context $context -FilePath $vswhere `
            -ArgumentList $vsQuery -RecordedStdout 'VS2022 candidates discovered; raw installation paths intentionally omitted.'
        Assert-P1ACommand $vsCommand 'VSWHERE_FAILED' 'Visual Studio discovery failed'
        $instances = @($vsCommand.result.stdout | ConvertFrom-Json)
        $vs = Select-P1AVs2022Instance -Instances $instances
        $context.RedactionRoots = @($context.RedactionRoots) + @([string]$vs.installationPath)
        $vsDevCmd = Join-Path $vs.installationPath 'Common7\Tools\VsDevCmd.bat'
        if (-not (Test-Path -LiteralPath $vsDevCmd -PathType Leaf)) { throw '[VSDEVCMD_NOT_FOUND] selected VS 2022 instance has no VsDevCmd.bat' }
        $captureCmd = Join-Path $temporaryRoot 'capture-vs-env.cmd'
        Write-P1AUtf8LfFile -Path $captureCmd -Text @'
@echo off
setlocal DisableDelayedExpansion
call "%~1" -no_logo -arch=amd64 -host_arch=amd64 -startdir=none >nul
if errorlevel 1 exit /b %errorlevel%
set
'@
        $cmdExe = Join-Path $env:SystemRoot 'System32\cmd.exe'
        $vsEnvCommand = Invoke-P1ARecordedCommand -Context $context -FilePath $cmdExe `
            -ArgumentList @('/d', '/c', $captureCmd, $vsDevCmd) -RecordedStdout 'VS2022 x64 developer environment captured; raw environment intentionally omitted.'
        Assert-P1ACommand $vsEnvCommand 'VSDEVCMD_FAILED' 'VS 2022 x64 developer environment initialization failed'
        $childEnvironment = @{}
        $childEnvironmentSeen = [Collections.Generic.Dictionary[string,string]]::new(
            [StringComparer]::OrdinalIgnoreCase
        )
        foreach ($line in ($vsEnvCommand.result.stdout -split '\r?\n')) {
            $separator = $line.IndexOf('=')
            if ($separator -gt 0) {
                $name = $line.Substring(0, $separator)
                $value = $line.Substring($separator + 1)
                if ($childEnvironmentSeen.ContainsKey($name) -and
                    $childEnvironmentSeen[$name] -cne $value) {
                    throw "[CHILD_ENVIRONMENT_CONFLICT] VsDevCmd emitted conflicting case variants for $name"
                }
                $childEnvironmentSeen[$name] = $value
                $childEnvironment[$name] = $value
            }
        }
        if ($childEnvironment['VSCMD_ARG_TGT_ARCH'] -cne 'x64' -or
            $childEnvironment['VSCMD_ARG_HOST_ARCH'] -cne 'x64') {
            throw '[VS_ARCH_MISMATCH] VsDevCmd did not produce an x64 host/x64 target environment'
        }
        $toolPaths = @{}
        foreach ($toolName in @('cl', 'link', 'lib', 'dumpbin', 'rc', 'mt')) {
            $toolPaths[$toolName] = Get-P1APathCommand -Name $toolName -Environment $childEnvironment
            if ([string]::IsNullOrWhiteSpace($toolPaths[$toolName])) { throw "[MSVC_TOOL_NOT_FOUND] required tool was not resolved: $toolName" }
        }
        $sdkRoot = [string]$childEnvironment['WindowsSdkDir']
        $sdkVersion = ([string]$childEnvironment['WindowsSDKVersion']).TrimEnd('\', '/')
        $ucrtVersion = ([string]$childEnvironment['UCRTVersion']).TrimEnd('\', '/')
        if ([string]::IsNullOrWhiteSpace($sdkRoot) -or [string]::IsNullOrWhiteSpace($sdkVersion)) {
            throw '[WINDOWS_SDK_NOT_FOUND] Windows SDK identity was not populated by VsDevCmd'
        }
        foreach ($toolName in @('cl', 'link', 'lib', 'dumpbin')) {
            if (-not (Test-P1APathWithin -Path $toolPaths[$toolName] -Root ([string]$vs.installationPath))) {
                throw "[MSVC_TOOL_OUTSIDE_INSTANCE] $toolName was resolved outside the selected VS2022 instance"
            }
        }
        foreach ($toolName in @('rc', 'mt')) {
            if (-not (Test-P1APathWithin -Path $toolPaths[$toolName] -Root $sdkRoot)) {
                throw "[WINDOWS_SDK_TOOL_OUTSIDE_ROOT] $toolName was resolved outside the selected Windows SDK"
            }
        }
        $sdkValidatedPaths = @(
            (Join-Path $sdkRoot "Include\$sdkVersion\um\Windows.h")
            (Join-Path $sdkRoot "Include\$ucrtVersion\ucrt\stdlib.h")
            (Join-Path $sdkRoot "Lib\$sdkVersion\um\x64\kernel32.lib")
            (Join-Path $sdkRoot "Lib\$ucrtVersion\ucrt\x64\ucrt.lib")
            $toolPaths['rc']
            $toolPaths['mt']
        )
        foreach ($requiredSdkPath in $sdkValidatedPaths) {
            if (-not (Test-Path -LiteralPath $requiredSdkPath -PathType Leaf)) {
                throw "[WINDOWS_SDK_INCOMPLETE] required SDK file is missing: $(Split-Path -Leaf $requiredSdkPath)"
            }
        }
        $context.RedactionRoots += @([string]$vs.installationPath, $sdkRoot)

        # Cargo build scripts must not inherit unrelated parent credentials,
        # user compiler flags, or machine configuration. Start with explicit
        # removals for the complete parent environment, then add only the
        # qualified VS/SDK runtime surface and the few deterministic identities
        # Cargo and rustup need for offline operation.
        $developerEnvironment = $childEnvironment
        $isolatedEnvironment = @{}
        foreach ($name in (Get-P1ACanonicalEnvironment).Keys) {
            $isolatedEnvironment[[string]$name] = $null
        }
        $developerEnvironmentAllow = '^(?i:Path|PATHEXT|SystemRoot|WINDIR|ComSpec|PROCESSOR_ARCHITECTURE|NUMBER_OF_PROCESSORS|INCLUDE|LIB|LIBPATH|VSINSTALLDIR|VCINSTALLDIR|VCToolsInstallDir|VCToolsVersion|VisualStudioVersion|VSCMD_.*|WindowsSdkDir|WindowsSDKVersion|WindowsSdkBinPath|WindowsSdkVerBinPath|UCRTVersion|UniversalCRTSdkDir|ExtensionSdkDir|FrameworkDir.*|FrameworkVersion.*|NETFXSDKDir|DevEnvDir)$'
        foreach ($name in @($developerEnvironment.Keys | ForEach-Object { [string]$_ })) {
            if ($name -match $developerEnvironmentAllow) {
                $isolatedEnvironment[$name] = [string]$developerEnvironment[$name]
            }
        }
        $effectiveCargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else {
            Join-Path $env:USERPROFILE '.cargo'
        }
        $effectiveRustupHome = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else {
            Join-Path $env:USERPROFILE '.rustup'
        }
        $isolatedEnvironment['CARGO_HOME'] = $effectiveCargoHome
        $isolatedEnvironment['RUSTUP_HOME'] = $effectiveRustupHome
        if ($env:RUSTUP_TOOLCHAIN) {
            $isolatedEnvironment['RUSTUP_TOOLCHAIN'] = [string]$env:RUSTUP_TOOLCHAIN
        }
        $isolatedEnvironment['USERPROFILE'] = $temporaryRoot
        $isolatedEnvironment['HOME'] = $temporaryRoot
        $isolatedEnvironment['TEMP'] = $temporaryRoot
        $isolatedEnvironment['TMP'] = $temporaryRoot

        $allowedPathRoots = @(
            [string]$vs.installationPath,
            $sdkRoot,
            $env:SystemRoot,
            (Split-Path -Parent $rustc),
            (Split-Path -Parent $cargo),
            $effectiveCargoHome,
            $effectiveRustupHome
        ) | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) }
        $safePathParts = [Collections.Generic.List[string]]::new()
        foreach ($part in @(([string]$developerEnvironment['Path']).Split(';') | Where-Object {
                    -not [string]::IsNullOrWhiteSpace([string]$_)
                })) {
            try {
                $fullPart = [IO.Path]::GetFullPath($part).TrimEnd('\', '/')
                if (@($allowedPathRoots | Where-Object {
                            Test-P1APathWithin -Path $fullPart -Root ([string]$_)
                        }).Count -gt 0 -and -not $safePathParts.Contains($fullPart)) {
                    $safePathParts.Add($fullPart)
                }
            }
            catch {
                # Malformed or unresolved inherited PATH entries are omitted.
            }
        }
        foreach ($requiredDirectory in @(
                (Split-Path -Parent $rustc), (Split-Path -Parent $cargo),
                (Split-Path -Parent $toolPaths['cl']), (Split-Path -Parent $toolPaths['rc']),
                (Join-Path $env:SystemRoot 'System32')
            )) {
            $fullDirectory = [IO.Path]::GetFullPath($requiredDirectory).TrimEnd('\', '/')
            if (-not $safePathParts.Contains($fullDirectory)) {
                $safePathParts.Add($fullDirectory)
            }
        }
        $isolatedEnvironment['Path'] = $safePathParts -join ';'
        $childEnvironment = $isolatedEnvironment
        Set-P1AGate $gates toolchain PASS 'Rust/MSVC host, VS2022 x64 compiler/linker, and Windows SDK resolved'

        $activeGate = 'native_probe'
        $cargoConfigCandidates = [Collections.Generic.List[string]]::new()
        $searchRoot = $repository
        while (-not [string]::IsNullOrWhiteSpace($searchRoot)) {
            $cargoConfigCandidates.Add((Join-Path $searchRoot '.cargo\config'))
            $cargoConfigCandidates.Add((Join-Path $searchRoot '.cargo\config.toml'))
            $parent = Split-Path -Parent $searchRoot
            if ([string]::Equals($parent, $searchRoot, [StringComparison]::OrdinalIgnoreCase)) {
                break
            }
            $searchRoot = $parent
        }
        $cargoConfigCandidates.Add((Join-Path $effectiveCargoHome 'config'))
        $cargoConfigCandidates.Add((Join-Path $effectiveCargoHome 'config.toml'))
        $cargoConfigHits = @($cargoConfigCandidates | Sort-Object -Unique | Where-Object {
            Test-Path -LiteralPath $_ -PathType Leaf
        })
        if ($cargoConfigHits.Count -ne 0) {
            throw '[CARGO_CONFIG_PRESENT] build-affecting Cargo configuration is not permitted during P1A'
        }
        $markerRoot = Join-Path $temporaryRoot 'markers'
        [void](New-Item -ItemType Directory -Path $markerRoot)
        $canaries = @(
            'python', 'python3', 'pythonw', 'py', 'pyw', 'pip', 'pip3',
            'nvcc', 'ptxas', 'fatbinary', 'nvlink', 'nvidia-smi'
        )
        foreach ($canary in $canaries) {
            $marker = Join-Path $markerRoot ($canary + '.hit')
            $body = "@echo off`n> `"$marker`" echo invoked`nexit /b 97"
            Write-P1AUtf8LfFile -Path (Join-Path $canaryRoot ($canary + '.cmd')) -Text $body
        }
        $cleared = @(
            'CUDA_PATH', 'CUDA_HOME', 'CUDA_COMPUTE_CAP', 'CUDNN_PATH', 'CUDNN_HOME',
            'NVCC', 'RUST_LLM_ALLOW_CUDA_13', 'RUSTC', 'RUSTDOC', 'RUSTC_WRAPPER',
            'RUSTC_WORKSPACE_WRAPPER', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS',
            'CARGO_BUILD_TARGET', 'CARGO_TARGET_DIR', 'SCCACHE_DIR', 'SCCACHE_CACHE_SIZE',
            'CL', '_CL_', 'LINK', '_LINK_'
        )
        foreach ($name in @($childEnvironment.Keys | ForEach-Object { [string]$_ })) {
            if ($name -match '^(?i:CUDA|CUDNN|NVCC)') {
                $cleared += $name
            }
        }
        foreach ($name in $cleared) { $childEnvironment[$name] = $null }
        $pathParts = @(([string]$childEnvironment['Path']).Split(';') | Where-Object {
            -not [string]::IsNullOrWhiteSpace($_) -and
            $_ -notmatch '(?i)(NVIDIA GPU Computing Toolkit|[\\/]CUDA[\\/]|[\\/]cuDNN[\\/])'
        })
        $childEnvironment['Path'] = $canaryRoot + ';' + ($pathParts -join ';')
        foreach ($pathVariable in @('INCLUDE', 'LIB', 'LIBPATH')) {
            if ($childEnvironment.ContainsKey($pathVariable)) {
                $childEnvironment[$pathVariable] = (@(([string]$childEnvironment[$pathVariable]).Split(';') |
                    Where-Object {
                        -not [string]::IsNullOrWhiteSpace($_) -and
                        $_ -notmatch '(?i)(NVIDIA GPU Computing Toolkit|[\\/]CUDA[\\/]|[\\/]cuDNN[\\/])'
                    }) -join ';')
            }
        }
        $childEnvironment['CARGO_TARGET_DIR'] = $targetRoot
        $childEnvironment['CARGO_NET_OFFLINE'] = 'true'
        $childEnvironment['CARGO_INCREMENTAL'] = '0'
        $childEnvironment['CARGO_TERM_COLOR'] = 'never'
        $childEnvironment['CARGO_TERM_VERBOSE'] = 'true'
        $childEnvironment['NO_COLOR'] = '1'
        $childEnvironment['P1A_CANARY_DIR'] = $markerRoot
        $childEnvironment['CC_x86_64_pc_windows_msvc'] = $toolPaths['cl']
        $childEnvironment['AR_x86_64_pc_windows_msvc'] = $toolPaths['lib']
        $childEnvironment['CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER'] = $toolPaths['link']

        $cPath = Join-Path $temporaryRoot 'native_probe.c'
        $rustPath = Join-Path $temporaryRoot 'native_probe.rs'
        $objectPath = Join-Path $temporaryRoot 'native_probe.obj'
        $libraryPath = Join-Path $temporaryRoot 'native_probe.lib'
        $probeExe = Join-Path $temporaryRoot 'native_probe.exe'
        $cSource = @'
#include <windows.h>
int p1a_native_probe(void) {
    SYSTEM_INFO info;
    ZeroMemory(&info, sizeof(info));
    GetNativeSystemInfo(&info);
    if (sizeof(void*) != 8 || info.wProcessorArchitecture != PROCESSOR_ARCHITECTURE_AMD64) {
        return 0;
    }
    return 0x1A01;
}
'@
        $rustSource = @'
#[link(name = "native_probe", kind = "static")]
unsafe extern "C" {
    fn p1a_native_probe() -> i32;
}
fn main() {
    let value = unsafe { p1a_native_probe() };
    assert_eq!(value, 0x1A01);
    println!("{{\"native_probe\":{}}}", value);
}
'@
        Write-P1AUtf8LfFile -Path $cPath -Text $cSource
        Write-P1AUtf8LfFile -Path $rustPath -Text $rustSource
        $nativeIds = [Collections.Generic.List[string]]::new()
        $canarySourcePath = Join-Path $temporaryRoot 'tool_canary.c'
        $canaryExe = Join-Path $temporaryRoot 'tool_canary.exe'
        $canaryObject = Join-Path $temporaryRoot 'tool_canary.obj'
        $canarySource = @'
#include <windows.h>
int main(void) {
    char module[MAX_PATH];
    char directory[MAX_PATH];
    char marker[MAX_PATH];
    DWORD module_length = GetModuleFileNameA(NULL, module, MAX_PATH);
    DWORD directory_length = GetEnvironmentVariableA("P1A_CANARY_DIR", directory, MAX_PATH);
    DWORD base = 0;
    DWORD end = 0;
    DWORD i = 0;
    HANDLE file = INVALID_HANDLE_VALUE;
    if (module_length == 0 || module_length >= MAX_PATH ||
        directory_length == 0 || directory_length >= MAX_PATH) {
        return 97;
    }
    for (i = 0; i < module_length; ++i) {
        if (module[i] == '\\' || module[i] == '/') {
            base = i + 1;
        }
    }
    end = module_length;
    for (i = base; i < module_length; ++i) {
        if (module[i] == '.') {
            end = i;
            break;
        }
    }
    if (directory_length + 1 + (end - base) + 4 >= MAX_PATH) {
        return 97;
    }
    for (i = 0; i < directory_length; ++i) marker[i] = directory[i];
    marker[directory_length++] = '\\';
    for (i = base; i < end; ++i) marker[directory_length++] = module[i];
    marker[directory_length++] = '.';
    marker[directory_length++] = 'h';
    marker[directory_length++] = 'i';
    marker[directory_length++] = 't';
    marker[directory_length] = '\0';
    file = CreateFileA(marker, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    if (file != INVALID_HANDLE_VALUE) CloseHandle(file);
    return 97;
}
'@
        Write-P1AUtf8LfFile -Path $canarySourcePath -Text $canarySource
        $canaryCompile = Invoke-P1ARecordedCommand -Context $context -FilePath $toolPaths['cl'] `
            -ArgumentList @(
                '/nologo', '/TC', '/W4', '/WX', '/MD', $canarySourcePath,
                ('/Fo' + $canaryObject), ('/Fe' + $canaryExe), '/link', 'kernel32.lib'
            ) -Environment $childEnvironment
        $nativeIds.Add($canaryCompile.id)
        Assert-P1ACommand $canaryCompile 'CANARY_COMPILE_FAILED' 'tool canary compilation failed'
        foreach ($canary in $canaries) {
            [IO.File]::Copy($canaryExe, (Join-Path $canaryRoot ($canary + '.exe')), $false)
        }
        $clCommand = Invoke-P1ARecordedCommand -Context $context -FilePath $toolPaths['cl'] `
            -ArgumentList @('/nologo', '/TC', '/W4', '/WX', '/MD', '/Bv', '/c', $cPath, ('/Fo' + $objectPath)) -Environment $childEnvironment
        $nativeIds.Add($clCommand.id); Assert-P1ACommand $clCommand 'NATIVE_C_COMPILE_FAILED' 'native C probe compilation failed'
        $libCommand = Invoke-P1ARecordedCommand -Context $context -FilePath $toolPaths['lib'] `
            -ArgumentList @('/nologo', ('/OUT:' + $libraryPath), $objectPath) -Environment $childEnvironment
        $nativeIds.Add($libCommand.id); Assert-P1ACommand $libCommand 'NATIVE_ARCHIVE_FAILED' 'native probe archive creation failed'
        $rustProbe = Invoke-P1ARecordedCommand -Context $context -FilePath $rustc `
            -ArgumentList @('--edition=2024', '--target', 'x86_64-pc-windows-msvc', $rustPath,
                ('-Lnative=' + $temporaryRoot), '-lstatic=native_probe', '-ldylib=kernel32', '-o', $probeExe) -Environment $childEnvironment
        $nativeIds.Add($rustProbe.id); Assert-P1ACommand $rustProbe 'NATIVE_RUST_LINK_FAILED' 'Rust/native probe link failed'
        $probeRun = Invoke-P1ARecordedCommand -Context $context -FilePath $probeExe -ArgumentList @() -Environment $childEnvironment
        $nativeIds.Add($probeRun.id); Assert-P1ACommand $probeRun 'NATIVE_PROBE_RUN_FAILED' 'Rust/native probe execution failed'
        try { $nativeProbeResult = $probeRun.result.stdout | ConvertFrom-Json }
        catch { throw '[NATIVE_PROBE_OUTPUT_INVALID] native probe did not emit valid JSON' }
        if ([int]$nativeProbeResult.native_probe -ne 0x1A01) {
            throw '[NATIVE_PROBE_OUTPUT_INVALID] native probe returned the wrong fixed value'
        }
        $dumpCommand = Invoke-P1ARecordedCommand -Context $context -FilePath $toolPaths['dumpbin'] `
            -ArgumentList @('/DEPENDENTS', $probeExe) -Environment $childEnvironment
        $nativeIds.Add($dumpCommand.id)
        Assert-P1ACommand $dumpCommand 'PE_IMPORT_SCAN_FAILED' 'native probe PE import scan failed'
        $peImports = @([regex]::Matches($dumpCommand.result.stdout, '(?im)^\s*(?<dll>[A-Za-z0-9._-]+\.dll)\s*$') |
            ForEach-Object { $_.Groups['dll'].Value.ToLowerInvariant() } | Sort-Object -Unique)
        if ($peImports.Count -eq 0) { throw '[PE_IMPORT_SCAN_FAILED] no PE imports were parsed from native probe' }
        if (@($peImports | Where-Object { $_ -match '(cuda|cudnn|cublas|nvrtc|nvjitlink)' }).Count -ne 0) {
            throw '[CUDA_IMPORT_DETECTED] native probe imported a CUDA runtime library'
        }
        Set-P1AGate $gates native_probe PASS 'C, archive, Rust link/run, and PE import probe passed'

        $activeGate = 'cpu_graph'
        $metadataCommand = Invoke-P1ARecordedCommand -Context $context -FilePath $cargo `
            -ArgumentList @(
                'metadata', '--locked', '--offline', '--format-version', '1',
                '--no-default-features', '--features', 'cpu-reference',
                '--filter-platform', 'x86_64-pc-windows-msvc'
            ) -Environment $childEnvironment -TimeoutSeconds 600 -RecordDigestOnly
        Assert-P1ACommand $metadataCommand 'CARGO_METADATA_FAILED' 'locked offline CPU metadata resolution failed'
        $metadata = $metadataCommand.result.stdout | ConvertFrom-Json
        $packageById = @{}
        foreach ($package in $metadata.packages) { $packageById[[string]$package.id] = [string]$package.name }
        $forbiddenPattern = '(?i)(cuda|cudnn|cublas|curand|cusparse|nvrtc|nvjitlink|cudarc|cubecl-cuda)'
        $metadataCudaNamedNodes = [Collections.Generic.List[string]]::new()
        foreach ($node in @($metadata.resolve.nodes)) {
            $packageName = [string]$packageById[[string]$node.id]
            if ($packageName -match $forbiddenPattern) {
                $metadataCudaNamedNodes.Add("package:$packageName")
            }
            foreach ($feature in @($node.features)) {
                if ([string]$feature -match $forbiddenPattern) {
                    $metadataCudaNamedNodes.Add("feature:$packageName/$feature")
                }
            }
        }
        $treeCommand = Invoke-P1ARecordedCommand -Context $context -FilePath $cargo `
            -ArgumentList @(
                'tree', '--locked', '--offline', '--no-default-features',
                '--features', 'cpu-reference', '--target',
                'x86_64-pc-windows-msvc', '-e', 'features'
            ) -Environment $childEnvironment -TimeoutSeconds 600 -RecordDigestOnly
        Assert-P1ACommand $treeCommand 'CARGO_TREE_FAILED' 'locked offline CPU feature tree failed'
        $forbiddenFeatures = @(
            Get-P1AForbiddenCargoTreeTokens -Text ([string]$treeCommand.result.stdout)
        )
        $metadataCudaNamedNodes = @($metadataCudaNamedNodes | Sort-Object -Unique)
        $cargoGraphStatus = if ($forbiddenFeatures.Count -eq 0) { 'PASS' } else { 'FAIL' }
        Write-P1AJsonFile -Path (Join-Path $artifactsRoot 'cargo-graph.json') -Value ([ordered]@{
                schema = 'python-slm-cargo-graph-v1'
                target = 'x86_64-pc-windows-msvc'
                feature = 'cpu-reference'
                metadata_command_id = $metadataCommand.id
                tree_command_id = $treeCommand.id
                resolved_package_count = @($metadata.resolve.nodes).Count
                metadata_cuda_named_nodes = @($metadataCudaNamedNodes)
                forbidden_matches = @($forbiddenFeatures)
                status = $cargoGraphStatus
            }) -CreateNew
        if ($forbiddenFeatures.Count -ne 0) {
            throw '[CPU_GRAPH_CONTAINS_CUDA] activated CPU dependency graph contains CUDA packages or features'
        }
        Set-P1AGate $gates cpu_graph PASS 'locked activated CPU feature tree contains no forbidden CUDA packages or features; metadata-only names are diagnostic'

        $activeGate = 'clean_build'
        if (Test-Path -LiteralPath $targetRoot) { throw '[TARGET_NOT_CLEAN] fresh Cargo target existed before the build' }
        $cleanBuild = Invoke-P1ARecordedCommand -Context $context -FilePath $cargo `
            -ArgumentList @(
                'test', '--locked', '--offline', '--no-default-features',
                '--features', 'cpu-reference', '--target', 'x86_64-pc-windows-msvc',
                '--no-run', '--message-format=json-render-diagnostics'
            ) `
            -Environment $childEnvironment -TimeoutSeconds 1800 -RecordDigestOnly
        Assert-P1ACommand $cleanBuild 'CLEAN_BUILD_FAILED' 'clean-target locked CPU compile failed'
        $cargoMessageCount = 0
        $cargoExecutables = [Collections.Generic.List[string]]::new()
        $cargoLinkDirectives = [Collections.Generic.List[string]]::new()
        foreach ($line in @($cleanBuild.result.stdout -split '\r?\n' | Where-Object {
                    -not [string]::IsNullOrWhiteSpace($_)
                })) {
            try { $cargoMessage = $line | ConvertFrom-Json }
            catch { throw '[CARGO_MESSAGE_INVALID] clean build emitted non-JSON stdout' }
            $cargoMessageCount++
            if ($cargoMessage.PSObject.Properties['executable'] -and
                -not [string]::IsNullOrWhiteSpace([string]$cargoMessage.executable)) {
                $executable = [IO.Path]::GetFullPath([string]$cargoMessage.executable)
                if (-not (Test-P1APathWithin -Path $executable -Root $targetRoot)) {
                    throw '[CARGO_ARTIFACT_OUTSIDE_TARGET] Cargo reported an executable outside the fresh target'
                }
                $cargoExecutables.Add($executable)
            }
            foreach ($field in @('linked_libs', 'linked_paths')) {
                if ($cargoMessage.PSObject.Properties[$field]) {
                    foreach ($value in @($cargoMessage.$field)) {
                        $cargoLinkDirectives.Add([string]$value)
                    }
                }
            }
        }
        if ($cargoMessageCount -eq 0) {
            throw '[CARGO_MESSAGE_INVALID] clean build emitted no Cargo JSON messages'
        }
        $forbiddenLinkDirectives = @($cargoLinkDirectives | Where-Object {
            $_ -match $forbiddenPattern
        } | Sort-Object -Unique)
        if ($forbiddenLinkDirectives.Count -ne 0) {
            throw '[CUDA_LINK_DIRECTIVE_DETECTED] clean CPU build emitted a CUDA link directive'
        }
        Set-P1AGate $gates clean_build PASS 'locked CPU test graph compiled from an initially absent target directory'

        $activeGate = 'quality_gate'
        $cargoAuditText = [Collections.Generic.List[string]]::new()
        $cargoAuditText.Add([string]$cleanBuild.result.stdout)
        $cargoAuditText.Add([string]$cleanBuild.result.stderr)
        $qualityIds = [Collections.Generic.List[string]]::new()
        foreach ($spec in @(
                @{ Args = @('fmt', '--all', '--', '--check'); Timeout = 600 },
                @{ Args = @('clippy', '--locked', '--all-targets', '--features', 'cpu-reference', '--', '-D', 'warnings'); Timeout = 1800 },
                @{ Args = @('test', '--locked', '--features', 'cpu-reference'); Timeout = 1800 }
            )) {
            $quality = Invoke-P1ARecordedCommand -Context $context -FilePath $cargo `
                -ArgumentList $spec.Args -Environment $childEnvironment -TimeoutSeconds $spec.Timeout `
                -RecordDigestOnly
            $qualityIds.Add($quality.id)
            $cargoAuditText.Add([string]$quality.result.stdout)
            $cargoAuditText.Add([string]$quality.result.stderr)
            Assert-P1ACommand $quality 'CPU_QUALITY_GATE_FAILED' ('CPU quality command failed: cargo ' + ($spec.Args -join ' '))
        }
        $qualityCommandIds = @($cleanBuild.id) + @($qualityIds)
        $verboseCommandLines = @(($cargoAuditText -join $script:Lf) -split '\r?\n' | Where-Object {
                $_ -match '^\s*Running\s+`'
            })
        $forbiddenProcessPattern = '(?i)(?:^|[\\/"''\s])(python(?:3(?:\.\d+)*)?|pythonw|py|pyw|pip3?|nvcc|ptxas|fatbinary|nvlink|nvidia-smi)(?:\.exe)?(?:["''\s]|$)'
        $forbiddenVerboseCommands = @($verboseCommandLines | Where-Object {
                $_ -match $forbiddenProcessPattern
            })
        if ($forbiddenVerboseCommands.Count -ne 0) {
            throw '[FORBIDDEN_PROCESS_IN_VERBOSE_BUILD] verbose Cargo output reported a Python or CUDA process'
        }

        $peAuditIds = [Collections.Generic.List[string]]::new()
        $allPeImports = [Collections.Generic.List[string]]::new()
        $forbiddenImports = [Collections.Generic.List[string]]::new()
        $peFiles = @(Get-ChildItem -LiteralPath $targetRoot -File -Recurse | Where-Object {
            $_.Extension -in @('.exe', '.dll')
        } | Sort-Object FullName)
        if ($peFiles.Count -eq 0) {
            throw '[PE_IMPORT_SCAN_FAILED] clean Cargo target produced no PE files to audit'
        }
        foreach ($peFile in $peFiles) {
            $peDump = Invoke-P1ARecordedCommand -Context $context -FilePath $toolPaths['dumpbin'] `
                -ArgumentList @('/DEPENDENTS', $peFile.FullName) -Environment $childEnvironment
            $peAuditIds.Add($peDump.id)
            Assert-P1ACommand $peDump 'PE_IMPORT_SCAN_FAILED' 'Cargo-produced PE import scan failed'
            foreach ($import in @([regex]::Matches(
                        $peDump.result.stdout,
                        '(?im)^\s*(?<dll>[A-Za-z0-9._-]+\.dll)\s*$'
                    ) | ForEach-Object { $_.Groups['dll'].Value.ToLowerInvariant() })) {
                $allPeImports.Add($import)
                if ($import -match '^(nvcuda|cudart|nvrtc|nvjitlink|cublas|cudnn|cusparse|curand).*\.dll$') {
                    $forbiddenImports.Add($import)
                }
            }
        }
        $allPeImports = @($allPeImports | Sort-Object -Unique)
        $forbiddenImports = @($forbiddenImports | Sort-Object -Unique)
        if ($forbiddenImports.Count -ne 0) {
            throw '[CUDA_IMPORT_DETECTED] Cargo-produced PE imports NVIDIA driver, runtime, or math libraries'
        }
        Set-P1AGate $gates quality_gate PASS 'clean compile, rustfmt, clippy, CPU tests, and Cargo PE import audit passed'

        $canaryHits = @(Get-ChildItem -LiteralPath $markerRoot -File -ErrorAction SilentlyContinue | ForEach-Object { $_.BaseName })
        if (@($canaryHits | Where-Object { $_ -in @('python', 'python3', 'pythonw', 'py', 'pyw', 'pip', 'pip3') }).Count -ne 0) { throw '[PYTHON_INVOKED] qualification invoked Python' }
        if (@($canaryHits | Where-Object { $_ -in @('nvcc', 'ptxas', 'fatbinary', 'nvlink', 'nvidia-smi') }).Count -ne 0) { throw '[CUDA_TOOL_INVOKED] CPU qualification invoked a CUDA tool' }
        Set-P1AGate $gates python_isolation PASS 'Python command canaries were not invoked'
        Set-P1AGate $gates cuda_isolation PASS 'CUDA command canaries, features, and imports were absent'

        $forbiddenArtifacts = @(Get-ChildItem -LiteralPath $targetRoot -File -Recurse | Where-Object {
            $_.Extension -match '^(?i:\.cu|\.ptx|\.cubin|\.fatbin)$' -or
            $_.Name -match '(?i)(cuda|cudnn|cublas|curand|cusparse|nvrtc|nvjitlink)'
        } | ForEach-Object { $_.Name } | Sort-Object -Unique)
        if ($forbiddenArtifacts.Count -ne 0) { throw '[CUDA_ARTIFACT_DETECTED] clean CPU target contains CUDA-named artifacts' }

        Write-P1AJsonFile -Path (Join-Path $artifactsRoot 'native-probe.json') -Value ([ordered]@{
                schema = 'python-slm-native-probe-v1'
                status = 'PASS'
                c_source_sha256 = Get-P1ATextSha256 $cSource
                rust_source_sha256 = Get-P1ATextSha256 $rustSource
                canary_source_sha256 = Get-P1ATextSha256 $canarySource
                command_ids = @($nativeIds)
                native_probe_value = [int]$nativeProbeResult.native_probe
                imports = @($peImports)
            }) -CreateNew
        Write-P1AJsonFile -Path (Join-Path $artifactsRoot 'cpu-isolation.json') -Value ([ordered]@{
                schema = 'python-slm-cpu-isolation-v1'
                status = 'PASS'
                canaries = @($canaries | Sort-Object -Unique)
                canary_hits = @($canaryHits)
                forbidden_feature_matches = @($forbiddenFeatures)
                forbidden_artifacts = @($forbiddenArtifacts)
                forbidden_imports = @($forbiddenImports)
                verbose_audit_mode = 'opportunistic_cargo_running_records'
                verbose_command_records_audited = $verboseCommandLines.Count
                pe_files_scanned = $peFiles.Count
                pe_imports = @($allPeImports)
                pe_audit_command_ids = @($peAuditIds)
            }) -CreateNew

        $inputEnd = Get-P1AInputManifest -RepositoryRoot $repository -ScriptPath $ScriptPath
        if ($inputEnd.sha256 -cne $inputStart.sha256) { throw '[INPUT_CHANGED_DURING_RUN] verifier/build inputs changed during qualification' }
        if ((Get-P1ASha256 -Path (Join-Path $repository 'Cargo.lock')) -cne $cargoLockStartHash) {
            throw '[CARGO_LOCK_CHANGED] Cargo.lock changed during qualification'
        }
        Set-P1AGate $gates input_stability PASS 'verifier, Cargo, source, native, and schema inputs remained stable'

        $sourceIdentity = [ordered]@{
            schema = 'python-slm-source-identity-v1'
            head = $head
            dirty = -not [string]::IsNullOrWhiteSpace($statusCommand.result.stdout)
            input_manifest_sha256 = $inputStart.sha256
            cargo_lock_sha256 = $cargoLockStartHash
            verifier_bundle_sha256 = $verifierBundleSha
            files = $inputStart.entries
        }
        Write-P1AJsonFile -Path $sourceIdentityPath -Value $sourceIdentity -CreateNew
        $sourceIdentitySha = Get-P1ASha256 -Path $sourceIdentityPath
        $toolRoots = @{
            REPO = $repository
            OUTPUT_ROOT = $output
            TEMP = $temporaryRoot
            USERPROFILE = $env:USERPROFILE
            VS_INSTALL = [string]$vs.installationPath
            VC_TOOLS = [string]$childEnvironment['VCToolsInstallDir']
            WINDOWS_KITS = $sdkRoot
            WINDOWS = $env:SystemRoot
            CARGO_HOME = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
            RUSTUP_HOME = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE '.rustup' }
        }
        $vsTools = @('cl', 'link', 'lib', 'dumpbin') | ForEach-Object {
            $version = [Diagnostics.FileVersionInfo]::GetVersionInfo($toolPaths[$_]).FileVersion
            New-P1AToolDescriptor -Name $_ -Version $version -Path $toolPaths[$_] -Roots $toolRoots
        }
        $sdkTools = @('rc', 'mt') | ForEach-Object {
            $version = [Diagnostics.FileVersionInfo]::GetVersionInfo($toolPaths[$_]).FileVersion
            New-P1AToolDescriptor -Name $_ -Version $version -Path $toolPaths[$_] -Roots $toolRoots
        }
        $status = 'PASS'
    }
    catch {
        $message = Protect-P1AText -Text $_.Exception.Message -RepositoryRoot $repository `
            -ExtraRoots $context.RedactionRoots
        $code = if ($message -match '^\[(?<code>[A-Z0-9_]+)\]\s*(?<body>.*)$') {
            $message = $Matches['body']; $Matches['code']
        }
        else { 'UNEXPECTED_VERIFIER_FAILURE' }
        $category = if ($_.Exception.Data.Contains('P1ACategory')) {
            [int]$_.Exception.Data['P1ACategory']
        }
        else {
            Get-P1AErrorCategory -Code $code
        }
        Add-P1AError -Errors $errors -Code $code -Message $message `
            -Remediation 'Correct the reported prerequisite or command failure, then run the exact P1A command again.' `
            -Category $category
        Set-P1AGate $gates $activeGate FAIL $message
        $status = 'FAIL'
    }
    finally {
        $cleanupAttempted = $true
        try {
            if (Test-Path -LiteralPath $temporaryRoot) {
                $resolvedTemp = (Resolve-Path -LiteralPath $temporaryRoot).Path
                if (-not (Test-P1APathWithin -Path $resolvedTemp -Root $temporaryBase) -or
                    (Split-Path -Leaf $resolvedTemp) -cnotmatch '^python-slm-p1a-[0-9]{8}T[0-9]{9}Z-[0-9a-f]{24}$') {
                    throw "unsafe temporary cleanup target: $resolvedTemp"
                }
                Remove-Item -LiteralPath $resolvedTemp -Recurse -Force
            }
            $temporaryRemoved = -not (Test-Path -LiteralPath $temporaryRoot)
        }
        catch {
            $temporaryRemoved = $false
            Add-P1AError -Errors $errors -Code 'TEMP_CLEANUP_FAILED' `
                -Message 'qualification temporary root could not be safely removed' `
                -Remediation 'Inspect the recorded temporary root ownership and remove only that exact directory.' `
                -Category 5
            $status = 'FAIL'
        }
    }

    $inputFinal = Get-P1AInputManifest -RepositoryRoot $repository -ScriptPath $ScriptPath
    $inputsUnchanged = $inputFinal.sha256 -ceq $inputStart.sha256
    $targetUnchanged = (Get-P1ADirectoryFingerprint -Path $repositoryTarget) -ceq $targetBefore
    $repositoryUnchanged = (Get-P1ARepositoryFingerprint -RepositoryRoot $repository -OutputRoot $output) -ceq $repositoryBefore
    $parentEnvironmentUnchanged = (Get-P1AEnvironmentFingerprint) -ceq $parentEnvironmentBefore
    if (-not $inputsUnchanged -or -not $targetUnchanged -or -not $repositoryUnchanged -or
        -not $parentEnvironmentUnchanged -or -not $temporaryRemoved) {
        $status = 'FAIL'
        Set-P1AGate $gates cleanup FAIL 'cleanup or state-preservation invariant failed'
        if (-not $inputsUnchanged) { Add-P1AError $errors 'INPUT_CHANGED_DURING_RUN' 'qualification inputs changed' 'Restore or review the concurrent input change before retrying.' 3 }
        if (-not $targetUnchanged) { Add-P1AError $errors 'REPOSITORY_TARGET_CHANGED' 'repository target directory changed' 'Use only the isolated CARGO_TARGET_DIR and retry.' 3 }
        if (-not $repositoryUnchanged) { Add-P1AError $errors 'REPOSITORY_MUTATED' 'a repository file outside the P1A output changed' 'Restore the unexpected repository mutation before retrying.' 3 }
        if (-not $parentEnvironmentUnchanged) { Add-P1AError $errors 'PARENT_ENVIRONMENT_CHANGED' 'parent environment changed' 'Run in a clean shell and avoid process-scope environment mutation.' 3 }
    }
    else {
        Set-P1AGate $gates cleanup PASS 'temporary root removed; repository files, target, inputs, and parent environment unchanged'
    }

    if ($status -eq 'PASS') {
        $environmentPath = Join-Path $artifactsRoot 'environment.json'
        $environmentManifest = [ordered]@{
            schema = 'python-slm-environment-manifest-v1'
            phase_id = 'P1A'
            run_id = $runId
            mode = 'Cpu'
            status = 'PASS'
            p0 = [ordered]@{
                receipt_commit = $p0.receipt_commit; receipt_sha256 = $p0.receipt_sha256
                contract_sha256 = $p0.contract_sha256; decision_ledger_sha256 = $p0.decision_ledger_sha256
            }
            source = [ordered]@{
                head = $head; dirty = $sourceIdentity.dirty; input_manifest_sha256 = $inputStart.sha256
                cargo_lock_sha256 = $cargoLockStartHash
                verifier_sha256 = $verifierBundleSha; schema_bundle_sha256 = $schemaBundleSha
            }
            host = [ordered]@{
                os_version = [Environment]::OSVersion.VersionString; os_architecture = 'AMD64'
                process_architecture = 'AMD64'; powershell_edition = [string]$PSVersionTable.PSEdition
                powershell_version = $PSVersionTable.PSVersion.ToString()
            }
            rust = [ordered]@{
                rustc_version = $rust.release; commit_hash = $rust.commit_hash; commit_date = $rust.commit_date
                host = $rust.host; llvm_version = $rust.llvm_version
                rustc = New-P1AToolDescriptor rustc $rust.release $rustc $toolRoots
                cargo_version = $cargoMatch.Groups['v'].Value
                cargo = New-P1AToolDescriptor cargo $cargoMatch.Groups['v'].Value $cargo $toolRoots
                rustup = $null
            }
            visual_studio = [ordered]@{
                discovery_method = 'vswhere'; query = $vsQuery; instance_id = [string]$vs.instanceId
                product_id = [string]$vs.productId
                product_version = if ($vs.catalog.productDisplayVersion) { [string]$vs.catalog.productDisplayVersion } else { [string]$vs.installationVersion }
                installation_version = [string]$vs.installationVersion
                complete = $true; launchable = $true; reboot_required = $false
                developer_environment_arguments = @('-arch=amd64', '-host_arch=amd64', '-startdir=none'); tools = @($vsTools)
            }
            windows_sdk = [ordered]@{
                version = $sdkVersion; ucrt_version = $ucrtVersion; root = '${WINDOWS_KITS}'
                validated_paths = @($sdkValidatedPaths | ForEach-Object { ConvertTo-P1ATokenizedToolPath $_ $toolRoots })
                tools = @($sdkTools)
            }
            isolation = [ordered]@{
                target_absent_before = $true; target_outside_repository = $true; offline = $true
                incremental_disabled = $true; wrappers_disabled = $true
                cargo_config_absent = $cargoConfigHits.Count -eq 0
                cleared_environment_names = @($cleared | Sort-Object -Unique); canaries = @($canaries | Sort-Object -Unique)
                canary_hits = @(); forbidden_feature_matches = @(); forbidden_artifacts = @()
                forbidden_imports = @(); python_invoked = $false; cuda_invoked = $false
            }
            native_probe = [ordered]@{
                c_source_sha256 = Get-P1ATextSha256 $cSource
                rust_source_sha256 = Get-P1ATextSha256 $rustSource
                command_ids = @($nativeIds); result = 'PASS'; pe_imports = @($peImports)
            }
            quality_gate = [ordered]@{ command_ids = @($qualityCommandIds); result = 'PASS' }
            cleanup = [ordered]@{
                temporary_root_removed = $true; repository_target_unchanged = $true
                parent_environment_unchanged = $true; inputs_unchanged = $true
            }
        }
        Write-P1AJsonFile -Path $environmentPath -Value $environmentManifest -CreateNew
        $environmentRef = [ordered]@{
            path = 'artifacts/environment.json'; sha256 = Get-P1ASha256 $environmentPath
            bytes = [int64](Get-Item $environmentPath).Length
        }
    }
    else {
        if (-not (Test-Path -LiteralPath $sourceIdentityPath)) {
            Write-P1AJsonFile -Path $sourceIdentityPath -Value ([ordered]@{
                input_manifest_sha256 = $inputStart.sha256; files = $inputStart.entries
            }) -CreateNew
        }
        $sourceIdentitySha = Get-P1ASha256 $sourceIdentityPath
        Write-P1AJsonFile -Path (Join-Path $artifactsRoot 'failure-summary.json') `
            -Value ([ordered]@{ status = 'FAIL'; errors = @($errors) }) -CreateNew
    }

    $leaks = [Collections.Generic.List[string]]::new()
    $sensitiveValues = @(
        $repository, $temporaryRoot, $env:USERPROFILE, $env:USERNAME,
        $env:COMPUTERNAME
    ) + @($context.RedactionRoots)
    foreach ($file in Get-ChildItem -LiteralPath $runRoot -File -Recurse) {
        $text = [IO.File]::ReadAllText($file.FullName, $script:Utf8NoBom)
        $relativeFile = ConvertTo-P1ANormalizedPath $file.FullName $runRoot
        foreach ($value in @($sensitiveValues | Where-Object {
                    -not [string]::IsNullOrWhiteSpace([string]$_)
                } | Sort-Object -Unique)) {
            if ($text -match [regex]::Escape([string]$value)) {
                $leaks.Add($relativeFile)
            }
        }
        if ($text -match '(?i)(?:^|[\s"''])[A-Z]:[\\/]' -or
            $text -match '(?i)(?:^|[\s"''])\\\\[^\\\s]+' -or
            $text -match '(?i)(?:authorization|api[_-]?key|access[_-]?token|password)\s*[:=]\s*[^\s,;}]+') {
            $leaks.Add($relativeFile)
        }
    }
    if ($leaks.Count -gt 0) {
        $status = 'FAIL'
        Set-P1AGate $gates redaction FAIL 'machine-specific absolute paths remained in run artifacts'
        Add-P1AError $errors 'REDACTION_FAILED' 'machine-specific absolute paths remained in run artifacts' 'Fix tokenization/redaction before retrying.' 3
        foreach ($relativeLeak in @($leaks | Sort-Object -Unique)) {
            $leakedPath = [IO.Path]::GetFullPath((Join-Path $runRoot $relativeLeak.Replace('/', '\')))
            if (-not (Test-P1APathWithin -Path $leakedPath -Root $runRoot) -or
                -not (Test-Path -LiteralPath $leakedPath -PathType Leaf)) {
                throw '[REDACTION_FAILED] unsafe leaked-artifact path'
            }
            $replacement = if ([IO.Path]::GetExtension($leakedPath) -ieq '.json') {
                '{"schema":"python-slm-redacted-artifact-v1","status":"REDACTED"}'
            }
            else {
                'P1A artifact content removed because the redaction gate failed.'
            }
            Write-P1AUtf8LfFile -Path $leakedPath -Text $replacement
            foreach ($command in @($commands)) {
                foreach ($streamName in @('stdout', 'stderr')) {
                    if ([string]$command[$streamName].path -ceq $relativeLeak) {
                        $command[$streamName].sha256 = Get-P1ASha256 -Path $leakedPath
                        $command[$streamName].bytes = [int64](Get-Item -LiteralPath $leakedPath).Length
                    }
                }
            }
            if ($relativeLeak -ceq 'artifacts/environment.json') {
                $environmentRef = $null
            }
        }
        $sourceIdentitySha = Get-P1ASha256 -Path $sourceIdentityPath
    }
    else { Set-P1AGate $gates redaction PASS 'run artifacts contain no absolute paths, host/user identity, credentials, or secret-shaped values' }

    $sealReference = [ordered]@{
        path = 'SHA256SUMS'
        entries = @(Get-ChildItem -LiteralPath $runRoot -Recurse -File).Count + 1
        coverage_rule = 'all_run_files_except_seal'
    }
    $finished = [DateTime]::UtcNow
    $evidence = [ordered]@{
        schema = 'python-slm-phase-evidence-v1'; phase_id = 'P1A'; run_id = $runId; mode = 'Cpu'
        status = $status; started_at = $started.ToString('o'); finished_at = $finished.ToString('o')
        duration_ms = [int64]($finished - $started).TotalMilliseconds
        invocation = [ordered]@{
            argv = @('powershell', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
                'scripts/verify-env.ps1', '-Mode', 'Cpu', '-OutputRoot', 'docs/receipts/P1A')
            cwd = '${REPO}'
        }
        source_identity_sha256 = $sourceIdentitySha; p0_dependency = $p0
        environment = $environmentRef; commands = @($commands); gates = $gates; errors = @($errors)
        cleanup = [ordered]@{ attempted = $cleanupAttempted; temporary_root_removed = $temporaryRemoved }
        seal = $sealReference
    }
    $evidencePath = Join-Path $runRoot 'evidence.json'
    Write-P1AJsonFile -Path $evidencePath -Value $evidence -CreateNew
    [void](New-P1ASeal -RunRoot $runRoot)
    if (-not (Test-P1ASeal -RunRoot $runRoot)) { throw '[EVIDENCE_SEAL_FAILED] sealed run did not verify after evidence publication' }
    if ($status -ne 'PASS') {
        return [pscustomobject]@{
            status = 'FAIL'; phase_id = 'P1A'; run_id = $runId
            evidence_path = "docs/receipts/P1A/runs/$runId/evidence.json"; errors = @($errors)
        }
    }
    $publication = Publish-P1AAcceptance -OutputRoot $output -RunId $runId `
        -SourceIdentitySha256 $sourceIdentitySha
    return [pscustomobject]@{
        status = 'PASS'; phase_id = 'P1A'; run_id = $runId
        evidence_path = "docs/receipts/P1A/runs/$runId/evidence.json"
        acceptance_path = $publication.acceptance_path; pointer_path = $publication.pointer_path
    }
    }
    catch {
        $exception = $_.Exception
        $rawMessage = [string]$exception.Message
        $code = if ($exception.Data.Contains('P1ACode')) {
            [string]$exception.Data['P1ACode']
        }
        elseif ($rawMessage -match '^\[(?<code>[A-Z0-9_]+)\]\s*(?<body>.*)$') {
            $rawMessage = $Matches['body']
            $Matches['code']
        }
        else {
            'UNEXPECTED_VERIFIER_FAILURE'
        }
        $category = if ($exception.Data.Contains('P1ACategory')) {
            [int]$exception.Data['P1ACategory']
        }
        else {
            Get-P1AErrorCategory -Code $code
        }
        $remediation = if ($exception.Data.Contains('P1ARemediation')) {
            [string]$exception.Data['P1ARemediation']
        }
        else {
            'Correct the reported prerequisite or verifier failure, then run the exact P1A command again.'
        }
        $extraRoots = if ($null -ne $context) { @($context.RedactionRoots) } else { @() }
        $message = Protect-P1AText -Text $rawMessage -RepositoryRoot $repository -ExtraRoots $extraRoots
        $alreadyRecorded = @($errors | Where-Object {
                [string]$_['code'] -ceq $code -and [string]$_['message'] -ceq $message
            }).Count -gt 0
        if (-not $alreadyRecorded) {
            Add-P1AError -Errors $errors -Code $code -Message $message `
                -Remediation $remediation -Category $category
        }
        if ($gates.Contains($activeGate) -and [string]$gates[$activeGate].status -eq 'NOT_RUN') {
            Set-P1AGate -Gates $gates -Name $activeGate -Status 'FAIL' -Detail $message
        }

        $cleanupAttempted = $true
        if (-not [string]::IsNullOrWhiteSpace($temporaryRoot)) {
            try {
                if (Test-Path -LiteralPath $temporaryRoot) {
                    $resolvedTemp = (Resolve-Path -LiteralPath $temporaryRoot -ErrorAction Stop).Path
                    if ([string]::IsNullOrWhiteSpace($temporaryBase) -or
                        -not (Test-P1APathWithin -Path $resolvedTemp -Root $temporaryBase) -or
                        (Split-Path -Leaf $resolvedTemp) -cnotmatch '^python-slm-p1a-[0-9]{8}T[0-9]{9}Z-[0-9a-f]{24}$') {
                        throw 'unsafe temporary cleanup target'
                    }
                    Remove-Item -LiteralPath $resolvedTemp -Recurse -Force -ErrorAction Stop
                }
                $temporaryRemoved = -not (Test-Path -LiteralPath $temporaryRoot)
            }
            catch {
                $temporaryRemoved = $false
                Add-P1AError -Errors $errors -Code 'TEMP_CLEANUP_FAILED' `
                    -Message 'qualification temporary root could not be safely removed' `
                    -Remediation 'Inspect the recorded temporary-root ownership and remove only that exact directory.' `
                    -Category 5
            }
        }

        return Complete-P1AFailureRun -RunRoot $runRoot -RunId $runId -Started $started `
            -Errors $errors -Commands $commands -Gates $gates -P0 $p0 `
            -CleanupAttempted $cleanupAttempted -TemporaryRemoved $temporaryRemoved
    }
}

Export-ModuleMember -Function @(
    'ConvertTo-P1ACommandLine',
    'ConvertTo-P1ANormalizedPath',
    'Get-P1ASha256',
    'Write-P1AUtf8LfFile',
    'Write-P1AJsonFile',
    'Test-P1APathWithin',
    'Resolve-P1AOutputRoot',
    'New-P1ARunId',
    'ConvertFrom-P1ARustcVersion',
    'Select-P1AVs2022Instance',
    'Protect-P1AText',
    'Invoke-P1AProcess',
    'New-P1ASeal',
    'Test-P1ASeal',
    'Publish-P1AAcceptance',
    'Invoke-P1AVerification'
)

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

function ConvertFrom-P1BNvccVersion {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Text)

    $match = [regex]::Match(
        $Text,
        '(?im)^Cuda compilation tools, release\s+(?<release>[0-9]+\.[0-9]+),\s+V(?<build>[0-9]+(?:\.[0-9]+){2,3})\s*$'
    )
    if (-not $match.Success) {
        throw '[CUDA_VERSION_INVALID] nvcc output did not contain a canonical release/version line'
    }
    $parts = $match.Groups['release'].Value.Split('.')
    $major = [int]$parts[0]
    $minor = [int]$parts[1]
    if ($major -lt 12 -or ($major -eq 12 -and $minor -lt 8)) {
        throw "[CUDA_VERSION_UNSUPPORTED] CUDA $major.$minor cannot target SM120; require CUDA 12.8 or newer"
    }
    $compilerVersion = $match.Groups['build'].Value
    $compilerParts = $compilerVersion.Split('.')
    if ([int]$compilerParts[0] -ne $major -or [int]$compilerParts[1] -ne $minor) {
        throw '[CUDA_VERSION_INVALID] nvcc release and compiler version disagree'
    }
    return [pscustomobject][ordered]@{
        toolkit_release = $match.Groups['release'].Value
        compiler_version = $compilerVersion
        major = $major
        minor = $minor
        build = [int]$compilerParts[-1]
    }
}

function Get-P1BNvccTargets {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$ArchitectureText,
        [Parameter(Mandatory)][string]$CodeText
    )

    $architectures = @($ArchitectureText -split '\r?\n' | ForEach-Object { $_.Trim() } |
        Where-Object { $_ -match '^compute_[0-9]+$' } | Sort-Object -Unique)
    $code = @($CodeText -split '\r?\n' | ForEach-Object { $_.Trim() } |
        Where-Object { $_ -match '^sm_[0-9]+$' } | Sort-Object -Unique)
    if ($architectures -notcontains 'compute_120' -or $code -notcontains 'sm_120') {
        throw '[CUDA_SM120_UNSUPPORTED] nvcc does not advertise both compute_120 PTX and sm_120 machine code'
    }
    return [pscustomobject][ordered]@{
        architectures = $architectures
        code = $code
        sass = @('sm_120')
        ptx = @('compute_120')
    }
}

function ConvertFrom-P1BNvidiaSmi {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Text)

    $lines = @($Text -split '\r?\n' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if ($lines.Count -lt 1) {
        throw '[GPU_INVENTORY_INVALID] nvidia-smi returned no GPU records'
    }
    $devices = [Collections.Generic.List[object]]::new()
    $indices = [Collections.Generic.HashSet[int]]::new()
    foreach ($line in $lines) {
        $fields = @($line.Split(',') | ForEach-Object { $_.Trim() })
        if ($fields.Count -ne 5) {
            throw '[GPU_INVENTORY_INVALID] nvidia-smi record did not contain five fields'
        }
        $index = 0
        $memory = [int64]0
        $capability = [regex]::Match($fields[4], '^(?<major>[0-9]+)\.(?<minor>[0-9]+)$')
        $memoryText = $fields[3] -replace '(?i)\s*MiB\s*$', ''
        if (-not [int]::TryParse($fields[0], [ref]$index) -or $index -lt 0 -or
            -not $indices.Add($index) -or
            -not [int64]::TryParse($memoryText, [ref]$memory) -or $memory -le 0 -or
            -not $capability.Success -or [string]::IsNullOrWhiteSpace($fields[1]) -or
            [string]::IsNullOrWhiteSpace($fields[2])) {
            throw '[GPU_INVENTORY_INVALID] nvidia-smi record contains an invalid, duplicate, or empty field'
        }
        $devices.Add([pscustomobject][ordered]@{
                index = $index
                name = $fields[1]
                driver_version = $fields[2]
                memory_total_mib = $memory
                compute_capability_major = [int]$capability.Groups['major'].Value
                compute_capability_minor = [int]$capability.Groups['minor'].Value
            })
    }
    $targets = @($devices | Where-Object {
            [string]$_.name -ceq 'NVIDIA GeForce RTX 5090' -and
            [int]$_.compute_capability_major -eq 12 -and
            [int]$_.compute_capability_minor -eq 0
        })
    if ($targets.Count -eq 0) {
        throw '[GPU_TARGET_MISSING] no NVIDIA GeForce RTX 5090 with compute capability 12.0 is runtime-visible'
    }
    if ($targets.Count -gt 1) {
        throw '[GPU_SELECTION_AMBIGUOUS] more than one NVIDIA GeForce RTX 5090 with compute capability 12.0 is runtime-visible'
    }
    return [pscustomobject][ordered]@{
        devices = @($devices)
        target_match_count = $targets.Count
        target = $targets[0]
    }
}

function Assert-P1BProbeResult {
    [CmdletBinding()]
    param([Parameter(Mandatory)]$Value)

    Assert-P1AClosedObject -Value $Value -Fields @(
        'schema', 'device_count', 'target_match_count', 'device_index', 'device_name', 'memory_total_bytes',
        'compute_capability_major', 'compute_capability_minor', 'runtime_version',
        'driver_version', 'cublas_version', 'cublaslt_version', 'sentinel'
    ) -Name 'CUDA device probe result'
    if ([string]$Value.schema -cne 'python-slm-cuda-device-probe-v1' -or
        [int]$Value.device_count -lt 1 -or [int]$Value.target_match_count -ne 1 -or
        [int]$Value.device_index -lt 0 -or
        [string]$Value.device_name -cne 'NVIDIA GeForce RTX 5090' -or
        [int64]$Value.memory_total_bytes -le 0 -or
        [int]$Value.compute_capability_major -ne 12 -or
        [int]$Value.compute_capability_minor -ne 0 -or
        [int]$Value.runtime_version -lt 12080 -or
        [int]$Value.driver_version -lt [int]$Value.runtime_version -or
        [int]$Value.cublas_version -le 0 -or [int64]$Value.cublaslt_version -le 0 -or
        [int]$Value.sentinel -ne 42) {
        throw '[CUDA_DEVICE_PROBE_INVALID] CUDA device probe did not satisfy the RTX 5090 runtime contract'
    }
    return $Value
}

function Test-P1BContainedPathWithoutReparse {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$Root,
        [switch]$Leaf
    )

    $rootFull = [IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    $pathFull = [IO.Path]::GetFullPath($Path)
    $exists = if ($Leaf) {
        Test-Path -LiteralPath $pathFull -PathType Leaf
    }
    else {
        Test-Path -LiteralPath $pathFull
    }
    if (-not (Test-P1APathWithin -Path $pathFull -Root $rootFull) -or -not $exists) {
        return $false
    }
    $cursor = $pathFull
    while (Test-P1APathWithin -Path $cursor -Root $rootFull) {
        $item = Get-Item -Force -LiteralPath $cursor
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            return $false
        }
        if ([string]::Equals(
                $cursor.TrimEnd('\', '/'), $rootFull,
                [StringComparison]::OrdinalIgnoreCase
            )) {
            break
        }
        $cursor = Split-Path -Parent $cursor
    }
    $cursor = Split-Path -Parent $rootFull
    while (-not [string]::IsNullOrWhiteSpace($cursor)) {
        if (Test-Path -LiteralPath $cursor) {
            $item = Get-Item -Force -LiteralPath $cursor
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                return $false
            }
        }
        $parent = Split-Path -Parent $cursor
        if ([string]::Equals($parent, $cursor, [StringComparison]::OrdinalIgnoreCase)) {
            break
        }
        $cursor = $parent
    }
    return $true
}

function Select-P1BCudaToolkit {
    [CmdletBinding()]
    param([Parameter(Mandatory)][object[]]$Candidates)

    $candidateByRoot = [Collections.Generic.Dictionary[string,object]]::new(
        [StringComparer]::OrdinalIgnoreCase
    )
    foreach ($candidate in @($Candidates)) {
        Assert-P1AClosedObject -Value $candidate -Fields @('root', 'source', 'explicit') `
            -Name 'CUDA toolkit candidate'
        if ([string]::IsNullOrWhiteSpace([string]$candidate.root) -or
            [string]::IsNullOrWhiteSpace([string]$candidate.source)) {
            throw '[CUDA_TOOLKIT_CANDIDATE_INVALID] CUDA toolkit candidate is incomplete'
        }
        $candidateRoot = [IO.Path]::GetFullPath([string]$candidate.root).TrimEnd('\', '/')
        if ($candidateByRoot.ContainsKey($candidateRoot)) {
            $prior = $candidateByRoot[$candidateRoot]
            $candidateByRoot[$candidateRoot] = [pscustomobject][ordered]@{
                root = $candidateRoot
                source = (@([string]$prior.source, [string]$candidate.source) |
                    Sort-Object -Unique) -join '+'
                explicit = [bool]$prior.explicit -or [bool]$candidate.explicit
            }
        }
        else {
            $candidateByRoot[$candidateRoot] = [pscustomobject][ordered]@{
                root = $candidateRoot
                source = [string]$candidate.source
                explicit = [bool]$candidate.explicit
            }
        }
    }
    $eligible = [Collections.Generic.List[object]]::new()
    foreach ($candidate in @($candidateByRoot.Values | Sort-Object root)) {
        $root = [IO.Path]::GetFullPath([string]$candidate.root).TrimEnd('\', '/')
        if (-not (Test-Path -LiteralPath $root -PathType Container) -or
            -not (Test-P1BContainedPathWithoutReparse -Path $root -Root $root)) {
            continue
        }
        $versionPath = Join-Path $root 'version.json'
        if (-not (Test-P1BContainedPathWithoutReparse -Path $versionPath -Root $root -Leaf)) {
            continue
        }
        try {
            $versionDocument = [IO.File]::ReadAllText($versionPath, $script:Utf8NoBom) |
                ConvertFrom-Json
            if ([string]$versionDocument.cuda.name -cne 'CUDA SDK' -or
                [string]$versionDocument.cuda.version -cnotmatch '^[0-9]+\.[0-9]+\.[0-9]+$') {
                throw 'CUDA version document is not canonical'
            }
            $version = [version][string]$versionDocument.cuda.version
            if ($version.Major -lt 12 -or ($version.Major -eq 12 -and $version.Minor -lt 8)) {
                throw 'CUDA version cannot target SM120'
            }
        }
        catch {
            continue
        }

        $toolRelative = [ordered]@{
            nvcc = 'bin\nvcc.exe'
            ptxas = 'bin\ptxas.exe'
            fatbinary = 'bin\fatbinary.exe'
            nvlink = 'bin\nvlink.exe'
            cuobjdump = 'bin\cuobjdump.exe'
        }
        $headerRelative = [ordered]@{
            cuda = 'include\cuda.h'
            cuda_runtime = 'include\cuda_runtime.h'
            cublas = 'include\cublas_v2.h'
            cublaslt = 'include\cublasLt.h'
        }
        $libraryRelative = [ordered]@{
            cuda = 'lib\x64\cuda.lib'
            cudart = 'lib\x64\cudart.lib'
            cublas = 'lib\x64\cublas.lib'
            cublaslt = 'lib\x64\cublasLt.lib'
        }
        $tools = [ordered]@{}
        $headers = [ordered]@{}
        $libraries = [ordered]@{}
        $missing = [Collections.Generic.List[string]]::new()
        foreach ($entry in $toolRelative.GetEnumerator()) {
            $path = Join-Path $root ([string]$entry.Value)
            if (-not (Test-P1BContainedPathWithoutReparse -Path $path -Root $root -Leaf)) {
                $missing.Add([string]$entry.Value)
            }
            else { $tools[[string]$entry.Key] = $path }
        }
        foreach ($entry in $headerRelative.GetEnumerator()) {
            $path = Join-Path $root ([string]$entry.Value)
            if (-not (Test-P1BContainedPathWithoutReparse -Path $path -Root $root -Leaf)) {
                $missing.Add([string]$entry.Value)
            }
            else { $headers[[string]$entry.Key] = $path }
        }
        foreach ($entry in $libraryRelative.GetEnumerator()) {
            $path = Join-Path $root ([string]$entry.Value)
            if (-not (Test-P1BContainedPathWithoutReparse -Path $path -Root $root -Leaf)) {
                $missing.Add([string]$entry.Value)
            }
            else { $libraries[[string]$entry.Key] = $path }
        }
        $runtimeDlls = [ordered]@{}
        foreach ($entry in ([ordered]@{
                    cudart = 'cudart64_*.dll'
                    cublas = 'cublas64_*.dll'
                    cublaslt = 'cublasLt64_*.dll'
                }).GetEnumerator()) {
            $matches = @(Get-ChildItem -LiteralPath (Join-Path $root 'bin\x64') `
                    -Filter ([string]$entry.Value) -File -ErrorAction SilentlyContinue | Where-Object {
                    Test-P1BContainedPathWithoutReparse -Path $_.FullName -Root $root -Leaf
                })
            if ($matches.Count -ne 1) {
                $missing.Add('bin\x64\' + [string]$entry.Value)
            }
            else { $runtimeDlls[[string]$entry.Key] = $matches[0].FullName }
        }
        if ($missing.Count -ne 0) {
            continue
        }
        $optionalTools = [ordered]@{}
        foreach ($entry in ([ordered]@{ nvdisasm = 'bin\nvdisasm.exe' }).GetEnumerator()) {
            $path = Join-Path $root ([string]$entry.Value)
            if (Test-P1BContainedPathWithoutReparse -Path $path -Root $root -Leaf) {
                $optionalTools[[string]$entry.Key] = $path
            }
        }
        $optionalInventory = [ordered]@{}
        foreach ($entry in ([ordered]@{
                    curand_header = 'include\curand.h'
                    curand_library = 'lib\x64\curand.lib'
                    nvrtc_header = 'include\nvrtc.h'
                    nvrtc_library = 'lib\x64\nvrtc.lib'
                    nvjitlink_header = 'include\nvJitLink.h'
                    nvjitlink_library = 'lib\x64\nvJitLink.lib'
                    compute_sanitizer = 'compute-sanitizer\compute-sanitizer.exe'
                    cudnn_header = 'include\cudnn.h'
                    cudnn_library = 'lib\x64\cudnn.lib'
                }).GetEnumerator()) {
            $path = Join-Path $root ([string]$entry.Value)
            if (Test-P1BContainedPathWithoutReparse -Path $path -Root $root -Leaf) {
                $optionalInventory[[string]$entry.Key] = $path
            }
        }
        foreach ($entry in ([ordered]@{
                    curand_runtime = 'curand64_*.dll'
                    nvrtc_runtime = 'nvrtc64_*.dll'
                    nvjitlink_runtime = 'nvJitLink_*.dll'
                    cudnn_runtime = 'cudnn*.dll'
                }).GetEnumerator()) {
            $matches = @(Get-ChildItem -LiteralPath (Join-Path $root 'bin\x64') `
                    -Filter ([string]$entry.Value) -File -ErrorAction SilentlyContinue | Where-Object {
                    Test-P1BContainedPathWithoutReparse -Path $_.FullName -Root $root -Leaf
                } | Sort-Object FullName)
            if ($matches.Count -gt 0) {
                $optionalInventory[[string]$entry.Key] = @($matches | ForEach-Object { $_.FullName })
            }
        }
        $eligible.Add([pscustomobject][ordered]@{
                root = $root
                source = [string]$candidate.source
                explicit = [bool]$candidate.explicit
                version = $version.ToString(3)
                major = $version.Major
                minor = $version.Minor
                tools = $tools
                optional_tools = $optionalTools
                optional_inventory = $optionalInventory
                headers = $headers
                libraries = $libraries
                runtime_dlls = $runtimeDlls
            })
    }
    $selected = @($eligible | Sort-Object -Property `
            @{ Expression = { [version]$_.version }; Descending = $true },
            @{ Expression = { ([string]$_.root).ToLowerInvariant() }; Descending = $false })
    if ($selected.Count -eq 0) {
        throw '[CUDA_TOOLKIT_NOT_FOUND] no complete non-reparse CUDA 12.8-or-newer toolkit was found'
    }
    return $selected[0]
}

function Get-P1BSelectedP1ADependency {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$RepositoryRoot)

    $repository = [IO.Path]::GetFullPath($RepositoryRoot).TrimEnd('\', '/')
    $p1aRoot = Join-Path $repository 'docs\receipts\P1A'
    $pointerPath = Join-Path $p1aRoot 'evidence.json'
    if (-not (Test-P1BContainedPathWithoutReparse -Path $pointerPath -Root $p1aRoot -Leaf)) {
        throw '[P1A_DEPENDENCY_INVALID] selected P1A pointer is missing or unsafe'
    }
    $pointer = [IO.File]::ReadAllText($pointerPath, $script:Utf8NoBom) | ConvertFrom-Json
    Assert-P1AClosedObject -Value $pointer -Fields @(
        'schema', 'phase_id', 'acceptance_path', 'acceptance_sha256', 'updated_at'
    ) -Name 'selected P1A pointer'
    if ([string]$pointer.schema -cne 'python-slm-phase-evidence-pointer-v1' -or
        [string]$pointer.phase_id -cne 'P1A' -or
        [string]$pointer.acceptance_path -cnotmatch '^acceptances/[0-9]{8}\.json$') {
        throw '[P1A_DEPENDENCY_INVALID] selected P1A pointer identity or path is invalid'
    }
    Assert-P1ASha256Value -Value ([string]$pointer.acceptance_sha256) `
        -Name 'selected P1A acceptance hash'
    $acceptancePath = [IO.Path]::GetFullPath((Join-Path $p1aRoot `
                ([string]$pointer.acceptance_path).Replace('/', '\')))
    $acceptancesRoot = Join-Path $p1aRoot 'acceptances'
    if (-not (Test-P1BContainedPathWithoutReparse -Path $acceptancePath `
            -Root $acceptancesRoot -Leaf) -or
        (Get-P1ASha256 -Path $acceptancePath) -cne [string]$pointer.acceptance_sha256) {
        throw '[P1A_DEPENDENCY_INVALID] selected P1A acceptance is missing or hash-invalid'
    }

    $allAcceptances = @(Get-ChildItem -LiteralPath $acceptancesRoot -File -ErrorAction Stop |
        Sort-Object Name)
    $previousHash = $null
    $selectedAcceptance = $null
    for ($index = 0; $index -lt $allAcceptances.Count; $index++) {
        $expectedSequence = $index + 1
        $file = $allAcceptances[$index]
        if ($file.Name -cne ($expectedSequence.ToString('00000000') + '.json')) {
            throw '[P1A_DEPENDENCY_INVALID] P1A acceptance chain has a gap or noncanonical file'
        }
        $acceptance = [IO.File]::ReadAllText($file.FullName, $script:Utf8NoBom) |
            ConvertFrom-Json
        Assert-P1AAcceptanceObject -Acceptance $acceptance -Sequence $expectedSequence `
            -PreviousHash $previousHash -OutputRoot $p1aRoot
        if ([string]::Equals($file.FullName, $acceptancePath, [StringComparison]::OrdinalIgnoreCase)) {
            $selectedAcceptance = $acceptance
        }
        $previousHash = Get-P1ASha256 -Path $file.FullName
    }
    if ($null -eq $selectedAcceptance) {
        throw '[P1A_DEPENDENCY_INVALID] P1A pointer does not select an acceptance in the validated chain'
    }
    $runId = Split-Path -Leaf ([string]$selectedAcceptance.run_path)
    $runRoot = Join-Path (Join-Path $p1aRoot 'runs') $runId
    if (-not (Test-P1ASeal -RunRoot $runRoot)) {
        throw '[P1A_DEPENDENCY_INVALID] selected P1A run seal is invalid'
    }
    $evidencePath = Join-Path $runRoot 'evidence.json'
    $environmentPath = Join-Path $runRoot 'artifacts\environment.json'
    $sourcePath = Join-Path $runRoot 'artifacts\source-identity.json'
    $sealPath = Join-Path $runRoot 'SHA256SUMS'
    $evidenceText = [IO.File]::ReadAllText($evidencePath, $script:Utf8NoBom)
    $evidence = $evidenceText | ConvertFrom-Json
    $environment = [IO.File]::ReadAllText($environmentPath, $script:Utf8NoBom) | ConvertFrom-Json
    if ([string]$evidence.phase_id -cne 'P1A' -or [string]$evidence.mode -cne 'Cpu' -or
        [string]$evidence.status -cne 'PASS' -or [string]$evidence.run_id -cne $runId -or
        [string]$environment.phase_id -cne 'P1A' -or [string]$environment.status -cne 'PASS' -or
        [string]$environment.run_id -cne $runId) {
        throw '[P1A_DEPENDENCY_INVALID] selected P1A run or environment is not PASS'
    }
    if ((Get-P1ASha256 -Path $evidencePath) -cne [string]$selectedAcceptance.run_evidence_sha256 -or
        (Get-P1ASha256 -Path $sealPath) -cne [string]$selectedAcceptance.seal_sha256 -or
        (Get-P1ASha256 -Path $environmentPath) -cne [string]$selectedAcceptance.environment_sha256 -or
        (Get-P1ASha256 -Path $sourcePath) -cne [string]$selectedAcceptance.source_identity_sha256) {
        throw '[P1A_DEPENDENCY_INVALID] selected P1A acceptance does not pin its run artifacts'
    }
    $reviewClosureCommit = '9359c989fa63d4a300abc509e735b7e81a24a2ea'
    $git = Get-P1AApplicationPath -Name 'git.exe'
    $ancestor = Invoke-P1AProcess -FilePath $git -ArgumentList @(
        'merge-base', '--is-ancestor', $reviewClosureCommit, 'HEAD'
    ) -WorkingDirectory $repository -Environment @{} -TimeoutSeconds 30
    if ($ancestor.timed_out -or $ancestor.exit_code -ne 0) {
        throw '[P1A_DEPENDENCY_INVALID] P1A review-closure commit is not an ancestor of HEAD'
    }
    return [ordered]@{
        status = 'PASS'
        pointer_path = 'docs/receipts/P1A/evidence.json'
        pointer_sha256 = Get-P1ASha256 -Path $pointerPath
        acceptance_path = 'docs/receipts/P1A/' + ([string]$pointer.acceptance_path)
        acceptance_sha256 = Get-P1ASha256 -Path $acceptancePath
        acceptance_sequence = [int]$selectedAcceptance.sequence
        run_path = "docs/receipts/P1A/runs/$runId"
        run_evidence_sha256 = Get-P1ASha256 -Path $evidencePath
        seal_path = "docs/receipts/P1A/runs/$runId/SHA256SUMS"
        seal_sha256 = Get-P1ASha256 -Path $sealPath
        environment_path = "docs/receipts/P1A/runs/$runId/artifacts/environment.json"
        environment_sha256 = Get-P1ASha256 -Path $environmentPath
        source_identity_sha256 = Get-P1ASha256 -Path $sourcePath
        verifier_sha256 = [string]$environment.source.verifier_sha256
        schema_bundle_sha256 = [string]$environment.source.schema_bundle_sha256
        review_closure_commit = $reviewClosureCommit
    }
}

function New-P1BGates {
    $gates = [ordered]@{}
    foreach ($name in @(
            'p0', 'p1a', 'cpu_regression', 'input_stability', 'toolchain',
            'cuda_toolkit', 'architecture_targets', 'gpu_identity', 'driver_runtime',
            'device_probe', 'python_isolation', 'redaction', 'cleanup'
        )) {
        $gates[$name] = [ordered]@{ status = 'NOT_RUN'; detail = 'not reached' }
    }
    return $gates
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
        elseif ($fullRoot -match '(?i)[\\/]NVIDIA GPU Computing Toolkit[\\/]CUDA[\\/]v[0-9.]+$') {
            'CUDA_TOOLKIT'
        }
        elseif ($fullRoot -match '(?i)[\\/]docs[\\/]receipts[\\/]P1[AB]$') {
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
        @((Split-Path -Leaf $DisplayFile)) + $Arguments | ForEach-Object {
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
        if ($normalized.StartsWith('docs/receipts/P1A/', [StringComparison]::OrdinalIgnoreCase) -or
            $normalized.StartsWith('docs/receipts/P1B/', [StringComparison]::OrdinalIgnoreCase)) {
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
    if ($Code -match '^(P0_|P1A_|INPUT_|CARGO_LOCK_|CARGO_CONFIG_|REPOSITORY_|PARENT_|REDACTION_|UNSAFE_|CHILD_ENVIRONMENT_)') {
        return 3
    }
    if ($Code -match '^(UNSUPPORTED_|TOOL_|RUST_|CARGO_VERSION_|VS_|VSWHERE_|VSDEVCMD_|MSVC_TOOL_|WINDOWS_SDK_|GIT_|TARGET_NOT_CLEAN|TEMP_PATH_)') {
        return 4
    }
    if ($Code -match '^(MODE_NOT_IMPLEMENTED|COMMAND_|NATIVE_|CANARY_|CPU_|CLEAN_|CARGO_METADATA_|CARGO_TREE_|CARGO_MESSAGE_|CARGO_ARTIFACT_|CUDA_|GPU_|PYTHON_|PE_|EVIDENCE_|TEMP_CLEANUP_)') {
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
    $environmentPolicyPattern = '^(?i:Path|PATHEXT|SystemRoot|WINDIR|ComSpec|TEMP|TMP|USERPROFILE|HOME|INCLUDE|LIB|LIBPATH|CL|_CL_|LINK|_LINK_|VS.*|VC.*|WindowsSDK.*|UCRT.*|UniversalCRTSdkDir|CARGO_.*|RUST.*|CUDA.*|CUDNN.*|NVCC.*|SCCACHE.*|NO_COLOR|CC_.*|AR_.*|P1[AB]_CANARY_DIR)$'
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
        if ($normalized.StartsWith('docs/receipts/P1A/', [StringComparison]::OrdinalIgnoreCase) -or
            $normalized.StartsWith('docs/receipts/P1B/', [StringComparison]::OrdinalIgnoreCase)) {
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
            -Remediation 'Use -Mode Cpu for P1A or -Mode Cuda for P1B.'
    }
    $phaseId = if ($Mode -ceq 'Cuda') { 'P1B' } else { 'P1A' }
    $output = Resolve-P1AOutputRoot -RepositoryRoot $repository -OutputRoot $OutputRoot -PhaseId $phaseId
    if ($Mode -ceq 'Cuda') {
        return Invoke-P1BVerification -OutputRoot $output -RepositoryRoot $repository `
            -ScriptPath $ScriptPath -Started $started
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
        (Join-Path $PSScriptRoot 'VerifyCuda.ps1'),
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

. (Join-Path $PSScriptRoot 'VerifyCuda.ps1')

function New-P1BNvccArguments {
    param(
        [Parameter(Mandatory)][ValidateSet('Mixed', 'PtxOnly')][string]$Kind,
        [Parameter(Mandatory)][string]$SourcePath,
        [Parameter(Mandatory)][string]$OutputPath,
        [Parameter(Mandatory)][string]$CudaRoot,
        [Parameter(Mandatory)][string]$CompilerDirectory,
        [Parameter(Mandatory)][int]$ToolkitMajor
    )
    $runtimeLinkage = if ($ToolkitMajor -ge 13) { 'hybrid' } else { 'shared' }
    $linkerOptions = if ($ToolkitMajor -ge 13) {
        # CUDA 13's Windows hybrid loader carries a LIBCMT default-library
        # directive. The probe is contractually /MD, so ignore that conflicting
        # static CRT while preserving warning-as-error for every other warning.
        '--linker-options=/WX,/NODEFAULTLIB:LIBCMT'
    }
    else { '--linker-options=/WX' }
    $arguments = @(
        '-m64', '-std=c++17', '-O2', ('--cudart=' + $runtimeLinkage),
        '--compiler-options=/EHsc,/W4,/WX,/MD',
        $linkerOptions,
        '-ccbin', $CompilerDirectory,
        ('-I=' + (Join-Path $CudaRoot 'include')),
        ('-L=' + (Join-Path $CudaRoot 'lib\x64')),
        $(if ($Kind -ceq 'Mixed') {
                '-gencode=arch=compute_120,code=[sm_120,compute_120]'
            }
            else {
                '-gencode=arch=compute_120,code=compute_120'
            })
    )
    $arguments += @(
        '-o', $OutputPath, $SourcePath,
        (Join-Path $CudaRoot 'lib\x64\cuda.lib'),
        (Join-Path $CudaRoot 'lib\x64\cublas.lib'),
        (Join-Path $CudaRoot 'lib\x64\cublasLt.lib')
    )
    return $arguments
}

function Assert-P1BArtifactInspection {
    param(
        [Parameter(Mandatory)][ValidateSet('Mixed', 'PtxOnly')][string]$Kind,
        [AllowEmptyString()][string]$ListElfText = '',
        [AllowEmptyString()][string]$ListPtxText = '',
        [AllowEmptyString()][string]$SassText = '',
        [AllowEmptyString()][string]$PtxText = ''
    )
    $elfRecords = @([regex]::Matches($ListElfText,
            '(?im)^\s*ELF file\s+\d+:\s+(?<name>\S+)\s*$'))
    $ptxRecords = @([regex]::Matches($ListPtxText,
            '(?im)^\s*PTX file\s+\d+:\s+(?<name>\S+)\s*$'))
    $allElfSm120 = $elfRecords.Count -gt 0 -and
        @($elfRecords | Where-Object {
                $_.Groups['name'].Value -cnotmatch '\.sm_120\.cubin$'
            }).Count -eq 0
    $allPtxCompute120 = $ptxRecords.Count -gt 0 -and
        @($ptxRecords | Where-Object {
                $_.Groups['name'].Value -cnotmatch '\.sm_120\.ptx$'
            }).Count -eq 0
    # `cuobjdump --dump-sass` prints the PTX fatbin header (including
    # `arch = sm_120`) even when the executable has no ELF/SASS image. Count
    # SASS only when every listed cubin is SM120 and the generated sentinel
    # kernel's function block contains a real encoded instruction.
    $sentinelSassBlock = [regex]::Match($SassText,
        '(?ims)^\s*Function\s*:\s*\S*add_sentinel\S*\s*$.*?(?=^\s*Function\s*:|\z)')
    $hasSm120Sass = $allElfSm120 -and
        $SassText -match '(?im)^\s*code for sm_120\s*$' -and
        $sentinelSassBlock.Success -and
        $sentinelSassBlock.Value -match '(?im)^\s*/\*[0-9a-f]{4,8}\*/\s+[A-Z][A-Z0-9_.]*\b.*;\s*/\*\s*0x[0-9a-f]{16}\s*\*/\s*$'
    $hasPtxPayload = $allPtxCompute120 -and
        $PtxText -match '(?im)^\s*\.version\s+' -and
        $PtxText -match '(?im)^\s*\.target\s+sm_120(?:\s|,|$)' -and
        $PtxText -match '(?im)^\s*(?:\.visible\s+)?\.entry\s+\S*add_sentinel\S*'
    if (-not $hasPtxPayload) {
        throw '[CUDA_PTX_INSPECTION_FAILED] artifact does not contain inspectable compute_120 PTX'
    }
    if ($Kind -ceq 'Mixed') {
        if (-not $hasSm120Sass) {
            throw '[CUDA_SASS_INSPECTION_FAILED] mixed artifact does not contain inspectable sm_120 SASS'
        }
        return [ordered]@{ embedded_sass = @('sm_120'); embedded_ptx = @('compute_120') }
    }
    if ($elfRecords.Count -ne 0 -or $SassText -match '(?im)^\s*Function\s*:') {
        throw '[CUDA_PTX_ONLY_INSPECTION_FAILED] PTX-only artifact unexpectedly contains SASS'
    }
    return [ordered]@{ embedded_sass = @(); embedded_ptx = @('compute_120') }
}

function Assert-P1BPeInspection {
    param(
        [Parameter(Mandatory)][string]$HeadersText,
        [Parameter(Mandatory)][string]$DependentsText,
        [Parameter(Mandatory)][ValidateSet('Hybrid', 'Shared')][string]$RuntimeLinkage
    )
    if ($HeadersText -notmatch '(?im)(?:^|\s)8664\s+machine\s+\(x64\)|machine\s+\(x64\)') {
        throw '[CUDA_PE_ARCH_INVALID] CUDA probe executable is not x64 PE'
    }
    $imports = @([regex]::Matches($DependentsText, '(?im)^\s*(?<dll>[A-Za-z0-9_.+-]+\.dll)\s*$') |
        ForEach-Object { $_.Groups['dll'].Value.ToLowerInvariant() } | Sort-Object -Unique)
    if ($imports.Count -lt 1) { throw '[CUDA_PE_IMPORTS_EMPTY] CUDA probe has no recorded imports' }
    # CUDA 13 on Windows uses the compiler's hybrid runtime, loaded from the
    # display driver, so nvcuda/cudart need not be direct PE imports. The probe
    # still calls both APIs and validates their versions at runtime. cuBLAS and
    # cuBLASLt remain direct dynamic-library boundaries and must be imported.
    foreach ($pattern in @('^cublas64_[0-9]+\.dll$', '^cublaslt64_[0-9]+\.dll$')) {
        if (@($imports | Where-Object { $_ -match $pattern }).Count -ne 1) {
            throw '[CUDA_PE_IMPORTS_INVALID] CUDA probe does not import the complete cuBLAS boundary'
        }
    }
    if ($RuntimeLinkage -ceq 'Shared' -and
        @($imports | Where-Object { $_ -match '^cudart64_[0-9]+\.dll$' }).Count -ne 1) {
        throw '[CUDA_PE_IMPORTS_INVALID] shared CUDA runtime is not a direct PE dependency'
    }
    $allowed = @(
        '^nvcuda\.dll$', '^cudart64_[0-9]+\.dll$', '^cublas64_[0-9]+\.dll$',
        '^cublaslt64_[0-9]+\.dll$', '^kernel32\.dll$', '^ntdll\.dll$',
        '^ucrtbase\.dll$', '^vcruntime140(?:_1)?\.dll$', '^msvcp140(?:_[0-9]+)?\.dll$',
        '^concrt140\.dll$', '^api-ms-win-[a-z0-9_-]+\.dll$'
    )
    foreach ($import in $imports) {
        if (@($allowed | Where-Object { $import -match $_ }).Count -eq 0) {
            throw "[CUDA_PE_IMPORTS_INVALID] CUDA probe imports an unapproved DLL: $import"
        }
    }
    return $imports
}

function New-P1BProbeSource {
    return @'
#include <cuda.h>
#include <cuda_runtime.h>
#include <cublas_v2.h>
#include <cublasLt.h>
#include <cstdio>
#include <cstring>

__global__ void add_sentinel(int* value) { if (blockIdx.x == 0 && threadIdx.x == 0) { *value += 35; } }

int main() {
    int device_count = 0, target_matches = 0, target_index = -1;
    int runtime_version = 0, driver_version = 0, direct_driver_version = 0;
    int cublas_version = 0, host_value = 7, result_value = 0;
    size_t cublaslt_version = 0;
    int* device_value = nullptr;
    cublasHandle_t cublas = nullptr;
    cublasLtHandle_t cublaslt = nullptr;
    cudaDeviceProp property = {};
    cudaDeviceProp target_property = {};
    bool selected = false;
    int failure = 0;

#define CUDA_TRY(expr) do { cudaError_t e = (expr); if (e != cudaSuccess) { std::fprintf(stderr, "CUDA failure %d at %s\n", (int)e, #expr); failure = 10; goto cleanup; } } while (0)
#define DRIVER_TRY(expr) do { CUresult e = (expr); if (e != CUDA_SUCCESS) { std::fprintf(stderr, "driver failure %d at %s\n", (int)e, #expr); failure = 11; goto cleanup; } } while (0)
#define CUBLAS_TRY(expr) do { cublasStatus_t e = (expr); if (e != CUBLAS_STATUS_SUCCESS) { std::fprintf(stderr, "cuBLAS failure %d at %s\n", (int)e, #expr); failure = 12; goto cleanup; } } while (0)

    CUDA_TRY(cudaGetDeviceCount(&device_count));
    for (int i = 0; i < device_count; ++i) {
        CUDA_TRY(cudaGetDeviceProperties(&property, i));
        if (std::strcmp(property.name, "NVIDIA GeForce RTX 5090") == 0 &&
            property.major == 12 && property.minor == 0) {
            ++target_matches;
            target_index = i;
            target_property = property;
        }
    }
    if (target_matches != 1) { std::fprintf(stderr, "target match count %d\n", target_matches); return 13; }
    CUDA_TRY(cudaSetDevice(target_index)); selected = true;
    DRIVER_TRY(cuInit(0));
    DRIVER_TRY(cuDriverGetVersion(&direct_driver_version));
    CUDA_TRY(cudaRuntimeGetVersion(&runtime_version));
    CUDA_TRY(cudaDriverGetVersion(&driver_version));
    if (driver_version != direct_driver_version) { std::fprintf(stderr, "driver version disagreement\n"); failure = 14; goto cleanup; }
    CUBLAS_TRY(cublasCreate(&cublas));
    CUBLAS_TRY(cublasGetVersion(cublas, &cublas_version));
    CUBLAS_TRY(cublasLtCreate(&cublaslt));
    cublaslt_version = cublasLtGetVersion();
    if (cublaslt_version == 0) { std::fprintf(stderr, "cuBLASLt version is zero\n"); failure = 15; goto cleanup; }
    CUDA_TRY(cudaMalloc(reinterpret_cast<void**>(&device_value), sizeof(int)));
    CUDA_TRY(cudaMemcpy(device_value, &host_value, sizeof(int), cudaMemcpyHostToDevice));
    add_sentinel<<<1, 1>>>(device_value);
    CUDA_TRY(cudaGetLastError());
    CUDA_TRY(cudaDeviceSynchronize());
    CUDA_TRY(cudaMemcpy(&result_value, device_value, sizeof(int), cudaMemcpyDeviceToHost));
    if (result_value != 42) { std::fprintf(stderr, "sentinel mismatch %d\n", result_value); failure = 16; goto cleanup; }

cleanup:
    if (device_value != nullptr) { cudaError_t e = cudaFree(device_value); device_value = nullptr; if (e != cudaSuccess && failure == 0) failure = 17; }
    if (cublaslt != nullptr) { cublasStatus_t e = cublasLtDestroy(cublaslt); cublaslt = nullptr; if (e != CUBLAS_STATUS_SUCCESS && failure == 0) failure = 18; }
    if (cublas != nullptr) { cublasStatus_t e = cublasDestroy(cublas); cublas = nullptr; if (e != CUBLAS_STATUS_SUCCESS && failure == 0) failure = 19; }
    if (selected) { cudaError_t e = cudaDeviceReset(); if (e != cudaSuccess && failure == 0) failure = 20; }
    if (failure != 0) return failure;
    std::printf("{\"schema\":\"python-slm-cuda-device-probe-v1\",\"device_count\":%d,\"target_match_count\":%d,\"device_index\":%d,\"device_name\":\"%s\",\"memory_total_bytes\":%llu,\"compute_capability_major\":%d,\"compute_capability_minor\":%d,\"runtime_version\":%d,\"driver_version\":%d,\"cublas_version\":%d,\"cublaslt_version\":%llu,\"sentinel\":%d}\n",
        device_count, target_matches, target_index, target_property.name,
        static_cast<unsigned long long>(target_property.totalGlobalMem),
        target_property.major, target_property.minor, runtime_version, driver_version,
        cublas_version, static_cast<unsigned long long>(cublaslt_version), result_value);
    return 0;
}
'@
}

function Get-P1BCudaCandidates {
    param()
    $values = [Collections.Generic.List[object]]::new()
    foreach ($scopeName in @('Process', 'Machine')) {
        try { $scope = [EnvironmentVariableTarget]::$scopeName } catch { continue }
        try { $variables = [Environment]::GetEnvironmentVariables($scope) } catch { continue }
        foreach ($entry in $variables.GetEnumerator()) {
            $name = [string]$entry.Key
            if ($name -match '^CUDA_PATH(?:_V[0-9_]+)?$' -and
                -not [string]::IsNullOrWhiteSpace([string]$entry.Value)) {
                $values.Add([pscustomobject]@{
                        root = [string]$entry.Value
                        source = ($scopeName.ToLowerInvariant() + ':' + $name)
                        explicit = $true
                    })
            }
        }
    }
    $programFiles = [Environment]::GetFolderPath([Environment+SpecialFolder]::ProgramFiles)
    if (-not [string]::IsNullOrWhiteSpace($programFiles)) {
        $parent = Join-Path $programFiles 'NVIDIA GPU Computing Toolkit\CUDA'
        if (Test-Path -LiteralPath $parent -PathType Container) {
            foreach ($directory in Get-ChildItem -LiteralPath $parent -Directory -ErrorAction SilentlyContinue) {
                if ($directory.Name -match '^v[0-9]+\.[0-9]+$') {
                    $values.Add([pscustomobject]@{
                            root = $directory.FullName
                            source = 'standard-installation'
                            explicit = $false
                        })
                }
            }
        }
    }
    if ($values.Count -eq 0) { throw '[CUDA_TOOLKIT_NOT_FOUND] no CUDA toolkit candidates were discovered' }
    return @($values)
}

function New-P1BComponentDescriptor {
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][hashtable]$Roots)
    return [ordered]@{
        name = Split-Path -Leaf $Path
        path = ConvertTo-P1ATokenizedToolPath -Path $Path -Roots $Roots
        sha256 = Get-P1ASha256 -Path $Path
        bytes = [int64](Get-Item -LiteralPath $Path).Length
    }
}

function New-P1BOptionalComponentRecord {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)]$Toolkit,
        [Parameter(Mandatory)][hashtable]$Roots
    )
    $keys = switch ($Name) {
        'cudnn' { @('cudnn_header', 'cudnn_library', 'cudnn_runtime') }
        'nvrtc' { @('nvrtc_header', 'nvrtc_library', 'nvrtc_runtime') }
        'nvjitlink' { @('nvjitlink_header', 'nvjitlink_library', 'nvjitlink_runtime') }
        'compute_sanitizer' { @('compute_sanitizer') }
        'curand' { @('curand_header', 'curand_library', 'curand_runtime') }
    }
    $selectedPath = $null
    foreach ($key in $keys) {
        if ($Toolkit.optional_inventory.Contains($key)) {
            $value = $Toolkit.optional_inventory[$key]
            $candidate = if ($value -is [Array]) { @($value | Sort-Object)[0] } else { [string]$value }
            if (-not [string]::IsNullOrWhiteSpace([string]$candidate)) { $selectedPath = [string]$candidate; break }
        }
    }
    if ($null -eq $selectedPath) {
        return [ordered]@{ present = $false; version = $null; path = $null; sha256 = $null }
    }
    return [ordered]@{
        present = $true
        version = $null
        path = ConvertTo-P1ATokenizedToolPath -Path $selectedPath -Roots $Roots
        sha256 = Get-P1ASha256 -Path $selectedPath
    }
}

function Assert-P1BManifestTool {
    param([Parameter(Mandatory)]$Value, [Parameter(Mandatory)][string]$Name)
    Assert-P1AToolDescriptor -Value $Value -Name $Name
    if ([string]$Value.path -cnotmatch '^\$\{(?:REPO|OUTPUT_ROOT|TEMP|USERPROFILE|VS_INSTALL|VC_TOOLS|WINDOWS_KITS|WINDOWS|CARGO_HOME|RUSTUP_HOME|CUDA_TOOLKIT)\}(?:/[A-Za-z0-9][A-Za-z0-9._+() -]*)*$') {
        throw "$Name path is not tokenized"
    }
}

function Assert-P1BManifestComponent {
    param([Parameter(Mandatory)]$Value, [Parameter(Mandatory)][string]$Name)
    Assert-P1AClosedObject -Value $Value -Fields @('name', 'path', 'sha256', 'bytes') -Name $Name
    Assert-P1ASha256Value -Value ([string]$Value.sha256) -Name "$Name.sha256"
    if ([string]::IsNullOrWhiteSpace([string]$Value.name) -or
        [string]$Value.path -cnotmatch '^\$\{(?:REPO|OUTPUT_ROOT|TEMP|USERPROFILE|VS_INSTALL|VC_TOOLS|WINDOWS_KITS|WINDOWS|CARGO_HOME|RUSTUP_HOME|CUDA_TOOLKIT)\}(?:/[A-Za-z0-9][A-Za-z0-9._+() -]*)*$' -or
        [int64]$Value.bytes -lt 1) {
        throw "$Name is incomplete"
    }
}

function Assert-P1BEnvironmentManifest {
    param(
        [Parameter(Mandatory)]$Environment,
        [Parameter(Mandatory)][string]$RunId,
        [Parameter(Mandatory)][string]$RunRoot,
        [Parameter(Mandatory)]$Evidence,
        [Parameter(Mandatory)][Collections.Generic.HashSet[string]]$CommandIds
    )

    Assert-P1AClosedObject -Value $Environment -Fields @(
        'schema', 'phase_id', 'run_id', 'mode', 'status', 'p0', 'p1a', 'source',
        'host', 'rust', 'visual_studio', 'windows_sdk', 'cuda_toolkit',
        'runtime_linkage', 'optional_components', 'driver', 'gpu', 'architecture_targets',
        'device_probe', 'isolation', 'cleanup'
    ) -Name 'P1B CUDA environment manifest'
    if ([string]$Environment.schema -cne 'python-slm-cuda-environment-manifest-v1' -or
        [string]$Environment.phase_id -cne 'P1B' -or [string]$Environment.run_id -cne $RunId -or
        [string]$Environment.mode -cne 'Cuda' -or [string]$Environment.status -cne 'PASS') {
        throw 'P1B CUDA environment identity is invalid'
    }
    Assert-P1AClosedObject -Value $Environment.p0 -Fields @(
        'receipt_commit', 'receipt_sha256', 'contract_sha256', 'decision_ledger_sha256'
    ) -Name 'P1B environment P0 identity'
    foreach ($field in @('receipt_sha256', 'contract_sha256', 'decision_ledger_sha256')) {
        Assert-P1ASha256Value -Value ([string]$Environment.p0.$field) -Name "P1B P0 $field"
    }
    if (([string]$Environment.p0.receipt_commit) -cnotmatch '^[0-9a-f]{40}$') {
        throw 'P1B P0 commit is invalid'
    }
    foreach ($field in @('receipt_commit', 'receipt_sha256', 'contract_sha256',
            'decision_ledger_sha256')) {
        if ([string]$Environment.p0.$field -cne [string]$Evidence.p0_dependency.$field) {
            throw 'P1B environment and run evidence bind different P0 identities'
        }
    }
    $p1aFields = @(
        'status', 'pointer_path', 'pointer_sha256', 'acceptance_path', 'acceptance_sha256',
        'acceptance_sequence', 'run_path', 'run_evidence_sha256', 'seal_path', 'seal_sha256',
        'environment_path', 'environment_sha256', 'source_identity_sha256', 'verifier_sha256',
        'schema_bundle_sha256', 'review_closure_commit'
    )
    Assert-P1AClosedObject -Value $Environment.p1a -Fields $p1aFields -Name 'P1B environment P1A dependency'
    if ((($Environment.p1a | ConvertTo-Json -Depth 12 -Compress)) -cne
        (($Evidence.p1a_dependency | ConvertTo-Json -Depth 12 -Compress))) {
        throw 'P1B environment and run evidence bind different P1A dependencies'
    }
    Assert-P1AClosedObject -Value $Environment.source -Fields @(
        'head', 'dirty', 'input_manifest_sha256', 'cargo_lock_sha256',
        'verifier_sha256', 'schema_bundle_sha256'
    ) -Name 'P1B source identity'
    if ([string]$Environment.source.head -cnotmatch '^[0-9a-f]{40}$') {
        throw 'P1B source HEAD is invalid'
    }
    foreach ($field in @('input_manifest_sha256', 'cargo_lock_sha256', 'verifier_sha256', 'schema_bundle_sha256')) {
        Assert-P1ASha256Value -Value ([string]$Environment.source.$field) -Name "P1B source $field"
    }
    if ([string]$Environment.source.verifier_sha256 -cne [string]$Environment.p1a.verifier_sha256 -or
        [string]$Environment.source.schema_bundle_sha256 -cne [string]$Environment.p1a.schema_bundle_sha256) {
        throw 'P1B source bundles do not match the selected P1A regression run'
    }

    $repository = $RunRoot
    for ($index = 0; $index -lt 5; $index++) { $repository = Split-Path -Parent $repository }
    $p1aEnvironmentPath = [IO.Path]::GetFullPath((Join-Path $repository `
                ([string]$Environment.p1a.environment_path).Replace('/', '\')))
    $p1aRoot = Join-Path $repository 'docs\receipts\P1A'
    if (-not (Test-P1BContainedPathWithoutReparse -Path $p1aEnvironmentPath `
            -Root $p1aRoot -Leaf) -or
        (Get-P1ASha256 -Path $p1aEnvironmentPath) -cne [string]$Environment.p1a.environment_sha256) {
        throw 'P1B host/toolchain reconciliation could not validate the selected P1A environment'
    }
    $selectedP1AEnvironment = [IO.File]::ReadAllText($p1aEnvironmentPath, $script:Utf8NoBom) |
        ConvertFrom-Json
    foreach ($field in @('host', 'rust', 'visual_studio', 'windows_sdk')) {
        Assert-P1AClosedObject -Value $Environment.$field `
            -Fields @($selectedP1AEnvironment.$field.PSObject.Properties.Name) `
            -Name "P1B reconciled $field identity"
        if (($Environment.$field | ConvertTo-Json -Depth 20 -Compress) -cne
            ($selectedP1AEnvironment.$field | ConvertTo-Json -Depth 20 -Compress)) {
            throw "P1B $field identity differs from the selected P1A environment"
        }
    }

    Assert-P1AClosedObject -Value $Environment.cuda_toolkit -Fields @(
        'version', 'root', 'tools', 'headers', 'libraries', 'runtime_dlls'
    ) -Name 'CUDA toolkit identity'
    if ([string]$Environment.cuda_toolkit.version -cnotmatch '^[0-9]+\.[0-9]+(?:\.[0-9]+){0,2}$' -or
        -not (Test-P1AVersionAtLeast -Actual ([string]$Environment.cuda_toolkit.version) -Minimum '12.8') -or
        [string]$Environment.cuda_toolkit.root -cne '${CUDA_TOOLKIT}') {
        throw 'CUDA toolkit version or root is invalid'
    }
    $toolNames = @($Environment.cuda_toolkit.tools | ForEach-Object {
            Assert-P1BManifestTool -Value $_ -Name 'CUDA toolkit tool'; [string]$_.name
        })
    if (($toolNames -join [char]0) -cne ((@('nvcc', 'ptxas', 'fatbinary', 'nvlink', 'cuobjdump')) -join [char]0)) {
        throw 'CUDA toolkit tool identities are incomplete or out of order'
    }
    foreach ($set in @(
            [pscustomobject]@{ Value = $Environment.cuda_toolkit.headers; Names = @('cuda.h', 'cuda_runtime.h', 'cublas_v2.h', 'cublasLt.h'); Label = 'CUDA header' },
            [pscustomobject]@{ Value = $Environment.cuda_toolkit.libraries; Names = @('cuda.lib', 'cudart.lib', 'cublas.lib', 'cublasLt.lib'); Label = 'CUDA library' }
        )) {
        $names = @($set.Value | ForEach-Object {
                Assert-P1BManifestComponent -Value $_ -Name $set.Label; [string]$_.name
            })
        if (($names -join [char]0) -cne (@($set.Names) -join [char]0)) {
            throw "$($set.Label) identities are incomplete or out of order"
        }
    }
    $runtimeNames = @($Environment.cuda_toolkit.runtime_dlls | ForEach-Object {
            Assert-P1BManifestComponent -Value $_ -Name 'CUDA runtime DLL'; [string]$_.name
        })
    if ($runtimeNames.Count -ne 3 -or $runtimeNames[0] -cnotmatch '^cudart64_[0-9]+\.dll$' -or
        $runtimeNames[1] -cnotmatch '^cublas64_[0-9]+\.dll$' -or
        $runtimeNames[2] -cnotmatch '^cublasLt64_[0-9]+\.dll$') {
        throw 'CUDA runtime DLL identities are incomplete or out of order'
    }
    $toolkitVersion = [version][string]$Environment.cuda_toolkit.version
    $expectedLinkage = if ($toolkitVersion.Major -ge 13) { 'hybrid' } else { 'shared' }
    $expectedProvider = if ($expectedLinkage -ceq 'hybrid') { 'display_driver' } else { 'toolkit_cudart' }
    $expectedStagedComponents = if ($expectedLinkage -ceq 'hybrid') {
        @('cublas', 'cublaslt')
    }
    else { @('cudart', 'cublas', 'cublaslt') }
    Assert-P1AClosedObject -Value $Environment.runtime_linkage -Fields @(
        'mode', 'active_provider', 'staged_runtime_dlls'
    ) -Name 'CUDA runtime linkage identity'
    if ([string]$Environment.runtime_linkage.mode -cne $expectedLinkage -or
        [string]$Environment.runtime_linkage.active_provider -cne $expectedProvider) {
        throw 'CUDA runtime linkage mode or provider does not match the toolkit major version'
    }
    $runtimeDescriptorByComponent = @{
        cudart = $Environment.cuda_toolkit.runtime_dlls[0]
        cublas = $Environment.cuda_toolkit.runtime_dlls[1]
        cublaslt = $Environment.cuda_toolkit.runtime_dlls[2]
    }
    $stagedComponents = @($Environment.runtime_linkage.staged_runtime_dlls | ForEach-Object {
            Assert-P1AClosedObject -Value $_ -Fields @('component', 'sha256') `
                -Name 'staged CUDA runtime DLL'
            $component = [string]$_.component
            if (-not $runtimeDescriptorByComponent.ContainsKey($component) -or
                [string]$_.sha256 -cne [string]$runtimeDescriptorByComponent[$component].sha256) {
                throw 'staged CUDA runtime DLL does not match the selected toolkit identity'
            }
            $component
        })
    if (($stagedComponents -join [char]0) -cne ($expectedStagedComponents -join [char]0)) {
        throw 'staged CUDA runtime DLL components are incomplete or out of order'
    }

    Assert-P1AClosedObject -Value $Environment.optional_components -Fields @(
        'cudnn', 'nvrtc', 'nvjitlink', 'compute_sanitizer', 'curand'
    ) -Name 'optional CUDA inventory'
    foreach ($name in @('cudnn', 'nvrtc', 'nvjitlink', 'compute_sanitizer', 'curand')) {
        $component = $Environment.optional_components.$name
        Assert-P1AClosedObject -Value $component -Fields @('present', 'version', 'path', 'sha256') `
            -Name "optional CUDA component $name"
        if ([bool]$component.present) {
            if ([string]$component.path -cnotmatch '^\$\{(?:REPO|OUTPUT_ROOT|TEMP|USERPROFILE|VS_INSTALL|VC_TOOLS|WINDOWS_KITS|WINDOWS|CARGO_HOME|RUSTUP_HOME|CUDA_TOOLKIT)\}/' ) {
                throw "optional CUDA component $name has invalid present identity"
            }
            Assert-P1ASha256Value -Value ([string]$component.sha256) -Name "optional CUDA component $name hash"
        }
        elseif ($null -ne $component.version -or $null -ne $component.path -or $null -ne $component.sha256) {
            throw "optional CUDA component $name has data while absent"
        }
    }

    Assert-P1AClosedObject -Value $Environment.driver -Fields @('driver_version', 'cuda_umd_version', 'nvidia_smi', 'library') `
        -Name 'CUDA driver identity'
    if ([string]$Environment.driver.driver_version -cnotmatch '^[0-9]+\.[0-9]+(?:\.[0-9]+){0,2}$' -or
        [string]::IsNullOrWhiteSpace([string]$Environment.driver.cuda_umd_version)) {
        throw 'CUDA driver version identity is invalid'
    }
    Assert-P1BManifestTool -Value $Environment.driver.nvidia_smi -Name 'nvidia-smi tool'
    Assert-P1BManifestTool -Value $Environment.driver.library -Name 'CUDA driver library'
    if ([string]$Environment.driver.nvidia_smi.name -cne 'nvidia-smi' -or
        [string]$Environment.driver.library.name -cne 'nvcuda') {
        throw 'CUDA driver library identity is invalid'
    }
    Assert-P1AClosedObject -Value $Environment.gpu -Fields @(
        'index', 'name', 'memory_total_bytes', 'compute_capability_major', 'compute_capability_minor'
    ) -Name 'qualified GPU identity'
    if ([int]$Environment.gpu.index -lt 0 -or
        [string]$Environment.gpu.name -cne 'NVIDIA GeForce RTX 5090' -or
        [int64]$Environment.gpu.memory_total_bytes -lt 1 -or
        [int]$Environment.gpu.compute_capability_major -ne 12 -or
        [int]$Environment.gpu.compute_capability_minor -ne 0) {
        throw 'qualified GPU identity is invalid'
    }

    Assert-P1AClosedObject -Value $Environment.architecture_targets -Fields @(
        'sass', 'ptx', 'advertised_architectures', 'advertised_code', 'mixed', 'ptx_only'
    ) -Name 'CUDA architecture targets'
    if ((@($Environment.architecture_targets.sass) -join ',') -cne 'sm_120' -or
        (@($Environment.architecture_targets.ptx) -join ',') -cne 'compute_120' -or
        @($Environment.architecture_targets.advertised_architectures) -notcontains 'compute_120' -or
        @($Environment.architecture_targets.advertised_code) -notcontains 'sm_120') {
        throw 'CUDA architecture target inventory is incomplete'
    }
    foreach ($variantName in @('mixed', 'ptx_only')) {
        $variant = $Environment.architecture_targets.$variantName
        Assert-P1AClosedObject -Value $variant -Fields @(
            'compile_command_id', 'executable_sha256', 'run_command_id',
            'pe_headers_command_id', 'pe_dependents_command_id', 'elf_command_id',
            'ptx_list_command_id', 'sass_dump_command_id', 'ptx_dump_command_id',
            'embedded_sass', 'embedded_ptx', 'pe_imports', 'result'
        ) -Name "CUDA $variantName artifact inspection"
        foreach ($idField in @('compile_command_id', 'run_command_id',
                'pe_headers_command_id', 'pe_dependents_command_id', 'elf_command_id',
                'ptx_list_command_id', 'sass_dump_command_id', 'ptx_dump_command_id')) {
            if (-not $CommandIds.Contains([string]$variant.$idField)) {
                throw "CUDA $variantName inspection references an unknown command ID"
            }
        }
        Assert-P1ASha256Value -Value ([string]$variant.executable_sha256) -Name "CUDA $variantName executable hash"
        if ([string]$variant.result -cne 'PASS' -or
            (@($variant.embedded_ptx) -join ',') -cne 'compute_120' -or
            ($variantName -ceq 'mixed' -and (@($variant.embedded_sass) -join ',') -cne 'sm_120') -or
            ($variantName -ceq 'ptx_only' -and @($variant.embedded_sass).Count -ne 0)) {
            throw "CUDA $variantName artifact inspection did not pass the target contract"
        }
        $variantImports = @($variant.pe_imports | ForEach-Object { ([string]$_).ToLowerInvariant() })
        if (@($variantImports | Sort-Object -Unique).Count -ne $variantImports.Count) {
            throw "CUDA $variantName PE import boundary contains duplicates"
        }
        foreach ($pattern in @('^cublas64_[0-9]+\.dll$', '^cublaslt64_[0-9]+\.dll$')) {
            if (@($variantImports | Where-Object { $_ -match $pattern }).Count -ne 1) {
                throw "CUDA $variantName PE import boundary is incomplete or ambiguous"
            }
        }
        if ($expectedLinkage -ceq 'shared' -and
            @($variantImports | Where-Object { $_ -match '^cudart64_[0-9]+\.dll$' }).Count -ne 1) {
            throw "CUDA $variantName shared runtime import is incomplete or ambiguous"
        }
        $allowedImportPatterns = @(
            '^nvcuda\.dll$', '^cudart64_[0-9]+\.dll$', '^cublas64_[0-9]+\.dll$',
            '^cublaslt64_[0-9]+\.dll$', '^kernel32\.dll$', '^ntdll\.dll$',
            '^ucrtbase\.dll$', '^vcruntime140(?:_1)?\.dll$', '^msvcp140(?:_[0-9]+)?\.dll$',
            '^concrt140\.dll$', '^api-ms-win-[a-z0-9_-]+\.dll$'
        )
        foreach ($import in $variantImports) {
            if (@($allowedImportPatterns | Where-Object { $import -match $_ }).Count -eq 0) {
                throw "CUDA $variantName PE import boundary contains an unapproved DLL"
            }
        }
        $compileRecord = @($Evidence.commands | Where-Object {
                [string]$_.id -ceq [string]$variant.compile_command_id
            })
        if ($compileRecord.Count -ne 1) {
            throw "CUDA $variantName compile command record is missing or ambiguous"
        }
        $expectedRuntimeArgument = '--cudart=' + $expectedLinkage
        $expectedLinkerArgument = if ($expectedLinkage -ceq 'hybrid') {
            '--linker-options=/WX,/NODEFAULTLIB:LIBCMT'
        }
        else { '--linker-options=/WX' }
        $linkerArguments = @($compileRecord[0].argv | Where-Object {
                $_ -like '--linker-options=*' -or $_ -match '(?i)(?:^|[,])/NODEFAULTLIB:'
            })
        if (@($compileRecord[0].argv) -notcontains $expectedRuntimeArgument -or
            $linkerArguments.Count -ne 1 -or
            [string]$linkerArguments[0] -cne $expectedLinkerArgument -or
            @($compileRecord[0].argv | Where-Object { $_ -match '(?i)[\\/]cudart\.lib$' }).Count -ne 0) {
            throw "CUDA $variantName compile command does not match the runtime linkage contract"
        }
    }

    Assert-P1AClosedObject -Value $Environment.device_probe -Fields @(
        'source_sha256', 'runtime_version',
        'driver_version', 'cublas_version', 'cublaslt_version', 'mixed_result',
        'ptx_only_result', 'sentinel', 'target_match_count'
    ) -Name 'CUDA device probe evidence'
    Assert-P1ASha256Value -Value ([string]$Environment.device_probe.source_sha256) -Name 'CUDA probe source hash'
    if ([int]$Environment.device_probe.runtime_version -lt 12080 -or
        [int]$Environment.device_probe.driver_version -lt [int]$Environment.device_probe.runtime_version -or
        [int]$Environment.device_probe.cublas_version -lt 1 -or
        [int64]$Environment.device_probe.cublaslt_version -lt 1 -or
        [string]$Environment.device_probe.mixed_result -cne 'PASS' -or
        [string]$Environment.device_probe.ptx_only_result -cne 'PASS' -or
        [int]$Environment.device_probe.sentinel -ne 42 -or
        [int]$Environment.device_probe.target_match_count -ne 1) {
        throw 'CUDA device probe values do not satisfy the qualification contract'
    }
    Assert-P1AClosedObject -Value $Environment.isolation -Fields @(
        'temporary_root_absent_before', 'temporary_root_outside_repository',
        'python_canaries', 'canary_hits', 'python_invoked'
    ) -Name 'P1B isolation evidence'
    if (-not [bool]$Environment.isolation.temporary_root_absent_before -or
        -not [bool]$Environment.isolation.temporary_root_outside_repository -or
        @($Environment.isolation.python_canaries).Count -lt 1 -or
        @($Environment.isolation.canary_hits).Count -ne 0 -or [bool]$Environment.isolation.python_invoked) {
        throw 'P1B Python or temporary-root isolation did not pass'
    }
    Assert-P1AClosedObject -Value $Environment.cleanup -Fields @(
        'temporary_root_removed', 'repository_target_unchanged',
        'parent_environment_unchanged', 'inputs_unchanged'
    ) -Name 'P1B manifest cleanup'
    foreach ($field in @('temporary_root_removed', 'repository_target_unchanged',
            'parent_environment_unchanged', 'inputs_unchanged')) {
        if (-not [bool]$Environment.cleanup.$field) { throw "P1B cleanup assertion is false: $field" }
    }
}

function Assert-P1BPassRun {
    param(
        [Parameter(Mandatory)][string]$RunRoot,
        [Parameter(Mandatory)][string]$RunId,
        [Parameter(Mandatory)][string]$SourceIdentitySha256
    )

    if ($RunId -cnotmatch '^[0-9]{8}T[0-9]{9}Z-[0-9a-f]{24}$' -or
        (Split-Path -Leaf ([IO.Path]::GetFullPath($RunRoot))) -cne $RunId -or
        -not (Test-P1ASeal -RunRoot $RunRoot)) {
        throw 'P1B run identity or seal is invalid'
    }
    Assert-P1ASha256Value -Value $SourceIdentitySha256 -Name 'P1B source identity hash'
    $evidencePath = Resolve-P1ARunFile -RunRoot $RunRoot -RelativePath 'evidence.json' -Name 'P1B evidence'
    $environmentPath = Resolve-P1ARunFile -RunRoot $RunRoot -RelativePath 'artifacts/environment.json' -Name 'P1B environment'
    $sourcePath = Resolve-P1ARunFile -RunRoot $RunRoot -RelativePath 'artifacts/source-identity.json' -Name 'P1B source identity'
    $sealPath = Resolve-P1ARunFile -RunRoot $RunRoot -RelativePath 'SHA256SUMS' -Name 'P1B seal'
    if ((Get-P1ASha256 -Path $sourcePath) -cne $SourceIdentitySha256) {
        throw 'P1B source identity hash does not match its run artifact'
    }
    $evidenceText = [IO.File]::ReadAllText($evidencePath, $script:Utf8NoBom)
    $evidence = $evidenceText | ConvertFrom-Json
    Assert-P1AClosedObject -Value $evidence -Fields @(
        'schema', 'phase_id', 'run_id', 'mode', 'status', 'started_at', 'finished_at',
        'duration_ms', 'invocation', 'source_identity_sha256', 'p0_dependency',
        'p1a_dependency', 'environment', 'commands', 'gates', 'errors', 'cleanup', 'seal'
    ) -Name 'P1B run evidence'
    if ([string]$evidence.schema -cne 'python-slm-phase-evidence-v2' -or
        [string]$evidence.phase_id -cne 'P1B' -or [string]$evidence.run_id -cne $RunId -or
        [string]$evidence.mode -cne 'Cuda' -or [string]$evidence.status -cne 'PASS' -or
        [string]$evidence.source_identity_sha256 -cne $SourceIdentitySha256 -or
        [int64]$evidence.duration_ms -lt 0) {
        throw 'P1B run evidence identity is invalid'
    }
    $parsedTimestamp = [DateTime]::MinValue
    foreach ($timestamp in @([string]$evidence.started_at, [string]$evidence.finished_at)) {
        if (-not $timestamp.EndsWith('Z', [StringComparison]::Ordinal) -or
            -not [DateTime]::TryParse($timestamp, [ref]$parsedTimestamp)) {
            throw 'P1B run evidence timestamp is invalid or not UTC'
        }
    }
    Assert-P1AClosedObject -Value $evidence.invocation -Fields @('argv', 'cwd') -Name 'P1B invocation'
    $expectedInvocation = @('powershell', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
        'scripts/verify-env.ps1', '-Mode', 'Cuda', '-OutputRoot', 'docs/receipts/P1B')
    if ((@($evidence.invocation.argv) -join [char]0) -cne ($expectedInvocation -join [char]0) -or
        [string]$evidence.invocation.cwd -cne '${REPO}') {
        throw 'P1B invocation is not the exact approved command'
    }
    Assert-P1AClosedObject -Value $evidence.p0_dependency -Fields @(
        'status', 'receipt_commit', 'receipt_sha256', 'contract_sha256', 'decision_ledger_sha256'
    ) -Name 'P1B P0 dependency'
    if ([string]$evidence.p0_dependency.status -cne 'PASS') { throw 'P1B P0 dependency is not PASS' }
    $p1aFields = @('status', 'pointer_path', 'pointer_sha256', 'acceptance_path',
        'acceptance_sha256', 'acceptance_sequence', 'run_path', 'run_evidence_sha256',
        'seal_path', 'seal_sha256', 'environment_path', 'environment_sha256',
        'source_identity_sha256', 'verifier_sha256', 'schema_bundle_sha256',
        'review_closure_commit')
    Assert-P1AClosedObject -Value $evidence.p1a_dependency -Fields $p1aFields -Name 'P1B P1A dependency'
    if ([string]$evidence.p1a_dependency.status -cne 'PASS' -or
        [string]$evidence.p1a_dependency.pointer_path -cne 'docs/receipts/P1A/evidence.json' -or
        [string]$evidence.p1a_dependency.review_closure_commit -cne '9359c989fa63d4a300abc509e735b7e81a24a2ea' -or
        [int]$evidence.p1a_dependency.acceptance_sequence -lt 1) {
        throw 'P1B P1A dependency is incomplete or not reviewed'
    }
    foreach ($field in @('pointer_sha256', 'acceptance_sha256', 'run_evidence_sha256',
            'seal_sha256', 'environment_sha256', 'source_identity_sha256',
            'verifier_sha256', 'schema_bundle_sha256')) {
        Assert-P1ASha256Value -Value ([string]$evidence.p1a_dependency.$field) -Name "P1A dependency $field"
    }

    $null = Assert-P1AFileReference -Reference $evidence.environment -RunRoot $RunRoot -Name 'P1B environment reference'
    if ([string]$evidence.environment.path -cne 'artifacts/environment.json') {
        throw 'P1B environment path is not canonical'
    }
    $commandIds = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $expectedNumber = 1
    foreach ($command in @($evidence.commands)) {
        Assert-P1AClosedObject -Value $command -Fields @(
            'id', 'argv', 'cwd', 'environment_delta_names', 'configuration_sha256',
            'exit_code', 'duration_ms', 'status', 'stdout', 'stderr'
        ) -Name 'P1B command record'
        $expectedId = 'C' + $expectedNumber.ToString('00')
        if ([string]$command.id -cne $expectedId -or -not $commandIds.Add([string]$command.id) -or
            [string]$command.status -cne 'PASS' -or [int]$command.exit_code -ne 0 -or
            [string]$command.cwd -cne '${REPO}' -or @($command.argv).Count -lt 1) {
            throw 'P1B command sequence contains a failed, duplicate, or noncanonical record'
        }
        Assert-P1ASha256Value -Value ([string]$command.configuration_sha256) -Name 'P1B command configuration hash'
        $null = Assert-P1AFileReference -Reference $command.stdout -RunRoot $RunRoot -Name 'P1B command stdout'
        $null = Assert-P1AFileReference -Reference $command.stderr -RunRoot $RunRoot -Name 'P1B command stderr'
        if ([string]$command.stdout.path -cne "commands/$expectedId.stdout.txt" -or
            [string]$command.stderr.path -cne "commands/$expectedId.stderr.txt") {
            throw 'P1B command transcript path is not canonical'
        }
        $expectedNumber++
    }
    if ($commandIds.Count -lt 1) { throw 'P1B PASS run contains no commands' }
    $gateNames = @('p0', 'p1a', 'cpu_regression', 'input_stability', 'toolchain',
        'cuda_toolkit', 'architecture_targets', 'gpu_identity', 'driver_runtime',
        'device_probe', 'python_isolation', 'redaction', 'cleanup')
    Assert-P1AClosedObject -Value $evidence.gates -Fields $gateNames -Name 'P1B gates'
    foreach ($gate in $gateNames) {
        Assert-P1AClosedObject -Value $evidence.gates.$gate -Fields @('status', 'detail') -Name "P1B gate $gate"
        if ([string]$evidence.gates.$gate.status -cne 'PASS' -or
            [string]::IsNullOrWhiteSpace([string]$evidence.gates.$gate.detail)) {
            throw "P1B gate did not pass: $gate"
        }
    }
    if ($null -eq $evidence.errors) {
        if ($evidenceText -notmatch '"errors"\s*:\s*\[\s*\]') {
            throw 'P1B PASS run errors field is not an empty array'
        }
    }
    elseif (@($evidence.errors | Where-Object { $null -ne $_ }).Count -ne 0) {
        throw 'P1B PASS run contains errors'
    }
    Assert-P1AClosedObject -Value $evidence.cleanup -Fields @('attempted', 'temporary_root_removed') `
        -Name 'P1B cleanup'
    if (-not [bool]$evidence.cleanup.attempted -or -not [bool]$evidence.cleanup.temporary_root_removed) {
        throw 'P1B cleanup did not complete'
    }
    Assert-P1AClosedObject -Value $evidence.seal -Fields @('path', 'entries', 'coverage_rule') -Name 'P1B seal reference'
    $sealLines = @([IO.File]::ReadAllLines($sealPath, $script:Utf8NoBom))
    if ([string]$evidence.seal.path -cne 'SHA256SUMS' -or
        [string]$evidence.seal.coverage_rule -cne 'all_run_files_except_seal' -or
        [int]$evidence.seal.entries -ne $sealLines.Count) {
        throw 'P1B seal reference does not match the immutable run'
    }
    $environment = [IO.File]::ReadAllText($environmentPath, $script:Utf8NoBom) | ConvertFrom-Json
    $null = Assert-P1BEnvironmentManifest -Environment $environment -RunId $RunId `
        -RunRoot $RunRoot -Evidence $evidence -CommandIds $commandIds
    return [pscustomobject][ordered]@{
        evidence_path = $evidencePath
        evidence_sha256 = Get-P1ASha256 -Path $evidencePath
        environment_path = $environmentPath
        environment_sha256 = Get-P1ASha256 -Path $environmentPath
        seal_path = $sealPath
        seal_sha256 = Get-P1ASha256 -Path $sealPath
    }
}

function Assert-P1BAcceptanceObject {
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
    ) -Name 'P1B acceptance record'
    if ([string]$Acceptance.schema -cne 'python-slm-phase-acceptance-v2' -or
        [string]$Acceptance.phase_id -cne 'P1B' -or [int]$Acceptance.sequence -ne $Sequence -or
        [string]$Acceptance.status -cne 'PASS' -or
        [string]$Acceptance.acceptance_kind -cne 'automatic_machine_qualification' -or
        @($Acceptance.required_approvals).Count -ne 0 -or
        [string]$Acceptance.previous_acceptance_sha256 -cne [string]$PreviousHash -or
        [string]$Acceptance.run_path -cnotmatch '^runs/(?<run>[0-9]{8}T[0-9]{9}Z-[0-9a-f]{24})$') {
        throw 'P1B acceptance record does not match the automatic qualification chain'
    }
    $parsedTimestamp = [DateTime]::MinValue
    if (-not ([string]$Acceptance.created_at).EndsWith('Z', [StringComparison]::Ordinal) -or
        -not [DateTime]::TryParse([string]$Acceptance.created_at, [ref]$parsedTimestamp)) {
        throw 'P1B acceptance timestamp is invalid or not UTC'
    }
    $runId = $Matches['run']
    $validated = Assert-P1BPassRun -RunRoot (Join-Path (Join-Path $OutputRoot 'runs') $runId) `
        -RunId $runId -SourceIdentitySha256 ([string]$Acceptance.source_identity_sha256)
    if ([string]$Acceptance.run_evidence_sha256 -cne $validated.evidence_sha256 -or
        [string]$Acceptance.seal_sha256 -cne $validated.seal_sha256 -or
        [string]$Acceptance.environment_sha256 -cne $validated.environment_sha256 -or
        [string]$Acceptance.seal_path -cne "runs/$runId/SHA256SUMS" -or
        [string]$Acceptance.environment_path -cne "runs/$runId/artifacts/environment.json") {
        throw 'P1B acceptance hashes or paths do not match its run'
    }
}

function Publish-P1BAcceptance {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$OutputRoot,
        [Parameter(Mandatory)][string]$RunId,
        [Parameter(Mandatory)][string]$SourceIdentitySha256
    )
    $root = [IO.Path]::GetFullPath($OutputRoot).TrimEnd('\', '/')
    if ($RunId -cnotmatch '^[0-9]{8}T[0-9]{9}Z-[0-9a-f]{24}$') { throw 'P1B acceptance run ID is invalid' }
    $runRoot = [IO.Path]::GetFullPath((Join-Path (Join-Path $root 'runs') $RunId))
    $validatedRun = Assert-P1BPassRun -RunRoot $runRoot -RunId $RunId `
        -SourceIdentitySha256 $SourceIdentitySha256
    $mutexNameHash = Get-P1ATextSha256 -Text $root.ToLowerInvariant()
    $mutex = [Threading.Mutex]::new($false, "Local\python-slm-p1b-$mutexNameHash")
    $locked = $false
    try {
        $locked = $mutex.WaitOne([TimeSpan]::FromSeconds(30))
        if (-not $locked) { throw 'timed out waiting for the P1B publication lock' }
        $validatedRun = Assert-P1BPassRun -RunRoot $runRoot -RunId $RunId `
            -SourceIdentitySha256 $SourceIdentitySha256
        $acceptanceRoot = Join-Path $root 'acceptances'
        [void][IO.Directory]::CreateDirectory($acceptanceRoot)
        if (((Get-Item -Force -LiteralPath $acceptanceRoot).Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw 'P1B acceptance directory is a reparse point'
        }
        $files = @(Get-ChildItem -LiteralPath $acceptanceRoot -File -ErrorAction Stop | Sort-Object Name)
        $previousHash = $null
        $lastAcceptance = $null
        foreach ($index in 0..($files.Count - 1)) {
            if ($files.Count -eq 0) { break }
            $sequence = $index + 1
            if ($files[$index].Name -cne ($sequence.ToString('00000000') + '.json')) {
                throw 'P1B acceptance sequence contains a gap or noncanonical file'
            }
            $prior = [IO.File]::ReadAllText($files[$index].FullName, $script:Utf8NoBom) | ConvertFrom-Json
            Assert-P1BAcceptanceObject -Acceptance $prior -Sequence $sequence `
                -PreviousHash $previousHash -OutputRoot $root
            $previousHash = Get-P1ASha256 -Path $files[$index].FullName
            $lastAcceptance = $prior
        }
        $pointerPath = Join-Path $root 'evidence.json'
        $oldPointer = $null
        if (Test-Path -LiteralPath $pointerPath -PathType Leaf) {
            $oldPointer = [IO.File]::ReadAllText($pointerPath, $script:Utf8NoBom) | ConvertFrom-Json
            Assert-P1AClosedObject -Value $oldPointer -Fields @(
                'schema', 'phase_id', 'acceptance_path', 'acceptance_sha256', 'updated_at'
            ) -Name 'P1B selected pointer'
            if ([string]$oldPointer.schema -cne 'python-slm-phase-evidence-pointer-v2' -or
                [string]$oldPointer.phase_id -cne 'P1B' -or
                [string]$oldPointer.acceptance_path -cnotmatch '^acceptances/(?<name>[0-9]{8}\.json)$') {
                throw 'P1B selected pointer is invalid'
            }
            $parsedPointerTimestamp = [DateTime]::MinValue
            if (-not ([string]$oldPointer.updated_at).EndsWith('Z', [StringComparison]::Ordinal) -or
                -not [DateTime]::TryParse([string]$oldPointer.updated_at, [ref]$parsedPointerTimestamp)) {
                throw 'P1B selected pointer timestamp is invalid or not UTC'
            }
            $oldSelected = Join-Path $acceptanceRoot $Matches['name']
            if (-not (Test-Path -LiteralPath $oldSelected -PathType Leaf) -or
                (Get-P1ASha256 -Path $oldSelected) -cne [string]$oldPointer.acceptance_sha256) {
                throw 'P1B selected pointer hash does not match its acceptance'
            }
        }
        $recoverOrphan = $null -ne $lastAcceptance -and
            [string]$lastAcceptance.run_path -ceq "runs/$RunId" -and
            [string]$lastAcceptance.source_identity_sha256 -ceq $SourceIdentitySha256 -and
            ($null -eq $oldPointer -or
                [string]$oldPointer.acceptance_sha256 -cne $previousHash)
        if ($recoverOrphan) {
            $sequence = [int]$lastAcceptance.sequence
            $acceptanceRelative = 'acceptances/' + $sequence.ToString('00000000') + '.json'
            $acceptanceHash = $previousHash
        }
        else {
            $sequence = $files.Count + 1
            $acceptanceRelative = 'acceptances/' + $sequence.ToString('00000000') + '.json'
            $acceptancePath = Join-Path $root $acceptanceRelative.Replace('/', '\')
            $acceptance = [ordered]@{
                schema = 'python-slm-phase-acceptance-v2'; phase_id = 'P1B'; sequence = $sequence
                status = 'PASS'; acceptance_kind = 'automatic_machine_qualification'; required_approvals = @()
                run_path = "runs/$RunId"; run_evidence_sha256 = $validatedRun.evidence_sha256
                seal_path = "runs/$RunId/SHA256SUMS"; seal_sha256 = $validatedRun.seal_sha256
                environment_path = "runs/$RunId/artifacts/environment.json"
                environment_sha256 = $validatedRun.environment_sha256
                source_identity_sha256 = $SourceIdentitySha256
                previous_acceptance_sha256 = $previousHash; created_at = [DateTime]::UtcNow.ToString('o')
            }
            Write-P1AJsonFile -Path $acceptancePath -Value $acceptance -CreateNew
            $acceptanceHash = Get-P1ASha256 -Path $acceptancePath
            Assert-P1BAcceptanceObject -Acceptance $acceptance -Sequence $sequence `
                -PreviousHash $previousHash -OutputRoot $root
        }
        $pointer = [ordered]@{
            schema = 'python-slm-phase-evidence-pointer-v2'; phase_id = 'P1B'
            acceptance_path = $acceptanceRelative; acceptance_sha256 = $acceptanceHash
            updated_at = [DateTime]::UtcNow.ToString('o')
        }
        $temporaryPointer = Join-Path $root ('.evidence.' + $RunId + '.tmp')
        Write-P1AJsonFile -Path $temporaryPointer -Value $pointer -CreateNew
        $backupPointer = $null
        $hadPointer = Test-Path -LiteralPath $pointerPath -PathType Leaf
        $replacementCompleted = $false
        try {
            if ($hadPointer) {
                $backupPointer = Join-Path $root ('.evidence.' + $RunId + '.bak')
                [IO.File]::Replace($temporaryPointer, $pointerPath, $backupPointer, $true)
            }
            else { [IO.File]::Move($temporaryPointer, $pointerPath) }
            $replacementCompleted = $true
            $verified = [IO.File]::ReadAllText($pointerPath, $script:Utf8NoBom) | ConvertFrom-Json
            if ([string]$verified.schema -cne 'python-slm-phase-evidence-pointer-v2' -or
                [string]$verified.phase_id -cne 'P1B' -or
                [string]$verified.acceptance_path -cne $acceptanceRelative -or
                [string]$verified.acceptance_sha256 -cne $acceptanceHash) {
                throw 'published P1B pointer failed verification'
            }
            $parsedPointerTimestamp = [DateTime]::MinValue
            if (-not ([string]$verified.updated_at).EndsWith('Z', [StringComparison]::Ordinal) -or
                -not [DateTime]::TryParse([string]$verified.updated_at, [ref]$parsedPointerTimestamp)) {
                throw 'published P1B pointer timestamp is invalid or not UTC'
            }
            $selectedPath = Join-Path $root $acceptanceRelative.Replace('/', '\')
            $selected = [IO.File]::ReadAllText($selectedPath, $script:Utf8NoBom) | ConvertFrom-Json
            $selectedPrevious = if ($sequence -eq 1) { $null } else {
                Get-P1ASha256 -Path (Join-Path $acceptanceRoot (($sequence - 1).ToString('00000000') + '.json'))
            }
            Assert-P1BAcceptanceObject -Acceptance $selected -Sequence $sequence `
                -PreviousHash $selectedPrevious -OutputRoot $root
            if ($null -ne $backupPointer -and (Test-Path -LiteralPath $backupPointer)) {
                Remove-Item -LiteralPath $backupPointer -Force -ErrorAction Stop
            }
        }
        catch {
            if ($null -ne $backupPointer -and (Test-Path -LiteralPath $backupPointer)) {
                $failed = Join-Path $root ('.evidence.' + $RunId + '.failed')
                [IO.File]::Replace($backupPointer, $pointerPath, $failed, $true)
                if (Test-Path -LiteralPath $failed) { Remove-Item -LiteralPath $failed -Force }
            }
            elseif (-not $hadPointer -and $replacementCompleted -and
                (Test-Path -LiteralPath $pointerPath)) {
                Remove-Item -LiteralPath $pointerPath -Force
            }
            if (Test-Path -LiteralPath $temporaryPointer) { Remove-Item -LiteralPath $temporaryPointer -Force }
            throw
        }
        return [pscustomobject][ordered]@{
            sequence = $sequence; acceptance_path = $acceptanceRelative
            acceptance_sha256 = $acceptanceHash; pointer_path = 'evidence.json'
        }
    }
    finally {
        if ($locked) { try { [void]$mutex.ReleaseMutex() } catch { } }
        try { $mutex.Dispose() } catch { }
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
    'ConvertFrom-P1BNvccVersion',
    'Get-P1BNvccTargets',
    'ConvertFrom-P1BNvidiaSmi',
    'Assert-P1BProbeResult',
    'Select-P1BCudaToolkit',
    'Get-P1BSelectedP1ADependency',
    'Select-P1AVs2022Instance',
    'Protect-P1AText',
    'Invoke-P1AProcess',
    'New-P1ASeal',
    'Test-P1ASeal',
    'Publish-P1AAcceptance',
    'Publish-P1BAcceptance',
    'Invoke-P1AVerification'
)

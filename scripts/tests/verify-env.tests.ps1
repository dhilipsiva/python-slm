[CmdletBinding()]
param(
    [string]$ModulePath,
    [string]$EntrypointPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($ModulePath)) {
    $ModulePath = Join-Path $PSScriptRoot '..\lib\VerifyEnv.psm1'
}
if ([string]::IsNullOrWhiteSpace($EntrypointPath)) {
    $EntrypointPath = Join-Path $PSScriptRoot '..\verify-env.ps1'
}

$script:Passed = 0
$script:Failed = 0
$script:Failures = [Collections.Generic.List[string]]::new()

function Assert-P1ATest {
    param(
        [Parameter(Mandatory)][bool]$Condition,
        [Parameter(Mandatory)][string]$Message
    )

    if (-not $Condition) {
        throw $Message
    }
}

function Assert-P1AEqual {
    param(
        [AllowNull()]$Actual,
        [AllowNull()]$Expected,
        [Parameter(Mandatory)][string]$Message
    )

    if ($Actual -cne $Expected) {
        throw "$Message; expected=<$Expected> actual=<$Actual>"
    }
}

function Assert-P1AThrows {
    param(
        [Parameter(Mandatory)][scriptblock]$Script,
        [string]$Pattern = '.'
    )

    $caught = $null
    try {
        & $Script
    }
    catch {
        $caught = $_
    }
    if ($null -eq $caught) {
        throw 'expected an exception, but the operation succeeded'
    }
    if ($caught.Exception.Message -notmatch $Pattern) {
        throw "exception did not match /$Pattern/: $($caught.Exception.Message)"
    }
}

function Invoke-P1ATest {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][scriptblock]$Script
    )

    try {
        & $Script
        $script:Passed++
        Write-Output "PASS $Name"
    }
    catch {
        $script:Failed++
        $message = "$Name`: $($_.Exception.Message)"
        $script:Failures.Add($message)
        Write-Output "FAIL $message"
    }
}

function Get-P1ATestProperty {
    param(
        [Parameter(Mandatory)]$Value,
        [Parameter(Mandatory)][string[]]$Names
    )

    foreach ($name in $Names) {
        $property = $Value.PSObject.Properties[$name]
        if ($null -ne $property) {
            return $property.Value
        }
    }
    throw "missing result property; expected one of: $($Names -join ', ')"
}

function Get-P1ATestStrictText {
    param([Parameter(Mandatory)][string]$Path)

    $utf8 = [Text.UTF8Encoding]::new($false, $true)
    return [IO.File]::ReadAllText($Path, $utf8)
}

function Get-P1ATestAst {
    param([Parameter(Mandatory)][string]$Path)

    $tokens = $null
    $errors = $null
    $ast = [Management.Automation.Language.Parser]::ParseFile(
        $Path,
        [ref]$tokens,
        [ref]$errors
    )
    if ($errors.Count -ne 0) {
        throw "PowerShell parse errors in $Path`: $($errors.Message -join '; ')"
    }
    return $ast
}

function New-P1ATestDirectory {
    param(
        [Parameter(Mandatory)][string]$Parent,
        [Parameter(Mandatory)][string]$Name
    )

    $path = Join-Path $Parent $Name
    if (Test-Path -LiteralPath $path) {
        throw "test path already exists: $path"
    }
    [void](New-Item -ItemType Directory -Path $path)
    return (Resolve-Path -LiteralPath $path).Path
}

function New-P1ATestRun {
    param(
        [Parameter(Mandatory)][string]$OutputRoot,
        [Parameter(Mandatory)][ValidateSet('PASS', 'FAIL')][string]$Status,
        [string]$RunRoot,
        [switch]$MinimalEnvironment
    )

    $runId = if ([string]::IsNullOrWhiteSpace($RunRoot)) { New-P1ARunId }
        else { Split-Path -Leaf $RunRoot }
    if ([string]::IsNullOrWhiteSpace($RunRoot)) {
        $RunRoot = Join-Path (Join-Path $OutputRoot 'runs') $runId
    }
    [void][IO.Directory]::CreateDirectory((Join-Path $RunRoot 'artifacts'))
    [void][IO.Directory]::CreateDirectory((Join-Path $RunRoot 'commands'))
    $shaA = 'a' * 64
    $shaB = 'b' * 64
    $shaC = 'c' * 64
    $shaD = 'd' * 64
    $sourceHead = 'a' * 40
    $p0 = [ordered]@{
        status = 'PASS'
        receipt_commit = '86fb1e4cc68efeb651e5362c4aca85c2827d8e4d'
        receipt_sha256 = 'f08c6a41658ff287e238d6a96c4f2c874975964202c3eeced2bc0bc21f308904'
        contract_sha256 = 'fc2c60b52fdd7c524e0da06bb03972a4d523c21ad5536cba536185435bd44ad4'
        decision_ledger_sha256 = '8349d8a3e06d96d6921889de5534715e7b2f7439caf7e06558a97652a8890c8d'
    }
    $sourceIdentityPath = Join-Path $RunRoot 'artifacts\source-identity.json'
    Write-P1AJsonFile -Path $sourceIdentityPath -Value ([ordered]@{
            schema = 'test-source-identity-v1'
            head = $sourceHead
            input_manifest_sha256 = $shaD
        })
    $sourceHash = Get-P1ASha256 -Path $sourceIdentityPath
    $environment = if ($MinimalEnvironment) {
        [ordered]@{ schema = 'test-environment-v1'; status = 'PASS' }
    }
    else {
        $tool = {
            param([string]$Name, [string]$Path)
            [ordered]@{ name = $Name; version = '1.0'; path = $Path; sha256 = $shaA }
        }
        [ordered]@{
            schema = 'python-slm-environment-manifest-v1'
            phase_id = 'P1A'
            run_id = $runId
            mode = 'Cpu'
            status = 'PASS'
            p0 = [ordered]@{
                receipt_commit = $p0.receipt_commit
                receipt_sha256 = $p0.receipt_sha256
                contract_sha256 = $p0.contract_sha256
                decision_ledger_sha256 = $p0.decision_ledger_sha256
            }
            source = [ordered]@{
                head = $sourceHead
                dirty = $false
                input_manifest_sha256 = $shaD
                cargo_lock_sha256 = $shaA
                verifier_sha256 = $shaB
                schema_bundle_sha256 = $shaC
            }
            host = [ordered]@{
                os_version = '10.0.26100'
                os_architecture = 'AMD64'
                process_architecture = 'AMD64'
                powershell_edition = 'Desktop'
                powershell_version = '5.1.26100.4652'
            }
            rust = [ordered]@{
                rustc_version = '1.96.0'
                commit_hash = $sourceHead
                commit_date = '2026-05-25'
                host = 'x86_64-pc-windows-msvc'
                llvm_version = '22.1.2'
                rustc = & $tool 'rustc' '${RUST}/rustc.exe'
                cargo_version = '1.96.0'
                cargo = & $tool 'cargo' '${RUST}/cargo.exe'
                rustup = [ordered]@{
                    version = '1.28.2'
                    active_toolchain = '1.96.0-x86_64-pc-windows-msvc'
                    installed_target = $true
                    components = @('cargo', 'clippy', 'rustc', 'rustfmt')
                }
            }
            visual_studio = [ordered]@{
                discovery_method = 'vswhere'
                query = @(
                    '-version', '[17.0,18.0)', '-products', '*', '-requires',
                    'Microsoft.VisualStudio.Component.VC.Tools.x86.x64',
                    '-format', 'json', '-utf8'
                )
                instance_id = 'test-vs2022'
                product_id = 'Microsoft.VisualStudio.Product.BuildTools'
                product_version = '17.14.0'
                installation_version = '17.14.36811.4'
                complete = $true
                launchable = $true
                reboot_required = $false
                developer_environment_arguments = @(
                    '-arch=amd64', '-host_arch=amd64', '-startdir=none'
                )
                tools = @(
                    (& $tool 'cl' '${VC_TOOLS}/bin/Hostx64/x64/cl.exe')
                    (& $tool 'link' '${VC_TOOLS}/bin/Hostx64/x64/link.exe')
                    (& $tool 'lib' '${VC_TOOLS}/bin/Hostx64/x64/lib.exe')
                    (& $tool 'dumpbin' '${VC_TOOLS}/bin/Hostx64/x64/dumpbin.exe')
                )
            }
            windows_sdk = [ordered]@{
                version = '10.0.26100.0'
                ucrt_version = '10.0.26100.0'
                root = '${WINDOWS_KITS}'
                validated_paths = @(
                    '${WINDOWS_KITS}/Include/10.0.26100.0/um/Windows.h'
                    '${WINDOWS_KITS}/Include/10.0.26100.0/ucrt/stdlib.h'
                    '${WINDOWS_KITS}/Lib/10.0.26100.0/um/x64/kernel32.lib'
                    '${WINDOWS_KITS}/Lib/10.0.26100.0/ucrt/x64/ucrt.lib'
                    '${WINDOWS_KITS}/bin/10.0.26100.0/x64/rc.exe'
                    '${WINDOWS_KITS}/bin/10.0.26100.0/x64/mt.exe'
                )
                tools = @(
                    (& $tool 'rc' '${WINDOWS_KITS}/bin/10.0.26100.0/x64/rc.exe')
                    (& $tool 'mt' '${WINDOWS_KITS}/bin/10.0.26100.0/x64/mt.exe')
                )
            }
            isolation = [ordered]@{
                target_absent_before = $true
                target_outside_repository = $true
                offline = $true
                incremental_disabled = $true
                wrappers_disabled = $true
                cargo_config_absent = $true
                cleared_environment_names = @('CARGO_TARGET_DIR', 'RUSTC_WRAPPER')
                canaries = @('P1A_TEST_CANARY')
                canary_hits = @()
                forbidden_feature_matches = @()
                forbidden_artifacts = @()
                forbidden_imports = @()
                python_invoked = $false
                cuda_invoked = $false
            }
            native_probe = [ordered]@{
                c_source_sha256 = $shaA
                rust_source_sha256 = $shaB
                command_ids = @('C01', 'C02', 'C03', 'C04', 'C05', 'C06')
                result = 'PASS'
                pe_imports = @('KERNEL32.dll')
            }
            quality_gate = [ordered]@{
                command_ids = @('C07', 'C08', 'C09', 'C10')
                result = 'PASS'
            }
            cleanup = [ordered]@{
                temporary_root_removed = $true
                repository_target_unchanged = $true
                parent_environment_unchanged = $true
                inputs_unchanged = $true
            }
        }
    }
    $environmentPath = Join-Path $RunRoot 'artifacts\environment.json'
    Write-P1AJsonFile -Path $environmentPath -Value $environment
    $commandArgv = [ordered]@{
        C01 = @(
            'cl.exe', '/nologo', '/TC', '/W4', '/WX', '/MD',
            '${TEMP}\tool_canary.c', '/Fo${TEMP}\tool_canary.obj',
            '/Fe${TEMP}\tool_canary.exe', '/link', 'kernel32.lib'
        )
        C02 = @(
            'cl.exe', '/nologo', '/TC', '/W4', '/WX', '/MD', '/Bv', '/c',
            '${TEMP}\native_probe.c', '/Fo${TEMP}\native_probe.obj'
        )
        C03 = @(
            'lib.exe', '/nologo', '/OUT:${TEMP}\native_probe.lib',
            '${TEMP}\native_probe.obj'
        )
        C04 = @(
            'rustc.exe', '--edition=2024', '--target', 'x86_64-pc-windows-msvc',
            '${TEMP}\native_probe.rs', '-Lnative=${TEMP}', '-lstatic=native_probe',
            '-ldylib=kernel32', '-o', '${TEMP}\native_probe.exe'
        )
        C05 = @('native_probe.exe')
        C06 = @('dumpbin.exe', '/DEPENDENTS', '${TEMP}\native_probe.exe')
        C07 = @(
            'cargo.exe', 'test', '--locked', '--offline', '--no-default-features',
            '--features', 'cpu-reference', '--target', 'x86_64-pc-windows-msvc',
            '--no-run', '--message-format=json-render-diagnostics'
        )
        C08 = @('cargo.exe', 'fmt', '--all', '--', '--check')
        C09 = @(
            'cargo.exe', 'clippy', '--locked', '--all-targets', '--features',
            'cpu-reference', '--', '-D', 'warnings'
        )
        C10 = @('cargo.exe', 'test', '--locked', '--features', 'cpu-reference')
    }
    $commands = [Collections.Generic.List[object]]::new()
    foreach ($number in 1..10) {
        $commandId = 'C' + $number.ToString('00')
        $stdoutRelative = "commands/$commandId.stdout.txt"
        $stderrRelative = "commands/$commandId.stderr.txt"
        $stdoutPath = Join-Path $RunRoot $stdoutRelative.Replace('/', '\')
        $stderrPath = Join-Path $RunRoot $stderrRelative.Replace('/', '\')
        Write-P1AUtf8LfFile -Path $stdoutPath -Text "PASS $commandId"
        Write-P1AUtf8LfFile -Path $stderrPath -Text ''
        $commands.Add([ordered]@{
                id = $commandId
                argv = @($commandArgv[$commandId])
                cwd = '${REPO}'
                environment_delta_names = @()
                configuration_sha256 = $shaD
                exit_code = 0
                duration_ms = 1
                status = 'PASS'
                stdout = [ordered]@{
                    path = $stdoutRelative
                    sha256 = Get-P1ASha256 -Path $stdoutPath
                    bytes = [int64](Get-Item -LiteralPath $stdoutPath).Length
                }
                stderr = [ordered]@{
                    path = $stderrRelative
                    sha256 = Get-P1ASha256 -Path $stderrPath
                    bytes = [int64](Get-Item -LiteralPath $stderrPath).Length
                }
            })
    }
    Write-P1AUtf8LfFile -Path (Join-Path $RunRoot 'probe.txt') -Text 'probe'
    $sealReference = [ordered]@{
        path = 'SHA256SUMS'
        entries = @(Get-ChildItem -LiteralPath $RunRoot -Recurse -File).Count + 1
        coverage_rule = 'all_run_files_except_seal'
    }
    $environmentRef = [ordered]@{
        path = 'artifacts/environment.json'
        sha256 = Get-P1ASha256 -Path $environmentPath
        bytes = [int64](Get-Item -LiteralPath $environmentPath).Length
    }
    $gateNames = @(
        'p0', 'input_stability', 'toolchain', 'native_probe', 'cpu_graph',
        'clean_build', 'quality_gate', 'python_isolation', 'cuda_isolation',
        'redaction', 'cleanup'
    )
    $gates = [ordered]@{}
    foreach ($gateName in $gateNames) {
        $gates[$gateName] = [ordered]@{ status = 'PASS'; detail = 'verified by fixture' }
    }
    $evidencePath = Join-Path $RunRoot 'evidence.json'
    Write-P1AJsonFile -Path $evidencePath -Value ([ordered]@{
            schema = 'python-slm-phase-evidence-v1'
            phase_id = 'P1A'
            run_id = $runId
            mode = 'Cpu'
            status = $Status
            started_at = '2026-08-11T00:00:00Z'
            finished_at = '2026-08-11T00:00:01Z'
            duration_ms = 1000
            invocation = [ordered]@{
                argv = @(
                    'powershell', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
                    'scripts/verify-env.ps1', '-Mode', 'Cpu', '-OutputRoot',
                    'docs/receipts/P1A'
                )
                cwd = '${REPO}'
            }
            source_identity_sha256 = $sourceHash
            p0_dependency = $p0
            environment = $environmentRef
            commands = @($commands)
            gates = $gates
            errors = @()
            cleanup = [ordered]@{ attempted = $true; temporary_root_removed = $true }
            seal = $sealReference
        })
    [void](New-P1ASeal -RunRoot $RunRoot)
    return [pscustomobject][ordered]@{
        run_id = $runId
        run_root = $RunRoot
        evidence_path = $evidencePath
        source_hash = $sourceHash
    }
}

function Invoke-P1ATestPublication {
    param(
        [Parameter(Mandatory)][string]$OutputRoot,
        [Parameter(Mandatory)]$Run
    )

    return Publish-P1AAcceptance -OutputRoot $OutputRoot -RunId $Run.run_id `
        -SourceIdentitySha256 $Run.source_hash
}

function Update-P1ATestSealedEnvironment {
    param(
        [Parameter(Mandatory)]$Run,
        [Parameter(Mandatory)][scriptblock]$Mutation
    )

    $environmentPath = Join-Path $Run.run_root 'artifacts\environment.json'
    $sealPath = Join-Path $Run.run_root 'SHA256SUMS'
    $environment = Get-P1ATestStrictText -Path $environmentPath | ConvertFrom-Json
    $updated = & $Mutation $environment
    if ($null -eq $updated) {
        $updated = $environment
    }
    [IO.File]::Delete($sealPath)
    Write-P1AJsonFile -Path $environmentPath -Value $updated
    $evidence = Get-P1ATestStrictText -Path $Run.evidence_path | ConvertFrom-Json
    $evidence.environment.sha256 = Get-P1ASha256 -Path $environmentPath
    $evidence.environment.bytes = [int64](Get-Item -LiteralPath $environmentPath).Length
    Write-P1AJsonFile -Path $Run.evidence_path -Value $evidence
    [void](New-P1ASeal -RunRoot $Run.run_root)
}

function Assert-P1ATestSelectedAcceptanceChain {
    param([Parameter(Mandatory)][string]$OutputRoot)

    $acceptanceRoot = Join-Path $OutputRoot 'acceptances'
    $files = @(Get-ChildItem -LiteralPath $acceptanceRoot -File -Filter '*.json' |
        Sort-Object Name)
    Assert-P1ATest ($files.Count -ge 1) 'acceptance chain is empty'
    $previousHash = $null
    foreach ($index in 0..($files.Count - 1)) {
        $sequence = $index + 1
        Assert-P1AEqual $files[$index].Name ($sequence.ToString('00000000') + '.json') `
            'acceptance chain has a gap or noncanonical name'
        $acceptance = Get-P1ATestStrictText -Path $files[$index].FullName | ConvertFrom-Json
        Assert-P1AEqual $acceptance.schema 'python-slm-phase-acceptance-v1' `
            'acceptance chain contains the wrong schema'
        Assert-P1AEqual $acceptance.phase_id 'P1A' `
            'acceptance chain contains the wrong phase'
        Assert-P1AEqual $acceptance.sequence $sequence `
            'acceptance sequence field does not match its filename'
        Assert-P1AEqual $acceptance.status 'PASS' `
            'acceptance chain contains a non-PASS record'
        Assert-P1AEqual ([string]$acceptance.previous_acceptance_sha256) `
            ([string]$previousHash) 'acceptance predecessor hash mismatch'
        $previousHash = Get-P1ASha256 -Path $files[$index].FullName
    }
    $pointerPath = Join-Path $OutputRoot 'evidence.json'
    Assert-P1ATest (Test-Path -LiteralPath $pointerPath -PathType Leaf) `
        'acceptance chain has no selected pointer'
    $pointer = Get-P1ATestStrictText -Path $pointerPath | ConvertFrom-Json
    Assert-P1AEqual $pointer.schema 'python-slm-phase-evidence-pointer-v1' `
        'selected pointer schema changed'
    Assert-P1AEqual $pointer.phase_id 'P1A' 'selected pointer phase changed'
    Assert-P1AEqual $pointer.acceptance_path ('acceptances/' + $files[-1].Name) `
        'selected pointer does not identify the latest acceptance'
    Assert-P1AEqual $pointer.acceptance_sha256 $previousHash `
        'selected pointer does not hash-pin the latest acceptance'
    return [pscustomobject]@{ count = $files.Count; latest_hash = $previousHash }
}

$moduleFullPath = [IO.Path]::GetFullPath($ModulePath)
$entrypointFullPath = [IO.Path]::GetFullPath($EntrypointPath)
$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$temporaryBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$temporaryRoot = Join-Path $temporaryBase (
    'python-slm-p1a-tests-' + [guid]::NewGuid().ToString('N')
)
$module = $null

if (-not $temporaryRoot.StartsWith(
        $temporaryBase,
        [StringComparison]::OrdinalIgnoreCase
    )) {
    throw 'refusing to create tests outside the system temporary directory'
}
[void](New-Item -ItemType Directory -Path $temporaryRoot)
$temporaryRoot = (Resolve-Path -LiteralPath $temporaryRoot).Path

try {
    Invoke-P1ATest 'module and entrypoint parse without external test dependencies' {
        Assert-P1ATest (Test-Path -LiteralPath $moduleFullPath -PathType Leaf) `
            "missing verifier module: $moduleFullPath"
        Assert-P1ATest (Test-Path -LiteralPath $entrypointFullPath -PathType Leaf) `
            "missing verifier entrypoint: $entrypointFullPath"

        $asts = @(
            Get-P1ATestAst -Path $moduleFullPath
            Get-P1ATestAst -Path $entrypointFullPath
        )
        $forbiddenCommands = @(
            'python', 'python.exe', 'py', 'py.exe', 'pip', 'pip.exe',
            'conda', 'uv', 'poetry', 'invoke-pester', 'setx',
            'invoke-webrequest', 'invoke-restmethod', 'start-bitstransfer',
            'curl', 'curl.exe', 'wget', 'wget.exe', 'winget', 'choco'
        )
        foreach ($ast in $asts) {
            $commands = $ast.FindAll({
                    param($node)
                    $node -is [Management.Automation.Language.CommandAst]
                }, $true)
            foreach ($command in $commands) {
                $name = $command.GetCommandName()
                if ($null -ne $name -and $forbiddenCommands -contains $name.ToLowerInvariant()) {
                    throw "forbidden command in verifier: $name"
                }
            }
        }

        $source = (Get-P1ATestStrictText $moduleFullPath) + "`n" +
            (Get-P1ATestStrictText $entrypointFullPath)
        Assert-P1ATest ($source -notmatch '(?im)^\s*#requires\s+-modules?\s+') `
            'verifier must not require external PowerShell modules'
        Assert-P1ATest ($source -notmatch '(?i)\bImport-Module\s+(?:Pester|powershell-yaml|ajson)\b') `
            'verifier imports a forbidden external module'
        Assert-P1ATest ($source -notmatch '(?i)SetEnvironmentVariable\s*\([^\r\n]*["''](?:User|Machine)["'']') `
            'verifier mutates user- or machine-scoped environment variables'
    }

    $module = Import-Module -Name $moduleFullPath -Force -PassThru

    Invoke-P1ATest 'module exports the stable P1A helper surface only' {
        $expected = @(
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
        ) | Sort-Object
        $actual = @($module.ExportedFunctions.Keys) | Sort-Object
        Assert-P1AEqual ($actual -join "`n") ($expected -join "`n") `
            'exported helper set changed'
    }

    Invoke-P1ATest 'CPU verifier preserves the approved key command sequence' {
        $source = Get-P1ATestStrictText -Path $moduleFullPath
        $orderedPatterns = @(
            [pscustomobject]@{ label = 'last P0 check'; pattern = '\$receiptStatus\s*=\s*Invoke-P1ARecordedCommand' }
            [pscustomobject]@{ label = 'Rust tool check'; pattern = '\$rustcVersionCommand\s*=\s*Invoke-P1ARecordedCommand' }
            [pscustomobject]@{ label = 'native probe start'; pattern = '\$canaryCompile\s*=\s*Invoke-P1ARecordedCommand' }
            [pscustomobject]@{ label = 'Cargo metadata'; pattern = '\$metadataCommand\s*=\s*Invoke-P1ARecordedCommand' }
            [pscustomobject]@{ label = 'Cargo tree'; pattern = '\$treeCommand\s*=\s*Invoke-P1ARecordedCommand' }
            [pscustomobject]@{ label = 'clean no-run build'; pattern = '\$cleanBuild\s*=\s*Invoke-P1ARecordedCommand' }
            [pscustomobject]@{ label = 'cargo fmt'; pattern = "@\{\s*Args\s*=\s*@\('fmt'" }
            [pscustomobject]@{ label = 'cargo clippy'; pattern = "@\{\s*Args\s*=\s*@\('clippy'" }
            [pscustomobject]@{ label = 'cargo test'; pattern = "@\{\s*Args\s*=\s*@\('test'" }
            [pscustomobject]@{ label = 'Cargo PE audits'; pattern = '\$peDump\s*=\s*Invoke-P1ARecordedCommand' }
        )
        $previousIndex = -1
        foreach ($expectedStep in $orderedPatterns) {
            $match = [regex]::Match($source, $expectedStep.pattern)
            Assert-P1ATest $match.Success "missing command-sequence step: $($expectedStep.label)"
            Assert-P1ATest ($match.Index -gt $previousIndex) `
                "command-sequence step is out of order: $($expectedStep.label)"
            $previousIndex = $match.Index
        }
    }

    Invoke-P1ATest 'CPU graph failure records its exact forbidden matches before stopping' {
        $source = Get-P1ATestStrictText -Path $moduleFullPath
        $artifactWrite = $source.IndexOf("Join-Path `$artifactsRoot 'cargo-graph.json'")
        $failureThrow = $source.IndexOf(
            "throw '[CPU_GRAPH_CONTAINS_CUDA] activated CPU dependency graph contains CUDA packages or features'"
        )
        Assert-P1ATest ($artifactWrite -ge 0) 'CPU graph artifact writer is missing'
        Assert-P1ATest ($failureThrow -gt $artifactWrite) `
            'CPU graph failure can occur before its forbidden-match artifact is written'
        Assert-P1ATest ($source -match 'forbidden_matches\s*=\s*@\(\$forbiddenFeatures\)') `
            'CPU graph artifact does not retain the exact forbidden matches'
    }

    Invoke-P1ATest 'CPU graph classification uses only the activated feature tree' {
        $treeText = @'
rust-llm-pretrain v0.1.0
├── burn feature "flex"
└── serde feature "derive"
'@
        $dormantOnly = & $module {
            param($Text)
            @(Get-P1AForbiddenCargoTreeTokens -Text $Text)
        } $treeText
        Assert-P1AEqual @($dormantOnly).Count 0 `
            'CPU graph rejected CUDA names that were absent from the activated tree'

        $activeText = $treeText + "`ncubecl-cuda feature `"default`"`ncudarc feature `"nvrtc`""
        $active = @(& $module {
                param($Text)
                @(Get-P1AForbiddenCargoTreeTokens -Text $Text)
            } $activeText)
        Assert-P1AEqual ($active -join "`n") (@(
                'cargo-tree-token:cubecl-cuda'
                'cargo-tree-token:cudarc'
                'cargo-tree-token:nvrtc'
            ) -join "`n") 'activated CUDA tokens were not classified deterministically'
    }

    Invoke-P1ATest 'command-line rendering is deterministic and single-line' {
        $arguments = @(
            'tool.exe', 'plain', 'space value', '', 'quote"inside',
            'C:\path with spaces\tail\', 'ampersand&is-literal'
        )
        $first = ConvertTo-P1ACommandLine -Argument $arguments
        $second = ConvertTo-P1ACommandLine -Argument $arguments
        Assert-P1AEqual $first $second 'command-line rendering is not deterministic'
        Assert-P1ATest ($first -is [string]) 'command-line rendering did not return a string'
        Assert-P1ATest ($first -notmatch '[\r\n]') 'command-line rendering contains a newline'
        Assert-P1ATest ($first -match 'space value') 'spaced argument disappeared'
        Assert-P1ATest ($first -match 'ampersand&is-literal') 'literal metacharacter disappeared'
        Assert-P1ATest ($first -match '""') 'empty argument is not represented'
    }

    Invoke-P1ATest 'UTF-8/LF writers and SHA-256 are byte deterministic' {
        $dir = New-P1ATestDirectory -Parent $temporaryRoot -Name 'encoding'
        $firstPath = Join-Path $dir 'first.txt'
        $secondPath = Join-Path $dir 'second.txt'
        Write-P1AUtf8LfFile -Path $firstPath -Text "alpha`r`nbeta`r"
        Write-P1AUtf8LfFile -Path $secondPath -Text "alpha`nbeta`n"

        $bytes = [IO.File]::ReadAllBytes($firstPath)
        Assert-P1ATest ($bytes.Length -ge 1) 'writer emitted an empty file'
        Assert-P1ATest (-not (
                $bytes.Length -ge 3 -and
                $bytes[0] -eq 0xef -and
                $bytes[1] -eq 0xbb -and
                $bytes[2] -eq 0xbf
            )) 'writer emitted a UTF-8 BOM'
        Assert-P1AEqual (Get-P1ATestStrictText $firstPath) "alpha`nbeta`n" `
            'writer did not normalize CR/LF or terminal newline'
        Assert-P1AEqual (Get-P1ASha256 -Path $firstPath) `
            'e49c81e2d2f84e259d40e2fb8192f3bcd198b355184845d76d8f58807d0d78ee' `
            'unexpected SHA-256 for known bytes'
        Assert-P1AEqual (Get-P1ASha256 -Path $firstPath) `
            (Get-P1ASha256 -Path $secondPath) 'equivalent normalized text hashed differently'

        $jsonA = Join-Path $dir 'a.json'
        $jsonB = Join-Path $dir 'b.json'
        $value = [ordered]@{ schema = 'test-v1'; nested = [ordered]@{ value = 7 }; list = @(1, 2) }
        Write-P1AJsonFile -Path $jsonA -Value $value
        Write-P1AJsonFile -Path $jsonB -Value $value
        [void](Get-P1ATestStrictText $jsonA | ConvertFrom-Json)
        Assert-P1AEqual (Get-P1ASha256 $jsonA) (Get-P1ASha256 $jsonB) `
            'JSON writer is not deterministic'
        Assert-P1ATest ((Get-P1ATestStrictText $jsonA) -notmatch "`r") `
            'JSON writer emitted CRLF'
    }

    Invoke-P1ATest 'path containment rejects prefix and traversal confusion' {
        $root = New-P1ATestDirectory -Parent $temporaryRoot -Name 'containment-root'
        $child = Join-Path $root 'nested\file.txt'
        $sibling = $root + '-sibling\file.txt'
        $escape = Join-Path $root 'nested\..\..\escape.txt'
        Assert-P1ATest (Test-P1APathWithin -Path $root -Root $root) `
            'root must contain itself'
        Assert-P1ATest (Test-P1APathWithin -Path $child -Root $root) `
            'root must contain a lexical child'
        Assert-P1ATest (-not (Test-P1APathWithin -Path $sibling -Root $root)) `
            'prefix sibling escaped containment'
        Assert-P1ATest (-not (Test-P1APathWithin -Path $escape -Root $root)) `
            'dot-dot traversal escaped containment'
        Assert-P1ATest (Test-P1APathWithin -Path $child.ToUpperInvariant() `
                -Root $root.ToLowerInvariant()) 'Windows containment must be case-insensitive'
    }

    Invoke-P1ATest 'output-root resolution permits only the receipt subtree' {
        $fakeRepository = New-P1ATestDirectory -Parent $temporaryRoot -Name 'fake-repository'
        [void](New-Item -ItemType Directory -Path (Join-Path $fakeRepository 'docs\receipts') -Force)
        [void](New-Item -ItemType Directory -Path (Join-Path $fakeRepository 'src') -Force)
        $safe = Join-Path $fakeRepository 'docs\receipts\P1A'
        $resolved = Resolve-P1AOutputRoot -RepositoryRoot $fakeRepository -OutputRoot $safe
        Assert-P1AEqual $resolved ([IO.Path]::GetFullPath($safe)) `
            'safe output root resolved unexpectedly'

        Assert-P1AThrows { Resolve-P1AOutputRoot -RepositoryRoot $fakeRepository `
                -OutputRoot (Join-Path $fakeRepository '..\outside') } '(?i)(outside|contain|receipt|output)'
        Assert-P1AThrows { Resolve-P1AOutputRoot -RepositoryRoot $fakeRepository `
                -OutputRoot $fakeRepository } '(?i)(root|receipt|output)'
        Assert-P1AThrows { Resolve-P1AOutputRoot -RepositoryRoot $fakeRepository `
                -OutputRoot (Join-Path $fakeRepository 'src\P1A') } '(?i)(receipt|output|allowed)'

        $filePath = Join-Path $fakeRepository 'docs\receipts\not-a-directory'
        [IO.File]::WriteAllText($filePath, 'x')
        Assert-P1AThrows { Resolve-P1AOutputRoot -RepositoryRoot $fakeRepository `
                -OutputRoot $filePath } '(?i)(directory|file|output)'
    }

    Invoke-P1ATest 'normalized paths are stable, relative, and slash-normalized' {
        $base = New-P1ATestDirectory -Parent $temporaryRoot -Name 'normalized-base'
        $path = Join-Path $base 'one\two\file.txt'
        $first = ConvertTo-P1ANormalizedPath -Path $path -BasePath $base
        $second = ConvertTo-P1ANormalizedPath -Path $path -BasePath $base
        Assert-P1AEqual $first $second 'normalized path is not deterministic'
        Assert-P1ATest ($first -notmatch [regex]::Escape($base)) `
            'normalized path leaked its absolute base'
        Assert-P1ATest ($first -notmatch '\\') 'normalized path retained backslashes'
        Assert-P1ATest ($first -notmatch '(^|/)\.\.(/|$)') 'normalized path contains traversal'
        Assert-P1ATest ($first -match 'one/two/file\.txt$') 'normalized path lost its suffix'
    }

    Invoke-P1ATest 'run IDs match the closed schema and are collision resistant' {
        $ids = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
        1..64 | ForEach-Object {
            $id = New-P1ARunId
            Assert-P1ATest ($id -match '^[0-9]{8}T[0-9]{9}Z-[0-9a-f]{24}$') `
                "invalid run ID: $id"
            Assert-P1ATest ($ids.Add($id)) "duplicate run ID: $id"
        }
    }

    Invoke-P1ATest 'rustc parser accepts the pinned host and rejects incomplete identity' {
        $valid = @'
rustc 1.96.0 (ac68faa20 2026-05-25)
binary: rustc
commit-hash: ac68faa20c58cbccd01ee7208bf3b6e93a7d7f96
commit-date: 2026-05-25
host: x86_64-pc-windows-msvc
release: 1.96.0
LLVM version: 22.1.2
'@
        $parsed = ConvertFrom-P1ARustcVersion -Text $valid
        Assert-P1AEqual $parsed.release '1.96.0' 'rustc release parsed incorrectly'
        Assert-P1AEqual $parsed.host 'x86_64-pc-windows-msvc' 'rustc host parsed incorrectly'
        Assert-P1AThrows { ConvertFrom-P1ARustcVersion -Text 'release: 1.96.0' } `
            '(?i)(host|rustc|version|parse)'
        Assert-P1AThrows { ConvertFrom-P1ARustcVersion -Text "host: x86_64-pc-windows-msvc`n" } `
            '(?i)(release|rustc|version|parse)'
        Assert-P1AThrows { ConvertFrom-P1ARustcVersion -Text '' } '(?i)(empty|rustc|version|parse)'
    }

    Invoke-P1ATest 'VS selector rejects newer major versions and is input-order independent' {
        $vs17Old = [pscustomobject]@{
            installationVersion = '17.13.100.1'
            installationPath = 'C:\VS\2022-old'
            isComplete = $true
            isLaunchable = $true
            isRebootRequired = $false
            isPrerelease = $false
        }
        $vs17New = [pscustomobject]@{
            installationVersion = '17.14.36811.4'
            installationPath = 'C:\VS\2022-new'
            isComplete = $true
            isLaunchable = $true
            isRebootRequired = $false
            isPrerelease = $false
        }
        $vs18 = [pscustomobject]@{
            installationVersion = '18.1.11312.151'
            installationPath = 'C:\VS\2026'
            isComplete = $true
            isLaunchable = $true
            isRebootRequired = $false
            isPrerelease = $false
        }
        $first = Select-P1AVs2022Instance -Instances @($vs18, $vs17Old, $vs17New)
        $second = Select-P1AVs2022Instance -Instances @($vs17New, $vs18, $vs17Old)
        Assert-P1AEqual $first.installationVersion '17.14.36811.4' `
            'selector did not choose the newest VS 2022 instance'
        Assert-P1AEqual $second.installationVersion $first.installationVersion `
            'VS selection depends on input order'
        Assert-P1AThrows { Select-P1AVs2022Instance -Instances @($vs18) } `
            '(?i)(2022|17\.|visual studio)'

        $incomplete = $vs17New.PSObject.Copy()
        $incomplete.isComplete = $false
        Assert-P1AThrows { Select-P1AVs2022Instance -Instances @($incomplete) } `
            '(?i)(complete|2022|visual studio)'
    }

    Invoke-P1ATest 'redaction removes configured roots and is idempotent' {
        $extra = New-P1ATestDirectory -Parent $temporaryRoot -Name 'redaction-extra'
        $text = "repo=$repositoryRoot\src\lib.rs extra=$extra\secret.txt"
        $protected = Protect-P1AText -Text $text -RepositoryRoot $repositoryRoot `
            -ExtraRoots @($extra)
        Assert-P1ATest ($protected -notmatch [regex]::Escape($repositoryRoot)) `
            'repository absolute path leaked after redaction'
        Assert-P1ATest ($protected -notmatch [regex]::Escape($extra)) `
            'extra absolute path leaked after redaction'
        Assert-P1ATest ($protected -match '\$\{[A-Z0-9_]+\}') `
            'redaction did not emit a stable token'
        Assert-P1AEqual (Protect-P1AText -Text $protected -RepositoryRoot $repositoryRoot `
                -ExtraRoots @($extra)) $protected 'redaction is not idempotent'
    }

    Invoke-P1ATest 'process runner preserves argv, cwd, streams, exit code, and environment' {
        $working = New-P1ATestDirectory -Parent $temporaryRoot -Name 'process-working'
        $powerShell = Join-Path $PSHOME 'powershell.exe'
        $marker = 'p1a-' + [guid]::NewGuid().ToString('N')
        $childPath = Join-Path $working 'child.ps1'
        $child = @'
param([Parameter(Mandatory)][string]$Value)
[Console]::Out.Write("OUT:" + $env:P1A_TEST_MARKER + ":" + ${env:3DVPATH} + ":" + (Get-Location).Path)
[Console]::Error.Write("ERR:" + $Value)
exit 7
'@
        Write-P1AUtf8LfFile -Path $childPath -Text $child
        $result = Invoke-P1AProcess -FilePath $powerShell -ArgumentList @(
            '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $childPath,
            'literal & value'
        ) -WorkingDirectory $repositoryRoot -Environment @{
            P1A_TEST_MARKER = $marker
            '3DVPATH' = 'numeric-name'
        } `
            -TimeoutSeconds 15
        $exitCode = Get-P1ATestProperty $result @('exit_code', 'ExitCode')
        $stdout = [string](Get-P1ATestProperty $result @('stdout', 'Stdout'))
        $stderr = [string](Get-P1ATestProperty $result @('stderr', 'Stderr'))
        Assert-P1AEqual $exitCode 7 'process exit code was not preserved'
        Assert-P1ATest ($stdout -match [regex]::Escape("OUT:${marker}:numeric-name:$repositoryRoot")) `
            "process stdout/cwd/environment was not preserved; stdout=<$stdout> stderr=<$stderr>"
        Assert-P1ATest ($stderr -match [regex]::Escape('ERR:literal & value')) `
            'process stderr or literal argument was not preserved'
        Assert-P1AThrows { Invoke-P1AProcess -FilePath (Join-Path $working 'missing.exe') `
                -ArgumentList @() -WorkingDirectory $working -Environment @{} `
                -TimeoutSeconds 2 } '(?i)(missing|not found|executable|process)'
    }

    Invoke-P1ATest 'process runner terminates a bounded timeout' {
        $working = New-P1ATestDirectory -Parent $temporaryRoot -Name 'timeout-working'
        $powerShell = Join-Path $PSHOME 'powershell.exe'
        $childPath = Join-Path $working 'sleep.ps1'
        Write-P1AUtf8LfFile -Path $childPath -Text 'Start-Sleep -Seconds 4'
        $caught = $null
        $result = $null
        try {
            $result = Invoke-P1AProcess -FilePath $powerShell -ArgumentList @(
                '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $childPath
            ) -WorkingDirectory $repositoryRoot -Environment @{} -TimeoutSeconds 1
        }
        catch {
            $caught = $_
        }
        if ($null -ne $caught) {
            Assert-P1ATest ($caught.Exception.Message -match '(?i)timeout|timed out') `
                'timeout threw an unrelated error'
        }
        else {
            $timedOut = Get-P1ATestProperty $result @('timed_out', 'TimedOut')
            Assert-P1ATest ([bool]$timedOut) `
                "timed-out process was reported as completed; result=$($result | ConvertTo-Json -Compress)"
        }
    }

    Invoke-P1ATest 'Cuda entrypoint emits sealed failure on stderr without moving pointer' {
        $fakeRepository = New-P1ATestDirectory -Parent $temporaryRoot -Name 'cuda-entrypoint-repository'
        $fakeScripts = Join-Path $fakeRepository 'scripts'
        $fakeLibrary = Join-Path $fakeScripts 'lib'
        [void][IO.Directory]::CreateDirectory($fakeLibrary)
        $fakeEntrypoint = Join-Path $fakeScripts 'verify-env.ps1'
        $fakeModule = Join-Path $fakeLibrary 'VerifyEnv.psm1'
        [IO.File]::Copy($entrypointFullPath, $fakeEntrypoint, $false)
        [IO.File]::Copy($moduleFullPath, $fakeModule, $false)

        $outputRoot = Join-Path $fakeRepository 'docs\receipts\P1B'
        [void][IO.Directory]::CreateDirectory($outputRoot)
        $pointerPath = Join-Path $outputRoot 'evidence.json'
        Write-P1AUtf8LfFile -Path $pointerPath -Text 'preexisting P1B sentinel'
        $pointerHash = Get-P1ASha256 -Path $pointerPath

        $powerShell = Join-Path $PSHOME 'powershell.exe'
        $result = Invoke-P1AProcess -FilePath $powerShell -ArgumentList @(
            '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $fakeEntrypoint,
            '-Mode', 'Cuda', '-OutputRoot', 'docs\receipts\P1B'
        ) -WorkingDirectory $repositoryRoot -Environment @{} -TimeoutSeconds 30
        Assert-P1AEqual $result.exit_code 5 'Cuda entrypoint returned the wrong failure category'
        Assert-P1ATest (-not [bool]$result.timed_out) 'Cuda entrypoint timed out'
        Assert-P1ATest ([string]::IsNullOrWhiteSpace([string]$result.stdout)) `
            'failure entrypoint wrote a success payload to stdout'

        $stderrLines = @([string]$result.stderr -split '\r?\n' | Where-Object {
                -not [string]::IsNullOrWhiteSpace($_)
            })
        Assert-P1AEqual $stderrLines.Count 2 `
            'Cuda entrypoint did not emit one diagnostic and one terminal error'
        $diagnostic = $stderrLines[0] | ConvertFrom-Json
        $terminal = $stderrLines[1] | ConvertFrom-Json
        Assert-P1AEqual $diagnostic.schema 'python-slm-script-diagnostic-v1' `
            'Cuda diagnostic schema changed'
        Assert-P1AEqual $diagnostic.phase_id 'P1B' 'Cuda diagnostic phase changed'
        Assert-P1AEqual $diagnostic.code 'MODE_NOT_IMPLEMENTED' `
            'Cuda diagnostic did not expose the bounded placeholder failure'
        Assert-P1AEqual $diagnostic.category 5 'Cuda diagnostic category changed'
        Assert-P1AEqual $terminal.schema 'python-slm-script-error-v1' `
            'Cuda terminal error schema changed'
        Assert-P1AEqual $terminal.code 'QUALIFICATION_FAILED' `
            'Cuda terminal error code changed'
        Assert-P1AEqual $terminal.category 5 'Cuda terminal exit category changed'
        Assert-P1AEqual (Get-P1ASha256 -Path $pointerPath) $pointerHash `
            'Cuda failure moved or replaced the preexisting pointer'
        Assert-P1ATest (-not (Test-Path -LiteralPath (Join-Path $outputRoot 'acceptances'))) `
            'Cuda placeholder created an acceptance directory'

        $runs = @(Get-ChildItem -LiteralPath (Join-Path $outputRoot 'runs') -Directory)
        Assert-P1AEqual $runs.Count 1 'Cuda entrypoint did not create exactly one failed run'
        $evidencePath = Join-Path $runs[0].FullName 'evidence.json'
        $evidence = Get-P1ATestStrictText -Path $evidencePath | ConvertFrom-Json
        Assert-P1AEqual $evidence.phase_id 'P1B' 'Cuda evidence phase changed'
        Assert-P1AEqual $evidence.mode 'Cuda' 'Cuda evidence mode changed'
        Assert-P1AEqual $evidence.status 'FAIL' 'Cuda evidence was not sealed as FAIL'
        Assert-P1AEqual $evidence.p0_dependency.status 'FAIL' `
            'Cuda placeholder overstated the P0 dependency'
        Assert-P1ATest ($null -eq $evidence.environment) `
            'Cuda placeholder emitted a fabricated environment manifest'
        Assert-P1AEqual @($evidence.commands).Count 0 `
            'Cuda placeholder executed qualification commands'
        Assert-P1AEqual @($evidence.errors).Count 1 `
            'Cuda placeholder error cardinality changed'
        Assert-P1AEqual $evidence.errors[0].code 'MODE_NOT_IMPLEMENTED' `
            'Cuda sealed error code changed'
        Assert-P1AEqual ($evidence.invocation.argv -join "`n") (@(
                'powershell', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
                'scripts/verify-env.ps1', '-Mode', 'Cuda', '-OutputRoot',
                'docs/receipts/P1B'
            ) -join "`n") 'Cuda evidence invocation changed'
        Assert-P1AEqual $terminal.run_id $evidence.run_id `
            'terminal error did not identify the sealed Cuda run'
        Assert-P1ATest ([string]$terminal.evidence_path -match `
                ('^docs/receipts/P1B/runs/' + [regex]::Escape($evidence.run_id) + '/evidence\.json$')) `
            'terminal error did not expose the repository-relative evidence path'
        Assert-P1ATest (Test-P1ASeal -RunRoot $runs[0].FullName) `
            'Cuda failed run seal did not verify'
        Assert-P1AEqual $evidence.seal.coverage_rule 'all_run_files_except_seal' `
            'Cuda evidence used the legacy incomplete coverage rule'
        Assert-P1ATest ((Get-P1ATestStrictText -Path `
                    (Join-Path $runs[0].FullName 'SHA256SUMS')) -match `
                '(?m)  evidence\.json$') 'Cuda seal omitted terminal evidence'
    }

    Invoke-P1ATest 'seal is complete, sorted, mutation-sensitive, and traversal-safe' {
        $run = New-P1ATestDirectory -Parent $temporaryRoot -Name 'seal-run'
        [void](New-Item -ItemType Directory -Path (Join-Path $run 'commands'))
        [void](New-Item -ItemType Directory -Path (Join-Path $run 'artifacts'))
        Write-P1AUtf8LfFile -Path (Join-Path $run 'commands\C01.stdout.txt') -Text 'out'
        Write-P1AUtf8LfFile -Path (Join-Path $run 'commands\C01.stderr.txt') -Text ''
        Write-P1AJsonFile -Path (Join-Path $run 'artifacts\environment.json') `
            -Value ([ordered]@{ schema = 'test-environment-v1' })

        [void](New-P1ASeal -RunRoot $run)
        Assert-P1ATest (Test-P1ASeal -RunRoot $run) 'fresh seal did not verify'
        $sealPath = Join-Path $run 'SHA256SUMS'
        $lines = @(Get-Content -Encoding UTF8 -LiteralPath $sealPath)
        Assert-P1AEqual $lines.Count 3 'seal did not cover every pre-seal run file'
        $paths = [Collections.Generic.List[string]]::new()
        foreach ($line in $lines) {
            Assert-P1ATest ($line -match '^[0-9a-f]{64}  [^:\\]+(?:/[^:\\]+)*$') `
                "malformed seal entry: $line"
            Assert-P1ATest ($line -notmatch 'SHA256SUMS') 'seal contains itself'
            $paths.Add($line.Substring(66))
        }
        Assert-P1AEqual ($paths -join "`n") (($paths | Sort-Object) -join "`n") `
            'seal entries are not sorted by repository-relative path'

        Write-P1AUtf8LfFile -Path (Join-Path $run 'commands\C01.stdout.txt') -Text 'changed'
        Assert-P1ATest (-not (Test-P1ASeal -RunRoot $run)) `
            'seal accepted a mutated file'
    }

    Invoke-P1ATest 'seal rejects files added after sealing' {
        $run = New-P1ATestDirectory -Parent $temporaryRoot -Name 'seal-added-file'
        [void](New-Item -ItemType Directory -Path (Join-Path $run 'artifacts'))
        Write-P1AUtf8LfFile -Path (Join-Path $run 'artifacts\one.txt') -Text 'one'
        [void](New-P1ASeal -RunRoot $run)
        Write-P1AUtf8LfFile -Path (Join-Path $run 'artifacts\two.txt') -Text 'two'
        Assert-P1ATest (-not (Test-P1ASeal -RunRoot $run)) `
            'seal accepted an uncovered file'
    }

    Invoke-P1ATest 'seal rejects malformed and escaping entries' {
        $run = New-P1ATestDirectory -Parent $temporaryRoot -Name 'seal-malformed'
        Write-P1AUtf8LfFile -Path (Join-Path $run 'inside.txt') -Text 'inside'
        [void](New-P1ASeal -RunRoot $run)
        $sealPath = Join-Path $run 'SHA256SUMS'
        $hash = Get-P1ASha256 -Path (Join-Path $run 'inside.txt')
        Write-P1AUtf8LfFile -Path $sealPath -Text "$hash  ../outside.txt"
        Assert-P1ATest (-not (Test-P1ASeal -RunRoot $run)) `
            'seal accepted a traversal entry'
        Write-P1AUtf8LfFile -Path $sealPath -Text "$($hash.ToUpperInvariant())  inside.txt"
        Assert-P1ATest (-not (Test-P1ASeal -RunRoot $run)) `
            'seal accepted a non-canonical hash'
        Write-P1AUtf8LfFile -Path $sealPath -Text "$hash inside.txt"
        Assert-P1ATest (-not (Test-P1ASeal -RunRoot $run)) `
            'seal accepted a malformed separator'
    }

    Invoke-P1ATest 'seal creation is create-new and covers terminal evidence' {
        $run = New-P1ATestDirectory -Parent $temporaryRoot -Name 'seal-immutable'
        Write-P1AUtf8LfFile -Path (Join-Path $run 'payload.txt') -Text 'payload'
        Write-P1AJsonFile -Path (Join-Path $run 'evidence.json') `
            -Value ([ordered]@{ schema = 'test-evidence-v1' })
        [void](New-P1ASeal -RunRoot $run)
        $sealPath = Join-Path $run 'SHA256SUMS'
        $before = Get-P1ASha256 -Path $sealPath
        $sealText = Get-P1ATestStrictText -Path $sealPath
        Assert-P1ATest ($sealText -match '(?m)  evidence\.json$') `
            'seal omitted terminal evidence.json'
        Assert-P1AThrows { [void](New-P1ASeal -RunRoot $run) } `
            '(?i)(existing|replace|sealed|seal)'
        Assert-P1AEqual (Get-P1ASha256 -Path $sealPath) $before `
            'second seal attempt changed an immutable manifest'
    }

    Invoke-P1ATest 'seal covers both root and nested evidence files' {
        $run = New-P1ATestDirectory -Parent $temporaryRoot -Name 'seal-evidence-scope'
        [void][IO.Directory]::CreateDirectory((Join-Path $run 'artifacts'))
        Write-P1AJsonFile -Path (Join-Path $run 'evidence.json') `
            -Value ([ordered]@{ schema = 'root-evidence-v1' })
        Write-P1AJsonFile -Path (Join-Path $run 'artifacts\evidence.json') `
            -Value ([ordered]@{ schema = 'nested-evidence-v1' })
        Write-P1AUtf8LfFile -Path (Join-Path $run 'payload.txt') -Text 'payload'
        [void](New-P1ASeal -RunRoot $run)
        $sealText = Get-P1ATestStrictText -Path (Join-Path $run 'SHA256SUMS')
        Assert-P1ATest ($sealText -match '(?m)^.+  evidence\.json$') `
            'seal omitted root evidence.json'
        Assert-P1ATest ($sealText -match '(?m)  artifacts/evidence\.json$') `
            'seal failed to cover a nested file merely named evidence.json'
    }

    Invoke-P1ATest 'seal rejects empty runs and noncanonical entry order' {
        $empty = New-P1ATestDirectory -Parent $temporaryRoot -Name 'seal-empty'
        Assert-P1AThrows { [void](New-P1ASeal -RunRoot $empty) } `
            '(?i)(empty|file|seal)'
        Write-P1AUtf8LfFile -Path (Join-Path $empty 'SHA256SUMS') -Text ''
        Assert-P1ATest (-not (Test-P1ASeal -RunRoot $empty)) `
            'seal verifier accepted an empty manifest for an empty run'

        $run = New-P1ATestDirectory -Parent $temporaryRoot -Name 'seal-order'
        Write-P1AUtf8LfFile -Path (Join-Path $run 'a.txt') -Text 'a'
        Write-P1AUtf8LfFile -Path (Join-Path $run 'z.txt') -Text 'z'
        [void](New-P1ASeal -RunRoot $run)
        $sealPath = Join-Path $run 'SHA256SUMS'
        $lines = @([IO.File]::ReadAllLines($sealPath, [Text.UTF8Encoding]::new($false, $true)))
        [array]::Reverse($lines)
        Write-P1AUtf8LfFile -Path $sealPath -Text ($lines -join "`n")
        Assert-P1ATest (-not (Test-P1ASeal -RunRoot $run)) `
            'seal verifier accepted noncanonical entry order'
    }

    Invoke-P1ATest 'legacy failed-run compatibility is closed and mutation-sensitive' {
        $legacyIds = @(
            '20260811T135205572Z-b4054fbd19f3447cbc6cb13e'
            '20260811T135349352Z-b7b1c0230b47473ba22ce6c0'
            '20260811T135551718Z-bac8f20fcdc54147811a1266'
            '20260811T135746406Z-aef03b68c7fb4bb5b6b75e17'
            '20260811T135924951Z-21d92bf2c9f14bdb8c57bbc2'
            '20260811T140125365Z-bcbbe06ec66245b4941cc2b9'
            '20260811T140306815Z-3af3710134354c378097aa9a'
            '20260811T140631147Z-fc46ebcc30184ee09fca08bd'
        )
        foreach ($legacyId in $legacyIds) {
            $legacyRoot = Join-Path $repositoryRoot "docs\receipts\P1A\runs\$legacyId"
            Assert-P1ATest (Test-P1ASeal -RunRoot $legacyRoot) `
                "known legacy failed run no longer verifies: $legacyId"
        }

        $copyRoot = New-P1ATestDirectory -Parent $temporaryRoot -Name 'legacy-copy'
        $sourceRoot = Join-Path $repositoryRoot `
            'docs\receipts\P1A\runs\20260811T140306815Z-3af3710134354c378097aa9a'
        foreach ($item in Get-ChildItem -LiteralPath $sourceRoot -Force) {
            Copy-Item -LiteralPath $item.FullName -Destination $copyRoot -Recurse
        }
        Assert-P1ATest (Test-P1ASeal -RunRoot $copyRoot) `
            'byte-identical known legacy run copy did not verify'
        $legacyEvidencePath = Join-Path $copyRoot 'evidence.json'
        Write-P1AUtf8LfFile -Path $legacyEvidencePath -Text `
            ((Get-P1ATestStrictText -Path $legacyEvidencePath) + "`n")
        Assert-P1ATest (-not (Test-P1ASeal -RunRoot $copyRoot)) `
            'legacy seal accepted mutated terminal evidence'

        $arbitraryRoot = New-P1ATestDirectory -Parent $temporaryRoot -Name 'legacy-arbitrary'
        Write-P1AUtf8LfFile -Path (Join-Path $arbitraryRoot 'payload.txt') -Text 'payload'
        Write-P1AJsonFile -Path (Join-Path $arbitraryRoot 'evidence.json') -Value ([ordered]@{
                run_id = '20260811T000000000Z-000000000000000000000000'
                status = 'FAIL'
                seal = [ordered]@{
                    coverage_rule = 'all_run_files_except_root_evidence_and_seal'
                }
            })
        $payloadHash = Get-P1ASha256 -Path (Join-Path $arbitraryRoot 'payload.txt')
        Write-P1AUtf8LfFile -Path (Join-Path $arbitraryRoot 'SHA256SUMS') `
            -Text "$payloadHash  payload.txt"
        Assert-P1ATest (-not (Test-P1ASeal -RunRoot $arbitraryRoot)) `
            'arbitrary new legacy failed run passed the compatibility branch'
    }

    $publicationRejections = @(
        [pscustomobject]@{
            id = 'minimal-environment'
            mutate = $null
            minimal_environment = $true
        }
        [pscustomobject]@{
            id = 'minimal-evidence'
            minimal_environment = $false
            mutate = {
                param($Evidence, $Run)
                [ordered]@{
                    schema = 'python-slm-phase-evidence-v1'
                    phase_id = 'P1A'
                    run_id = $Run.run_id
                    status = 'PASS'
                    source_identity_sha256 = $Run.source_hash
                    seal = $Evidence.seal
                }
            }
        }
        [pscustomobject]@{
            id = 'p0-fail'
            minimal_environment = $false
            mutate = { param($Evidence, $Run); $Evidence.p0_dependency.status = 'FAIL'; $Evidence }
        }
        [pscustomobject]@{
            id = 'null-environment'
            minimal_environment = $false
            mutate = { param($Evidence, $Run); $Evidence.environment = $null; $Evidence }
        }
        [pscustomobject]@{
            id = 'not-run-gate'
            minimal_environment = $false
            mutate = { param($Evidence, $Run); $Evidence.gates.quality_gate.status = 'NOT_RUN'; $Evidence }
        }
        [pscustomobject]@{
            id = 'failed-gate'
            minimal_environment = $false
            mutate = { param($Evidence, $Run); $Evidence.gates.cleanup.status = 'FAIL'; $Evidence }
        }
        [pscustomobject]@{
            id = 'nonempty-errors'
            minimal_environment = $false
            mutate = {
                param($Evidence, $Run)
                $Evidence.errors = @([pscustomobject]@{
                        code = 'TEST_FAILURE'
                        category = 1
                        message = 'fixture failure'
                        remediation = 'repair fixture'
                    })
                $Evidence
            }
        }
        [pscustomobject]@{
            id = 'cleanup-false'
            minimal_environment = $false
            mutate = {
                param($Evidence, $Run)
                $Evidence.cleanup.temporary_root_removed = $false
                $Evidence
            }
        }
        [pscustomobject]@{
            id = 'source-mismatch'
            minimal_environment = $false
            mutate = {
                param($Evidence, $Run)
                $Evidence.source_identity_sha256 = 'f' * 64
                $Evidence
            }
        }
        [pscustomobject]@{
            id = 'run-id-mismatch'
            minimal_environment = $false
            mutate = { param($Evidence, $Run); $Evidence.run_id = New-P1ARunId; $Evidence }
        }
        [pscustomobject]@{
            id = 'mode-mismatch'
            minimal_environment = $false
            mutate = { param($Evidence, $Run); $Evidence.mode = 'Cuda'; $Evidence }
        }
        [pscustomobject]@{
            id = 'invocation-mismatch'
            minimal_environment = $false
            mutate = {
                param($Evidence, $Run)
                $Evidence.invocation.argv[-1] = 'docs/receipts/P1B'
                $Evidence
            }
        }
        [pscustomobject]@{
            id = 'command-record-order'
            minimal_environment = $false
            mutate = {
                param($Evidence, $Run)
                $records = @($Evidence.commands)
                [array]::Reverse($records)
                $Evidence.commands = $records
                $Evidence
            }
        }
        [pscustomobject]@{
            id = 'native-command-argv-substitution'
            minimal_environment = $false
            mutate = {
                param($Evidence, $Run)
                $Evidence.commands[3].argv = @('rustc.exe', '--version')
                $Evidence
            }
        }
        [pscustomobject]@{
            id = 'quality-command-argv-substitution'
            minimal_environment = $false
            mutate = {
                param($Evidence, $Run)
                $Evidence.commands[8].argv = @(
                    'cargo.exe', 'check', '--locked', '--all-targets', '--features',
                    'cpu-reference'
                )
                $Evidence
            }
        }
        [pscustomobject]@{
            id = 'transcript-hash-mismatch'
            minimal_environment = $false
            mutate = {
                param($Evidence, $Run)
                $Evidence.commands[0].stdout.sha256 = 'f' * 64
                $Evidence
            }
        }
        [pscustomobject]@{
            id = 'transcript-path-traversal'
            minimal_environment = $false
            mutate = {
                param($Evidence, $Run)
                $probePath = Join-Path $Run.run_root 'probe.txt'
                $Evidence.commands[0].stdout.path = 'commands/../probe.txt'
                $Evidence.commands[0].stdout.sha256 = Get-P1ASha256 -Path $probePath
                $Evidence.commands[0].stdout.bytes = [int64](Get-Item -LiteralPath $probePath).Length
                $Evidence
            }
        }
    )
    foreach ($rejection in $publicationRejections) {
        Invoke-P1ATest "publication rejects $($rejection.id)" {
            $outputRoot = New-P1ATestDirectory -Parent $temporaryRoot `
                -Name ("publication-invalid-" + $rejection.id)
            [void][IO.Directory]::CreateDirectory((Join-Path $outputRoot 'runs'))
            $runParameters = @{
                OutputRoot = $outputRoot
                Status = 'PASS'
                MinimalEnvironment = [bool]$rejection.minimal_environment
            }
            $run = New-P1ATestRun @runParameters
            if ($null -ne $rejection.mutate) {
                $evidence = Get-P1ATestStrictText -Path $run.evidence_path | ConvertFrom-Json
                $evidence = & $rejection.mutate $evidence $run
                Write-P1AJsonFile -Path $run.evidence_path -Value $evidence
            }
            Assert-P1AThrows { [void](Invoke-P1ATestPublication -OutputRoot $outputRoot `
                        -Run $run) } '(?i)(accept|evidence|invalid|pass|gate|source|run|mode|invocation|transcript|environment|p0|cleanup|seal|reference|null|canonical|contained|path)'
            Assert-P1ATest (-not (Test-Path -LiteralPath (Join-Path $outputRoot 'evidence.json'))) `
                "invalid $($rejection.id) run changed the selected pointer"
        }
    }

    $commandIdRejections = @(
        [pscustomobject]@{
            id = 'native-command-id-order'
            mutate = {
                param($Environment)
                $ids = @($Environment.native_probe.command_ids)
                [array]::Reverse($ids)
                $Environment.native_probe.command_ids = $ids
                $Environment
            }
        }
        [pscustomobject]@{
            id = 'quality-command-id-order'
            mutate = {
                param($Environment)
                $ids = @($Environment.quality_gate.command_ids)
                [array]::Reverse($ids)
                $Environment.quality_gate.command_ids = $ids
                $Environment
            }
        }
        [pscustomobject]@{
            id = 'unknown-command-id-reference'
            mutate = {
                param($Environment)
                $ids = @($Environment.quality_gate.command_ids)
                $ids[-1] = 'C99'
                $Environment.quality_gate.command_ids = $ids
                $Environment
            }
        }
    )
    foreach ($rejection in $commandIdRejections) {
        Invoke-P1ATest "publication rejects $($rejection.id)" {
            $outputRoot = New-P1ATestDirectory -Parent $temporaryRoot `
                -Name ("publication-invalid-" + $rejection.id)
            [void][IO.Directory]::CreateDirectory((Join-Path $outputRoot 'runs'))
            $run = New-P1ATestRun -OutputRoot $outputRoot -Status PASS
            Update-P1ATestSealedEnvironment -Run $run -Mutation $rejection.mutate
            Assert-P1AThrows { [void](Invoke-P1ATestPublication -OutputRoot $outputRoot `
                        -Run $run) } '(?i)(command|order|unknown|reference|evidence|environment|accept|invalid)'
            Assert-P1ATest (-not (Test-Path -LiteralPath (Join-Path $outputRoot 'evidence.json'))) `
                "invalid $($rejection.id) run changed the selected pointer"
        }
    }

    Invoke-P1ATest 'publication rejects failed, tampered, and out-of-scope runs' {
        $outputRoot = New-P1ATestDirectory -Parent $temporaryRoot -Name 'publication-reject'
        [void][IO.Directory]::CreateDirectory((Join-Path $outputRoot 'runs'))

        $failed = New-P1ATestRun -OutputRoot $outputRoot -Status FAIL
        Assert-P1AThrows { [void](Invoke-P1ATestPublication -OutputRoot $outputRoot `
                    -Run $failed) } '(?i)(pass|failed|accept)'
        Assert-P1ATest (-not (Test-Path -LiteralPath (Join-Path $outputRoot 'evidence.json'))) `
            'failed run created or replaced the selected pointer'

        $tampered = New-P1ATestRun -OutputRoot $outputRoot -Status PASS
        Write-P1AUtf8LfFile -Path (Join-Path $tampered.run_root 'probe.txt') -Text 'tampered'
        Assert-P1AThrows { [void](Invoke-P1ATestPublication -OutputRoot $outputRoot `
                    -Run $tampered) } '(?i)(seal|invalid|accept)'
        Assert-P1ATest (-not (Test-Path -LiteralPath (Join-Path $outputRoot 'evidence.json'))) `
            'tampered run created or replaced the selected pointer'

        $unsealed = New-P1ATestRun -OutputRoot $outputRoot -Status PASS
        Write-P1AUtf8LfFile -Path (Join-Path $unsealed.run_root 'unsealed-extra.txt') `
            -Text 'not covered by SHA256SUMS'
        Assert-P1AThrows { [void](Invoke-P1ATestPublication -OutputRoot $outputRoot `
                    -Run $unsealed) } '(?i)(seal|invalid|accept)'
        Assert-P1ATest (-not (Test-Path -LiteralPath (Join-Path $outputRoot 'evidence.json'))) `
            'run with an unsealed extra file created or replaced the selected pointer'

        $outsideRoot = Join-Path $outputRoot 'outside'
        $outside = New-P1ATestRun -OutputRoot $outputRoot -Status PASS -RunRoot $outsideRoot
        $outside.run_id = '..\outside'
        Assert-P1AThrows { [void](Invoke-P1ATestPublication -OutputRoot $outputRoot `
                    -Run $outside) } '(?i)(outside|runs|invalid|path|accept)'
        Assert-P1ATest (-not (Test-Path -LiteralPath (Join-Path $outputRoot 'evidence.json'))) `
            'out-of-scope run created or replaced the selected pointer'
    }

    Invoke-P1ATest 'publication creates immutable acceptance and verified pointer' {
        $outputRoot = New-P1ATestDirectory -Parent $temporaryRoot -Name 'publication-pass'
        [void][IO.Directory]::CreateDirectory((Join-Path $outputRoot 'runs'))
        $run = New-P1ATestRun -OutputRoot $outputRoot -Status PASS
        [void](Invoke-P1ATestPublication -OutputRoot $outputRoot -Run $run)

        $pointerPath = Join-Path $outputRoot 'evidence.json'
        Assert-P1ATest (Test-Path -LiteralPath $pointerPath -PathType Leaf) `
            'passing publication did not create the selected pointer'
        $pointer = Get-P1ATestStrictText -Path $pointerPath | ConvertFrom-Json
        $pointerNames = @($pointer.PSObject.Properties.Name | Sort-Object)
        $expectedPointerNames = @(
            'acceptance_path', 'acceptance_sha256', 'phase_id', 'schema', 'updated_at'
        ) | Sort-Object
        Assert-P1AEqual ($pointerNames -join "`n") ($expectedPointerNames -join "`n") `
            'pointer fields do not match the closed schema'
        Assert-P1AEqual $pointer.schema 'python-slm-phase-evidence-pointer-v1' `
            'pointer schema identifier changed'
        Assert-P1AEqual $pointer.phase_id 'P1A' 'pointer selected the wrong phase'
        Assert-P1ATest ($pointer.acceptance_path -match '^acceptances/[0-9]{8}\.json$') `
            'pointer acceptance path is not canonical'

        $acceptancePath = Join-Path $outputRoot $pointer.acceptance_path.Replace('/', '\')
        Assert-P1ATest (Test-P1APathWithin -Path $acceptancePath `
                -Root (Join-Path $outputRoot 'acceptances')) `
            'pointer escaped the acceptance directory'
        Assert-P1AEqual (Get-P1ASha256 -Path $acceptancePath) `
            ([string]$pointer.acceptance_sha256) 'pointer acceptance hash mismatch'
        $acceptance = Get-P1ATestStrictText -Path $acceptancePath | ConvertFrom-Json
        $acceptanceNames = @($acceptance.PSObject.Properties.Name | Sort-Object)
        $expectedAcceptanceNames = @(
            'schema', 'phase_id', 'sequence', 'status', 'acceptance_kind',
            'required_approvals', 'run_path', 'run_evidence_sha256', 'seal_path',
            'seal_sha256', 'environment_path', 'environment_sha256',
            'source_identity_sha256', 'previous_acceptance_sha256', 'created_at'
        ) | Sort-Object
        Assert-P1AEqual ($acceptanceNames -join "`n") ($expectedAcceptanceNames -join "`n") `
            'acceptance fields do not match the closed schema'
        Assert-P1AEqual $acceptance.schema 'python-slm-phase-acceptance-v1' `
            'acceptance schema identifier changed'
        Assert-P1AEqual $acceptance.phase_id 'P1A' 'acceptance recorded the wrong phase'
        Assert-P1AEqual $acceptance.status 'PASS' 'acceptance recorded a non-pass run'
        Assert-P1AEqual $acceptance.acceptance_kind 'automatic_machine_qualification' `
            'acceptance kind changed'
        Assert-P1AEqual @($acceptance.required_approvals).Count 0 `
            'automatic P1A acceptance unexpectedly requires approvals'
        Assert-P1AEqual $acceptance.sequence 1 'first acceptance sequence is not one'
        Assert-P1AEqual $acceptance.run_path "runs/$($run.run_id)" `
            'acceptance selected the wrong run'
        Assert-P1AEqual $acceptance.run_evidence_sha256 `
            (Get-P1ASha256 -Path $run.evidence_path) 'acceptance evidence hash mismatch'
        Assert-P1AEqual $acceptance.seal_sha256 `
            (Get-P1ASha256 -Path (Join-Path $run.run_root 'SHA256SUMS')) `
            'acceptance seal hash mismatch'
        Assert-P1AEqual $acceptance.environment_sha256 `
            (Get-P1ASha256 -Path (Join-Path $run.run_root 'artifacts\environment.json')) `
            'acceptance environment hash mismatch'
        Assert-P1AEqual $acceptance.source_identity_sha256 $run.source_hash `
            'acceptance source identity mismatch'
        Assert-P1ATest ($null -eq $acceptance.previous_acceptance_sha256) `
            'first acceptance unexpectedly has a predecessor'
    }

    Invoke-P1ATest 'failed publication preserves an existing selected acceptance' {
        $outputRoot = New-P1ATestDirectory -Parent $temporaryRoot -Name 'publication-preserve'
        [void][IO.Directory]::CreateDirectory((Join-Path $outputRoot 'runs'))
        $passing = New-P1ATestRun -OutputRoot $outputRoot -Status PASS
        [void](Invoke-P1ATestPublication -OutputRoot $outputRoot -Run $passing)
        $pointerPath = Join-Path $outputRoot 'evidence.json'
        $pointerHash = Get-P1ASha256 -Path $pointerPath
        $acceptancesBefore = @(Get-ChildItem -LiteralPath (Join-Path $outputRoot 'acceptances') `
            -File -Filter '*.json').Count

        $failed = New-P1ATestRun -OutputRoot $outputRoot -Status FAIL
        Assert-P1AThrows { [void](Invoke-P1ATestPublication -OutputRoot $outputRoot `
                    -Run $failed) } '(?i)(pass|failed|accept)'
        Assert-P1AEqual (Get-P1ASha256 -Path $pointerPath) $pointerHash `
            'failed publication replaced the selected pointer'
        Assert-P1AEqual @(Get-ChildItem -LiteralPath (Join-Path $outputRoot 'acceptances') `
                -File -Filter '*.json').Count $acceptancesBefore `
            'failed publication created an acceptance record'
    }

    Invoke-P1ATest 'invalid PASS evidence preserves an existing selected acceptance' {
        $outputRoot = New-P1ATestDirectory -Parent $temporaryRoot -Name 'publication-invalid-preserve'
        [void][IO.Directory]::CreateDirectory((Join-Path $outputRoot 'runs'))
        $first = New-P1ATestRun -OutputRoot $outputRoot -Status PASS
        [void](Invoke-P1ATestPublication -OutputRoot $outputRoot -Run $first)
        $pointerPath = Join-Path $outputRoot 'evidence.json'
        $pointerHash = Get-P1ASha256 -Path $pointerPath
        $acceptancesBefore = @(Get-ChildItem -LiteralPath (Join-Path $outputRoot 'acceptances') `
            -File -Filter '*.json').Count

        $invalid = New-P1ATestRun -OutputRoot $outputRoot -Status PASS
        $evidence = Get-P1ATestStrictText -Path $invalid.evidence_path | ConvertFrom-Json
        $evidence.p0_dependency.status = 'FAIL'
        Write-P1AJsonFile -Path $invalid.evidence_path -Value $evidence
        Assert-P1AThrows { [void](Invoke-P1ATestPublication -OutputRoot $outputRoot `
                    -Run $invalid) } '(?i)(p0|pass|evidence|accept|invalid)'
        Assert-P1AEqual (Get-P1ASha256 -Path $pointerPath) $pointerHash `
            'invalid PASS evidence replaced the selected pointer'
        Assert-P1AEqual @(Get-ChildItem -LiteralPath (Join-Path $outputRoot 'acceptances') `
                -File -Filter '*.json').Count $acceptancesBefore `
            'invalid PASS evidence created an acceptance record'
    }

    Invoke-P1ATest 'publication advances an immutable hash-linked acceptance chain' {
        $outputRoot = New-P1ATestDirectory -Parent $temporaryRoot -Name 'publication-chain'
        [void][IO.Directory]::CreateDirectory((Join-Path $outputRoot 'runs'))
        $first = New-P1ATestRun -OutputRoot $outputRoot -Status PASS
        [void](Invoke-P1ATestPublication -OutputRoot $outputRoot -Run $first)
        $firstPath = Join-Path $outputRoot 'acceptances\00000001.json'
        $firstHash = Get-P1ASha256 -Path $firstPath

        $second = New-P1ATestRun -OutputRoot $outputRoot -Status PASS
        [void](Invoke-P1ATestPublication -OutputRoot $outputRoot -Run $second)
        Assert-P1AEqual (Get-P1ASha256 -Path $firstPath) $firstHash `
            'second publication mutated the first acceptance'
        $secondPath = Join-Path $outputRoot 'acceptances\00000002.json'
        Assert-P1ATest (Test-Path -LiteralPath $secondPath -PathType Leaf) `
            'second publication did not create sequence 00000002'
        $secondAcceptance = Get-P1ATestStrictText -Path $secondPath | ConvertFrom-Json
        Assert-P1AEqual $secondAcceptance.sequence 2 'second acceptance sequence is not two'
        Assert-P1AEqual $secondAcceptance.previous_acceptance_sha256 $firstHash `
            'second acceptance does not hash-link to its predecessor'
        $pointer = Get-P1ATestStrictText -Path (Join-Path $outputRoot 'evidence.json') | ConvertFrom-Json
        Assert-P1AEqual $pointer.acceptance_path 'acceptances/00000002.json' `
            'root pointer did not select the second acceptance'
        Assert-P1AEqual $pointer.acceptance_sha256 (Get-P1ASha256 -Path $secondPath) `
            'root pointer does not hash-pin the second acceptance'
    }

    Invoke-P1ATest 'pointer finalization failure preserves selection and leaves no transient files' {
        $outputRoot = New-P1ATestDirectory -Parent $temporaryRoot -Name 'publication-finalize-failure'
        [void][IO.Directory]::CreateDirectory((Join-Path $outputRoot 'runs'))
        $first = New-P1ATestRun -OutputRoot $outputRoot -Status PASS
        [void](Invoke-P1ATestPublication -OutputRoot $outputRoot -Run $first)
        $pointerPath = Join-Path $outputRoot 'evidence.json'
        $pointerHash = Get-P1ASha256 -Path $pointerPath
        $firstAcceptancePath = Join-Path $outputRoot 'acceptances\00000001.json'
        $firstAcceptanceHash = Get-P1ASha256 -Path $firstAcceptancePath

        $second = New-P1ATestRun -OutputRoot $outputRoot -Status PASS
        $blockingBackup = Join-Path $outputRoot ('.evidence.' + $second.run_id + '.bak')
        [void][IO.Directory]::CreateDirectory($blockingBackup)
        Assert-P1AThrows { [void](Invoke-P1ATestPublication -OutputRoot $outputRoot `
                    -Run $second) } '(?i)(pointer|replace|backup|path|directory|file|exist|access)'
        Assert-P1AEqual (Get-P1ASha256 -Path $pointerPath) $pointerHash `
            'pointer finalization failure changed the prior selection'
        Assert-P1AEqual (Get-P1ASha256 -Path $firstAcceptancePath) $firstAcceptanceHash `
            'pointer finalization failure mutated the prior acceptance'
        $acceptanceFiles = @(Get-ChildItem -LiteralPath (Join-Path $outputRoot 'acceptances') `
            -File -Filter '*.json' | Sort-Object Name)
        Assert-P1ATest ($acceptanceFiles.Count -in @(1, 2)) `
            'pointer finalization failure created an invalid acceptance count'
        if ($acceptanceFiles.Count -eq 2) {
            $orphan = Get-P1ATestStrictText -Path $acceptanceFiles[1].FullName | ConvertFrom-Json
            Assert-P1AEqual $orphan.sequence 2 'orphan acceptance sequence changed'
            Assert-P1AEqual $orphan.previous_acceptance_sha256 $firstAcceptanceHash `
                'orphan acceptance does not extend the prior immutable chain'
        }
        if ([IO.Directory]::Exists($blockingBackup)) {
            [IO.Directory]::Delete($blockingBackup)
        }
        foreach ($suffix in @('.tmp', '.bak', '.failed')) {
            Assert-P1ATest (-not (Test-Path -LiteralPath `
                        (Join-Path $outputRoot ('.evidence.' + $second.run_id + $suffix)))) `
                "pointer finalization failure left a transient $suffix file"
        }
    }

    Invoke-P1ATest 'publication recovers a schema-valid orphan acceptance chain' {
        $outputRoot = New-P1ATestDirectory -Parent $temporaryRoot -Name 'publication-orphan-recovery'
        [void][IO.Directory]::CreateDirectory((Join-Path $outputRoot 'runs'))
        $first = New-P1ATestRun -OutputRoot $outputRoot -Status PASS
        [void](Invoke-P1ATestPublication -OutputRoot $outputRoot -Run $first)
        $pointerPath = Join-Path $outputRoot 'evidence.json'
        $pointerOneText = Get-P1ATestStrictText -Path $pointerPath
        $firstAcceptancePath = Join-Path $outputRoot 'acceptances\00000001.json'
        $firstAcceptanceHash = Get-P1ASha256 -Path $firstAcceptancePath

        $second = New-P1ATestRun -OutputRoot $outputRoot -Status PASS
        [void](Invoke-P1ATestPublication -OutputRoot $outputRoot -Run $second)
        $secondAcceptancePath = Join-Path $outputRoot 'acceptances\00000002.json'
        $secondAcceptanceHash = Get-P1ASha256 -Path $secondAcceptancePath
        Write-P1AUtf8LfFile -Path $pointerPath -Text $pointerOneText

        [void](Invoke-P1ATestPublication -OutputRoot $outputRoot -Run $second)
        Assert-P1AEqual (Get-P1ASha256 -Path $firstAcceptancePath) $firstAcceptanceHash `
            'orphan recovery mutated the first acceptance'
        Assert-P1AEqual (Get-P1ASha256 -Path $secondAcceptancePath) $secondAcceptanceHash `
            'orphan recovery mutated the orphan acceptance'
        $chain = Assert-P1ATestSelectedAcceptanceChain -OutputRoot $outputRoot
        Assert-P1ATest ($chain.count -in @(2, 3)) `
            'orphan recovery created an unexpected number of acceptance records'
        Assert-P1ATest (@(Get-ChildItem -LiteralPath $outputRoot -File |
                Where-Object { $_.Name -match '^\.evidence\..*\.(?:tmp|bak|failed)$' }).Count -eq 0) `
            'orphan recovery left pointer-finalization transients'
    }

    Invoke-P1ATest 'publication rejects a tampered selected pointer without advancing' {
        $outputRoot = New-P1ATestDirectory -Parent $temporaryRoot -Name 'publication-pointer-tamper'
        [void][IO.Directory]::CreateDirectory((Join-Path $outputRoot 'runs'))
        $first = New-P1ATestRun -OutputRoot $outputRoot -Status PASS
        [void](Invoke-P1ATestPublication -OutputRoot $outputRoot -Run $first)
        $pointerPath = Join-Path $outputRoot 'evidence.json'
        $pointer = Get-P1ATestStrictText -Path $pointerPath | ConvertFrom-Json
        $pointer.acceptance_sha256 = '0' * 64
        Write-P1AJsonFile -Path $pointerPath -Value $pointer
        $tamperedPointerHash = Get-P1ASha256 -Path $pointerPath
        $acceptancesBefore = @(Get-ChildItem -LiteralPath (Join-Path $outputRoot 'acceptances') `
            -File -Filter '*.json').Count

        $second = New-P1ATestRun -OutputRoot $outputRoot -Status PASS
        Assert-P1AThrows { [void](Invoke-P1ATestPublication -OutputRoot $outputRoot `
                    -Run $second) } '(?i)(pointer|acceptance|hash|selected|invalid)'
        Assert-P1AEqual (Get-P1ASha256 -Path $pointerPath) $tamperedPointerHash `
            'failed pointer validation replaced the tampered pointer'
        Assert-P1AEqual @(Get-ChildItem -LiteralPath (Join-Path $outputRoot 'acceptances') `
                -File -Filter '*.json').Count $acceptancesBefore `
            'failed pointer validation advanced the acceptance sequence'
    }

    Invoke-P1ATest 'publication rejects a tampered selected acceptance without advancing' {
        $outputRoot = New-P1ATestDirectory -Parent $temporaryRoot -Name 'publication-acceptance-tamper'
        [void][IO.Directory]::CreateDirectory((Join-Path $outputRoot 'runs'))
        $first = New-P1ATestRun -OutputRoot $outputRoot -Status PASS
        [void](Invoke-P1ATestPublication -OutputRoot $outputRoot -Run $first)
        $pointerPath = Join-Path $outputRoot 'evidence.json'
        $pointer = Get-P1ATestStrictText -Path $pointerPath | ConvertFrom-Json
        $acceptancePath = Join-Path $outputRoot $pointer.acceptance_path.Replace('/', '\')
        $acceptance = Get-P1ATestStrictText -Path $acceptancePath | ConvertFrom-Json
        $acceptance.status = 'FAIL'
        Write-P1AJsonFile -Path $acceptancePath -Value $acceptance
        $pointerHash = Get-P1ASha256 -Path $pointerPath
        $acceptancesBefore = @(Get-ChildItem -LiteralPath (Join-Path $outputRoot 'acceptances') `
            -File -Filter '*.json').Count

        $second = New-P1ATestRun -OutputRoot $outputRoot -Status PASS
        Assert-P1AThrows { [void](Invoke-P1ATestPublication -OutputRoot $outputRoot `
                    -Run $second) } '(?i)(pointer|acceptance|hash|selected|invalid)'
        Assert-P1AEqual (Get-P1ASha256 -Path $pointerPath) $pointerHash `
            'failed acceptance validation replaced the selected pointer'
        Assert-P1AEqual @(Get-ChildItem -LiteralPath (Join-Path $outputRoot 'acceptances') `
                -File -Filter '*.json').Count $acceptancesBefore `
            'failed acceptance validation advanced the acceptance sequence'
    }
}
finally {
    if ($null -ne $module -and
        (Get-Module -Name $module.Name -ErrorAction SilentlyContinue)) {
        Remove-Module -Name $module.Name -Force -ErrorAction SilentlyContinue
    }
    if (Test-Path -LiteralPath $temporaryRoot) {
        $resolved = (Resolve-Path -LiteralPath $temporaryRoot).Path
        $leaf = Split-Path -Leaf $resolved
        if (-not $resolved.StartsWith($temporaryBase, [StringComparison]::OrdinalIgnoreCase) -or
            $leaf -notmatch '^python-slm-p1a-tests-[0-9a-f]{32}$') {
            throw "refusing unsafe test cleanup: $resolved"
        }
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}

Write-Output "RESULT passed=$script:Passed failed=$script:Failed"
if ($script:Failures.Count -gt 0) {
    $script:Failures | ForEach-Object { [Console]::Error.WriteLine($_) }
    exit 1
}
exit 0

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

function New-P1BTestToolkit {
    param(
        [Parameter(Mandatory)][string]$Parent,
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][string]$Version,
        [string[]]$Omit = @(),
        [bool]$Explicit = $false
    )

    $root = New-P1ATestDirectory -Parent $Parent -Name $Name
    $versionPath = Join-Path $root 'version.json'
    Write-P1AJsonFile -Path $versionPath -Value ([ordered]@{
            cuda = [ordered]@{ name = 'CUDA SDK'; version = $Version }
        })
    $major = ([version]$Version).Major
    $required = @(
        'bin\nvcc.exe',
        'bin\ptxas.exe',
        'bin\fatbinary.exe',
        'bin\nvlink.exe',
        'bin\cuobjdump.exe',
        'include\cuda.h',
        'include\cuda_runtime.h',
        'include\cublas_v2.h',
        'include\cublasLt.h',
        'lib\x64\cuda.lib',
        'lib\x64\cudart.lib',
        'lib\x64\cublas.lib',
        'lib\x64\cublasLt.lib',
        "bin\x64\cudart64_$major.dll",
        "bin\x64\cublas64_$major.dll",
        "bin\x64\cublasLt64_$major.dll"
    )
    foreach ($relative in $required) {
        if ($Omit -contains $relative) {
            continue
        }
        $path = Join-Path $root $relative
        [void][IO.Directory]::CreateDirectory((Split-Path -Parent $path))
        Write-P1AUtf8LfFile -Path $path -Text "fixture:$relative"
    }
    return [pscustomobject]@{
        root = $root
        source = "fixture:$Name"
        explicit = $Explicit
    }
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

function New-P1BTestOutputRoot {
    param(
        [Parameter(Mandatory)][string]$Parent,
        [Parameter(Mandatory)][string]$Name
    )

    $repository = New-P1ATestDirectory -Parent $Parent -Name $Name
    $outputRoot = Join-Path $repository 'docs\receipts\P1B'
    [void][IO.Directory]::CreateDirectory((Join-Path $outputRoot 'runs'))
    return $outputRoot
}

function New-P1BTestRun {
    param(
        [Parameter(Mandatory)][string]$OutputRoot,
        [Parameter(Mandatory)][ValidateSet('PASS', 'FAIL')][string]$Status
    )

    $runId = New-P1ARunId
    $runRoot = Join-Path (Join-Path $OutputRoot 'runs') $runId
    $artifactsRoot = Join-Path $runRoot 'artifacts'
    $commandsRoot = Join-Path $runRoot 'commands'
    [void][IO.Directory]::CreateDirectory($artifactsRoot)
    [void][IO.Directory]::CreateDirectory($commandsRoot)
    $shaA = 'a' * 64
    $shaB = 'b' * 64
    $shaC = 'c' * 64
    $shaD = 'd' * 64
    $head = 'a' * 40
    $p0 = [ordered]@{
        status = 'PASS'
        receipt_commit = '86fb1e4cc68efeb651e5362c4aca85c2827d8e4d'
        receipt_sha256 = $shaA
        contract_sha256 = $shaB
        decision_ledger_sha256 = $shaC
    }
    $p1a = [ordered]@{
        status = 'PASS'
        pointer_path = 'docs/receipts/P1A/evidence.json'
        pointer_sha256 = $shaA
        acceptance_path = 'docs/receipts/P1A/acceptances/00000001.json'
        acceptance_sha256 = $shaB
        acceptance_sequence = 1
        run_path = 'docs/receipts/P1A/runs/20260811T142631436Z-a1936b8e585f4bcb905b3c3e'
        run_evidence_sha256 = $shaC
        seal_path = 'docs/receipts/P1A/runs/20260811T142631436Z-a1936b8e585f4bcb905b3c3e/SHA256SUMS'
        seal_sha256 = $shaD
        environment_path = 'docs/receipts/P1A/runs/20260811T142631436Z-a1936b8e585f4bcb905b3c3e/artifacts/environment.json'
        environment_sha256 = $shaA
        source_identity_sha256 = $shaB
        verifier_sha256 = $shaC
        schema_bundle_sha256 = $shaD
        review_closure_commit = '9359c989fa63d4a300abc509e735b7e81a24a2ea'
    }
    $sourcePath = Join-Path $artifactsRoot 'source-identity.json'
    Write-P1AJsonFile -Path $sourcePath -Value ([ordered]@{
            schema = 'test-source-identity-v1'
            head = $head
            input_manifest_sha256 = $shaD
        })
    $sourceHash = Get-P1ASha256 -Path $sourcePath

    $newTool = {
        param([string]$Name, [string]$Path)
        [ordered]@{ name = $Name; version = '1.0'; path = $Path; sha256 = $shaA }
    }
    $newComponent = {
        param([string]$Name, [string]$Path)
        [ordered]@{ name = $Name; path = $Path; sha256 = $shaB; bytes = 1 }
    }
    $peImports = @(
        'KERNEL32.dll', 'nvcuda.dll', 'cudart64_13.dll',
        'cublas64_13.dll', 'cublasLt64_13.dll'
    )
    $newArtifactInspection = {
        param([bool]$Mixed)
        $sassTargets = [Collections.Generic.List[string]]::new()
        if ($Mixed) { [void]$sassTargets.Add('sm_120') }
        [ordered]@{
            compile_command_id = 'C01'
            executable_sha256 = $shaC
            run_command_id = 'C01'
            pe_headers_command_id = 'C01'
            pe_dependents_command_id = 'C01'
            elf_command_id = 'C01'
            ptx_list_command_id = 'C01'
            sass_dump_command_id = 'C01'
            ptx_dump_command_id = 'C01'
            embedded_sass = @($sassTargets)
            embedded_ptx = @('compute_120')
            pe_imports = $peImports
            result = 'PASS'
        }
    }
    $absentOptional = {
        [ordered]@{ present = $false; version = $null; path = $null; sha256 = $null }
    }
    $environmentPath = Join-Path $artifactsRoot 'environment.json'
    $environment = [ordered]@{
            schema = 'python-slm-cuda-environment-manifest-v1'
            phase_id = 'P1B'
            run_id = $runId
            mode = 'Cuda'
            status = $Status
            p0 = [ordered]@{
                receipt_commit = $p0.receipt_commit
                receipt_sha256 = $p0.receipt_sha256
                contract_sha256 = $p0.contract_sha256
                decision_ledger_sha256 = $p0.decision_ledger_sha256
            }
            p1a = $p1a
            source = [ordered]@{
                head = $head
                dirty = $false
                input_manifest_sha256 = $shaD
                cargo_lock_sha256 = $shaA
                verifier_sha256 = $shaC
                schema_bundle_sha256 = $shaD
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
                commit_hash = $head
                commit_date = '2026-05-25'
                host = 'x86_64-pc-windows-msvc'
                llvm_version = '22.1.2'
                rustc = & $newTool 'rustc' '${CARGO_HOME}/bin/rustc.exe'
                cargo_version = '1.96.0'
                cargo = & $newTool 'cargo' '${CARGO_HOME}/bin/cargo.exe'
                rustup = $null
            }
            visual_studio = [ordered]@{
                discovery_method = 'vswhere'
                query = @(
                    '-version', '[17.0,18.0)', '-products', '*', '-requires',
                    'Microsoft.VisualStudio.Component.VC.Tools.x86.x64',
                    '-format', 'json', '-utf8'
                )
                instance_id = 'fixture-vs2022'
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
                    (& $newTool 'cl' '${VC_TOOLS}/bin/Hostx64/x64/cl.exe')
                    (& $newTool 'link' '${VC_TOOLS}/bin/Hostx64/x64/link.exe')
                    (& $newTool 'lib' '${VC_TOOLS}/bin/Hostx64/x64/lib.exe')
                    (& $newTool 'dumpbin' '${VC_TOOLS}/bin/Hostx64/x64/dumpbin.exe')
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
                    (& $newTool 'rc' '${WINDOWS_KITS}/bin/10.0.26100.0/x64/rc.exe')
                    (& $newTool 'mt' '${WINDOWS_KITS}/bin/10.0.26100.0/x64/mt.exe')
                )
            }
            cuda_toolkit = [ordered]@{
                version = '13.1.80'
                root = '${CUDA_TOOLKIT}'
                tools = @(
                    (& $newTool 'nvcc' '${CUDA_TOOLKIT}/bin/nvcc.exe')
                    (& $newTool 'ptxas' '${CUDA_TOOLKIT}/bin/ptxas.exe')
                    (& $newTool 'fatbinary' '${CUDA_TOOLKIT}/bin/fatbinary.exe')
                    (& $newTool 'nvlink' '${CUDA_TOOLKIT}/bin/nvlink.exe')
                    (& $newTool 'cuobjdump' '${CUDA_TOOLKIT}/bin/cuobjdump.exe')
                )
                headers = @(
                    (& $newComponent 'cuda.h' '${CUDA_TOOLKIT}/include/cuda.h')
                    (& $newComponent 'cuda_runtime.h' '${CUDA_TOOLKIT}/include/cuda_runtime.h')
                    (& $newComponent 'cublas_v2.h' '${CUDA_TOOLKIT}/include/cublas_v2.h')
                    (& $newComponent 'cublasLt.h' '${CUDA_TOOLKIT}/include/cublasLt.h')
                )
                libraries = @(
                    (& $newComponent 'cuda.lib' '${CUDA_TOOLKIT}/lib/x64/cuda.lib')
                    (& $newComponent 'cudart.lib' '${CUDA_TOOLKIT}/lib/x64/cudart.lib')
                    (& $newComponent 'cublas.lib' '${CUDA_TOOLKIT}/lib/x64/cublas.lib')
                    (& $newComponent 'cublasLt.lib' '${CUDA_TOOLKIT}/lib/x64/cublasLt.lib')
                )
                runtime_dlls = @(
                    (& $newComponent 'cudart64_13.dll' '${CUDA_TOOLKIT}/bin/x64/cudart64_13.dll')
                    (& $newComponent 'cublas64_13.dll' '${CUDA_TOOLKIT}/bin/x64/cublas64_13.dll')
                    (& $newComponent 'cublasLt64_13.dll' '${CUDA_TOOLKIT}/bin/x64/cublasLt64_13.dll')
                )
            }
            runtime_linkage = [ordered]@{
                mode = 'hybrid'
                active_provider = 'display_driver'
                staged_runtime_dlls = @(
                    [ordered]@{ component = 'cublas'; sha256 = $shaB }
                    [ordered]@{ component = 'cublaslt'; sha256 = $shaB }
                )
            }
            optional_components = [ordered]@{
                cudnn = & $absentOptional
                nvrtc = & $absentOptional
                nvjitlink = & $absentOptional
                compute_sanitizer = & $absentOptional
                curand = & $absentOptional
            }
            driver = [ordered]@{
                driver_version = '610.88'
                cuda_umd_version = '13.1'
                nvidia_smi = & $newTool 'nvidia-smi' '${WINDOWS}/System32/nvidia-smi.exe'
                library = & $newTool 'nvcuda' '${WINDOWS}/System32/nvcuda.dll'
            }
            gpu = [ordered]@{
                index = 0
                name = 'NVIDIA GeForce RTX 5090'
                memory_total_bytes = [int64]32607 * 1MB
                compute_capability_major = 12
                compute_capability_minor = 0
            }
            architecture_targets = [ordered]@{
                sass = @('sm_120')
                ptx = @('compute_120')
                advertised_architectures = @('compute_120')
                advertised_code = @('sm_120')
                mixed = & $newArtifactInspection $true
                ptx_only = & $newArtifactInspection $false
            }
            device_probe = [ordered]@{
                source_sha256 = $shaD
                runtime_version = 13010
                driver_version = 13010
                cublas_version = 130100
                cublaslt_version = 130100
                mixed_result = 'PASS'
                ptx_only_result = 'PASS'
                sentinel = 42
                target_match_count = 1
            }
            isolation = [ordered]@{
                temporary_root_absent_before = $true
                temporary_root_outside_repository = $true
                python_canaries = @('python.exe')
                canary_hits = @()
                python_invoked = $false
            }
            cleanup = [ordered]@{
                temporary_root_removed = $true
                repository_target_unchanged = $true
                parent_environment_unchanged = $true
                inputs_unchanged = $true
            }
        }

    $fixtureRepository = $runRoot
    for ($index = 0; $index -lt 5; $index++) {
        $fixtureRepository = Split-Path -Parent $fixtureRepository
    }
    $p1aEnvironmentPath = Join-Path $fixtureRepository `
        ($p1a.environment_path.Replace('/', '\'))
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $p1aEnvironmentPath))
    Write-P1AJsonFile -Path $p1aEnvironmentPath -Value ([ordered]@{
            host = $environment.host
            rust = $environment.rust
            visual_studio = $environment.visual_studio
            windows_sdk = $environment.windows_sdk
        })
    $p1a.environment_sha256 = Get-P1ASha256 -Path $p1aEnvironmentPath
    Write-P1AJsonFile -Path $environmentPath -Value $environment

    $stdoutPath = Join-Path $commandsRoot 'C01.stdout.txt'
    $stderrPath = Join-Path $commandsRoot 'C01.stderr.txt'
    Write-P1AUtf8LfFile -Path $stdoutPath -Text 'fixture pass'
    Write-P1AUtf8LfFile -Path $stderrPath -Text ''
    $commands = @([ordered]@{
            id = 'C01'
            argv = @(
                'nvcc.exe', '--cudart=hybrid',
                '--linker-options=/WX,/NODEFAULTLIB:LIBCMT',
                '-o', 'fixture.exe'
            )
            cwd = '${REPO}'
            environment_delta_names = @('CUDA_CACHE_PATH')
            configuration_sha256 = $shaA
            exit_code = if ($Status -ceq 'PASS') { 0 } else { 5 }
            duration_ms = 1
            status = $Status
            stdout = [ordered]@{
                path = 'commands/C01.stdout.txt'
                sha256 = Get-P1ASha256 -Path $stdoutPath
                bytes = [int64](Get-Item -LiteralPath $stdoutPath).Length
            }
            stderr = [ordered]@{
                path = 'commands/C01.stderr.txt'
                sha256 = Get-P1ASha256 -Path $stderrPath
                bytes = [int64](Get-Item -LiteralPath $stderrPath).Length
            }
        })
    $gates = [ordered]@{}
    foreach ($gateName in @(
            'p0', 'p1a', 'cpu_regression', 'input_stability', 'toolchain',
            'cuda_toolkit', 'architecture_targets', 'gpu_identity', 'driver_runtime',
            'device_probe', 'python_isolation', 'redaction', 'cleanup'
        )) {
        $gates[$gateName] = [ordered]@{
            status = if ($Status -ceq 'PASS') { 'PASS' } else { 'FAIL' }
            detail = if ($Status -ceq 'PASS') { 'fixture passed' } else { 'fixture failed' }
        }
    }
    $runErrors = [Collections.Generic.List[object]]::new()
    if ($Status -ceq 'FAIL') {
        [void]$runErrors.Add([ordered]@{
                code = 'FIXTURE_FAILURE'
                category = 5
                message = 'fixture failed'
                remediation = 'repair fixture'
            })
    }
    $evidencePath = Join-Path $runRoot 'evidence.json'
    $futureSealEntries = @(Get-ChildItem -LiteralPath $runRoot -Recurse -File).Count + 1
    Write-P1AJsonFile -Path $evidencePath -Value ([ordered]@{
            schema = 'python-slm-phase-evidence-v2'
            phase_id = 'P1B'
            run_id = $runId
            mode = 'Cuda'
            status = $Status
            started_at = '2026-08-11T00:00:00.0000000Z'
            finished_at = '2026-08-11T00:00:00.0010000Z'
            duration_ms = 1
            invocation = [ordered]@{
                argv = @(
                    'powershell', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
                    'scripts/verify-env.ps1', '-Mode', 'Cuda', '-OutputRoot',
                    'docs/receipts/P1B'
                )
                cwd = '${REPO}'
            }
            source_identity_sha256 = $sourceHash
            p0_dependency = $p0
            p1a_dependency = $p1a
            environment = [ordered]@{
                path = 'artifacts/environment.json'
                sha256 = Get-P1ASha256 -Path $environmentPath
                bytes = [int64](Get-Item -LiteralPath $environmentPath).Length
            }
            commands = $commands
            gates = $gates
            errors = @($runErrors)
            cleanup = [ordered]@{
                attempted = $true
                temporary_root_removed = $true
            }
            seal = [ordered]@{
                path = 'SHA256SUMS'
                entries = $futureSealEntries
                coverage_rule = 'all_run_files_except_seal'
            }
        })
    [void](New-P1ASeal -RunRoot $runRoot)
    return [pscustomobject][ordered]@{
        run_id = $runId
        run_root = $runRoot
        evidence_path = $evidencePath
        environment_path = $environmentPath
        source_hash = $sourceHash
    }
}

function Invoke-P1BTestPublication {
    param(
        [Parameter(Mandatory)][string]$OutputRoot,
        [Parameter(Mandatory)]$Run
    )

    return Publish-P1BAcceptance -OutputRoot $OutputRoot -RunId $Run.run_id `
        -SourceIdentitySha256 $Run.source_hash
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

function Assert-P1BTestSelectedAcceptanceChain {
    param([Parameter(Mandatory)][string]$OutputRoot)

    $acceptanceRoot = Join-Path $OutputRoot 'acceptances'
    $files = @(Get-ChildItem -LiteralPath $acceptanceRoot -File -Filter '*.json' |
        Sort-Object Name)
    Assert-P1ATest ($files.Count -ge 1) 'P1B acceptance chain is empty'
    $previousHash = $null
    foreach ($index in 0..($files.Count - 1)) {
        $sequence = $index + 1
        Assert-P1AEqual $files[$index].Name ($sequence.ToString('00000000') + '.json') `
            'P1B acceptance chain has a gap or noncanonical name'
        $acceptance = Get-P1ATestStrictText -Path $files[$index].FullName | ConvertFrom-Json
        Assert-P1AEqual $acceptance.schema 'python-slm-phase-acceptance-v2' `
            'P1B acceptance chain contains the wrong schema'
        Assert-P1AEqual $acceptance.phase_id 'P1B' `
            'P1B acceptance chain contains the wrong phase'
        Assert-P1AEqual $acceptance.sequence $sequence `
            'P1B acceptance sequence does not match its filename'
        Assert-P1AEqual $acceptance.status 'PASS' `
            'P1B acceptance chain contains a non-PASS record'
        Assert-P1AEqual $acceptance.acceptance_kind 'automatic_machine_qualification' `
            'P1B acceptance kind changed'
        Assert-P1AEqual @($acceptance.required_approvals).Count 0 `
            'automatic P1B acceptance unexpectedly requires approvals'
        Assert-P1AEqual ([string]$acceptance.previous_acceptance_sha256) `
            ([string]$previousHash) 'P1B acceptance predecessor hash mismatch'
        $previousHash = Get-P1ASha256 -Path $files[$index].FullName
    }

    $pointerPath = Join-Path $OutputRoot 'evidence.json'
    Assert-P1ATest (Test-Path -LiteralPath $pointerPath -PathType Leaf) `
        'P1B selected pointer is missing'
    $pointer = Get-P1ATestStrictText -Path $pointerPath | ConvertFrom-Json
    Assert-P1AEqual $pointer.schema 'python-slm-phase-evidence-pointer-v2' `
        'P1B selected pointer schema changed'
    Assert-P1AEqual $pointer.phase_id 'P1B' 'P1B selected pointer phase changed'
    Assert-P1AEqual $pointer.acceptance_path ('acceptances/' + $files[-1].Name) `
        'P1B selected pointer does not identify the latest acceptance'
    Assert-P1AEqual $pointer.acceptance_sha256 $previousHash `
        'P1B selected pointer does not hash-pin the latest acceptance'
    return [pscustomobject]@{ count = $files.Count; latest_hash = $previousHash }
}

$moduleFullPath = [IO.Path]::GetFullPath($ModulePath)
$entrypointFullPath = [IO.Path]::GetFullPath($EntrypointPath)
$p1bImplementationFullPath = Join-Path (Split-Path -Parent $moduleFullPath) 'VerifyCuda.ps1'
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
        Assert-P1ATest (Test-Path -LiteralPath $p1bImplementationFullPath -PathType Leaf) `
            "missing concrete CUDA verifier: $p1bImplementationFullPath"

        $asts = @(
            Get-P1ATestAst -Path $moduleFullPath
            Get-P1ATestAst -Path $entrypointFullPath
            Get-P1ATestAst -Path $p1bImplementationFullPath
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
            (Get-P1ATestStrictText $entrypointFullPath) + "`n" +
            (Get-P1ATestStrictText $p1bImplementationFullPath)
        Assert-P1ATest ($source -notmatch '(?im)^\s*#requires\s+-modules?\s+') `
            'verifier must not require external PowerShell modules'
        Assert-P1ATest ($source -notmatch '(?i)\bImport-Module\s+(?:Pester|powershell-yaml|ajson)\b') `
            'verifier imports a forbidden external module'
        Assert-P1ATest ($source -notmatch '(?i)SetEnvironmentVariable\s*\([^\r\n]*["''](?:User|Machine)["'']') `
            'verifier mutates user- or machine-scoped environment variables'
    }

    Invoke-P1ATest 'P1B schemas are closed JSON and P1A schema identities are unchanged' {
        $schemaExpectations = [ordered]@{
            'python-slm-cuda-environment-manifest-v1.schema.json' = `
                'python-slm-cuda-environment-manifest-v1'
            'python-slm-phase-evidence-v2.schema.json' = 'python-slm-phase-evidence-v2'
            'python-slm-phase-acceptance-v2.schema.json' = 'python-slm-phase-acceptance-v2'
            'python-slm-phase-evidence-pointer-v2.schema.json' = `
                'python-slm-phase-evidence-pointer-v2'
        }
        foreach ($entry in $schemaExpectations.GetEnumerator()) {
            $path = Join-Path $repositoryRoot ('docs\schemas\' + [string]$entry.Key)
            Assert-P1ATest (Test-Path -LiteralPath $path -PathType Leaf) `
                "missing P1B schema: $($entry.Key)"
            $text = Get-P1ATestStrictText -Path $path
            Assert-P1ATest ($text -notmatch "`r") `
                "P1B schema is not LF-only: $($entry.Key)"
            $schema = $text | ConvertFrom-Json
            Assert-P1AEqual $schema.type 'object' `
                "P1B schema root is not an object: $($entry.Key)"
            Assert-P1AEqual $schema.additionalProperties $false `
                "P1B schema root is not closed: $($entry.Key)"
            Assert-P1AEqual $schema.properties.schema.const ([string]$entry.Value) `
                "P1B schema identity changed: $($entry.Key)"
        }

        $p1aSchemaHashes = [ordered]@{
            'python-slm-environment-manifest-v1.schema.json' = `
                '6f6f8ca688dc680d6fa8d9c35484f361cb08c4982d005ebe417ce16a0b6be312'
            'python-slm-phase-evidence-v1.schema.json' = `
                '1274a12b561c71072900fc39f67f911741dfd84bcc9faa6a45fce019f8f7a666'
            'python-slm-phase-acceptance-v1.schema.json' = `
                '4248418de110ef6bdaa793416a61bdb7949bfd348dd0b4da75024567c4aff5b8'
            'python-slm-phase-evidence-pointer-v1.schema.json' = `
                '0e4919ff49d94be2690c0a3ff70516947ba96bb13dc6c6cfc05cdcc48532354d'
        }
        foreach ($entry in $p1aSchemaHashes.GetEnumerator()) {
            $path = Join-Path $repositoryRoot ('docs\schemas\' + [string]$entry.Key)
            $actualHash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
            Assert-P1AEqual $actualHash ([string]$entry.Value) `
                "published P1A schema changed: $($entry.Key)"
        }
    }

    $module = Import-Module -Name $moduleFullPath -Force -PassThru

    Invoke-P1ATest 'module exports the stable P1A and P1B helper surface only' {
        $expected = @(
            'Assert-P1BProbeResult',
            'ConvertTo-P1ACommandLine',
            'ConvertTo-P1ANormalizedPath',
            'ConvertFrom-P1BNvidiaSmi',
            'ConvertFrom-P1BNvccVersion',
            'Get-P1ASha256',
            'Get-P1BNvccTargets',
            'Get-P1BSelectedP1ADependency',
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
            'Publish-P1BAcceptance',
            'Select-P1BCudaToolkit',
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

    Invoke-P1ATest 'Cuda mode dispatches to one concrete P1B verifier implementation' {
        $ast = Get-P1ATestAst -Path $p1bImplementationFullPath
        $definitions = @($ast.FindAll({
                    param($node)
                    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and
                    $node.Name -ceq 'Invoke-P1BVerification'
                }, $true))
        Assert-P1AEqual $definitions.Count 1 `
            'module must define exactly one concrete Invoke-P1BVerification function'
        $source = Get-P1ATestStrictText -Path $moduleFullPath
        Assert-P1ATest ($source -match `
                '(?m)^\.\s*\(Join-Path\s+\$PSScriptRoot\s+''VerifyCuda\.ps1''\)\s*$') `
            'shared verifier does not dot-source the bounded CUDA implementation'
        Assert-P1ATest ($source -match `
                '(?s)if\s*\(\$Mode\s+-ceq\s+''Cuda''\).*?Invoke-P1BVerification') `
            'Cuda mode does not dispatch to the concrete P1B verifier'
        Assert-P1ATest ($source -notmatch `
                '(?s)if\s*\(\$Mode\s+-ceq\s+''Cuda''\).*?New-P1AModeNotImplementedRun') `
            'Cuda mode still dispatches to the Phase 1A placeholder'
    }

    Invoke-P1ATest 'concrete P1B verifier contains the ordered bounded CUDA qualification flow' {
        $source = Get-P1ATestStrictText -Path $p1bImplementationFullPath
        Assert-P1ATest ($source -notmatch 'CUDA_QUALIFICATION_INCOMPLETE') `
            'concrete P1B verifier is still an unconditional failure stub'
        $requiredFragments = @(
            'Get-P1BSelectedP1ADependency',
            'Select-P1BCudaToolkit',
            'ConvertFrom-P1BNvccVersion',
            'Get-P1BNvccTargets',
            'ConvertFrom-P1BNvidiaSmi',
            'New-P1BNvccArguments -Kind Mixed',
            'New-P1BNvccArguments -Kind PtxOnly',
            'Invoke-P1BArtifactInspectionCommands -Kind Mixed',
            'Invoke-P1BArtifactInspectionCommands -Kind PtxOnly',
            'Assert-P1BArtifactInspection -Kind $Kind',
            'Assert-P1BPeInspection',
            'Assert-P1BProbeResult',
            '[IO.File]::Copy($canaryExecutable',
            'source_sha256 = Get-P1ASha256 -Path $probePath',
            'Get-P1AInputManifest',
            'python-slm-cuda-environment-manifest-v1',
            'Publish-P1BAcceptance'
        )
        foreach ($fragment in $requiredFragments) {
            Assert-P1ATest ($source.Contains($fragment)) `
                "concrete P1B verifier omitted required stage: $fragment"
        }
        foreach ($argument in @(
                '--version', '--list-gpu-arch', '--list-gpu-code',
                '--query-gpu=', '--list-elf', '--list-ptx', '--dump-sass',
                '--dump-ptx', '/HEADERS', '/DEPENDENTS'
            )) {
            Assert-P1ATest ($source.Contains($argument)) `
                "concrete P1B verifier omitted required command argument: $argument"
        }
        Assert-P1ATest ($source -match `
                '(?s)p1a\.verifier_sha256.*?p1a\.schema_bundle_sha256') `
            'P1B does not bind current verifier/schema bundles to its selected P1A regression'
        Assert-P1ATest ($source -match `
                '(?s)Set-P1AGate\s+\$Context\.Gates\s+cpu_regression\s+PASS') `
            'P1B never records a passing CPU regression gate'

        $ast = Get-P1ATestAst -Path $p1bImplementationFullPath
        $functions = @($ast.FindAll({
                    param($node)
                    $node -is [Management.Automation.Language.FunctionDefinitionAst]
                }, $true))
        $entrypoint = @($functions | Where-Object { $_.Name -ceq 'Invoke-P1BVerification' })
        $qualification = @($functions | Where-Object {
                $_.Name -ceq 'Invoke-P1BQualificationStages'
            })
        $inspection = @($functions | Where-Object {
                $_.Name -ceq 'Invoke-P1BArtifactInspectionCommands'
            })
        Assert-P1AEqual $entrypoint.Count 1 'P1B entrypoint definition is ambiguous'
        Assert-P1AEqual $qualification.Count 1 'P1B qualification-stage definition is ambiguous'
        Assert-P1AEqual $inspection.Count 1 'P1B artifact-inspection definition is ambiguous'

        $inspectionSource = $inspection[0].Extent.Text
        Assert-P1ATest ($inspectionSource -match `
                '(?s)--dump-sass.*?-RecordDigestOnly.*?--dump-ptx.*?-RecordDigestOnly') `
            'P1B retains raw SASS or PTX dump transcripts instead of digest-only evidence'

        $entrypointSource = $entrypoint[0].Extent.Text
        $entrypointOrder = @(
            'Test-P1AP0Dependency',
            'Get-P1BSelectedP1ADependency',
            'Invoke-P1BQualificationStages',
            'Publish-P1BAcceptance'
        )
        $previous = -1
        foreach ($fragment in $entrypointOrder) {
            $index = $entrypointSource.IndexOf($fragment, [StringComparison]::Ordinal)
            Assert-P1ATest ($index -gt $previous) `
                "P1B entrypoint stage is missing or out of order: $fragment"
            $previous = $index
        }

        $qualificationSource = $qualification[0].Extent.Text
        $qualificationOrder = @(
            'Select-P1BCudaToolkit',
            'ConvertFrom-P1BNvccVersion',
            'Get-P1BNvccTargets',
            'ConvertFrom-P1BNvidiaSmi',
            'New-P1BNvccArguments -Kind Mixed',
            'New-P1BNvccArguments -Kind PtxOnly',
            'Invoke-P1BArtifactInspectionCommands -Kind Mixed',
            'Invoke-P1BArtifactInspectionCommands -Kind PtxOnly',
            '-FilePath $mixedExe',
            'Assert-P1BProbeResult -Value ($mixedRun.result.stdout',
            '-FilePath $ptxExe',
            'Assert-P1BProbeResult -Value ($ptxRun.result.stdout',
            'python-slm-cuda-environment-manifest-v1'
        )
        $previous = -1
        foreach ($fragment in $qualificationOrder) {
            $index = $qualificationSource.IndexOf($fragment, $previous + 1,
                [StringComparison]::Ordinal)
            Assert-P1ATest ($index -gt $previous) `
                "P1B qualification stage is missing or out of order: $fragment"
            $previous = $index
        }

        $warningGate = [regex]::Match($qualificationSource,
            '(?s)foreach\s*\(\$compile\s+in\s+@\(\$mixedCompile,\s*\$ptxCompile\)\).*?CUDA_COMPILE_WARNING')
        Assert-P1ATest $warningGate.Success `
            'P1B does not apply one warning gate to both CUDA compilations'
        foreach ($stream in @('$compile.result.stdout', '$compile.result.stderr')) {
            Assert-P1ATest ($warningGate.Value.Contains($stream)) `
                "P1B CUDA compile warning gate omits $stream"
        }
        Assert-P1ATest ($warningGate.Value -match '\\bwarning\\b') `
            'P1B CUDA compile warning gate does not reject warning text'
    }

    Invoke-P1ATest 'P1A regression manifest binds the complete P1B verifier bundle' {
        $manifest = & $module {
            param($Repository, $Entrypoint)
            Get-P1AInputManifest -RepositoryRoot $Repository -ScriptPath $Entrypoint
        } $repositoryRoot $entrypointFullPath
        $paths = @($manifest.entries | ForEach-Object { [string]$_.path })
        foreach ($required in @(
                'scripts/verify-env.ps1',
                'scripts/lib/VerifyEnv.psm1',
                'scripts/lib/VerifyCuda.ps1',
                'scripts/tests/verify-env.tests.ps1',
                'docs/schemas/python-slm-cuda-environment-manifest-v1.schema.json',
                'docs/schemas/python-slm-phase-evidence-v2.schema.json',
                'docs/schemas/python-slm-phase-acceptance-v2.schema.json',
                'docs/schemas/python-slm-phase-evidence-pointer-v2.schema.json'
            )) {
            Assert-P1ATest ($paths -contains $required) `
                "P1A regression input manifest omitted $required"
        }
        Assert-P1ATest (@($paths | Where-Object {
                    $_ -match '^docs/receipts/P1[AB]/'
                }).Count -eq 0) `
            'P1A regression input manifest included mutable receipt output'
        Assert-P1ATest ([string]$manifest.sha256 -match '^[0-9a-f]{64}$') `
            'P1A regression input manifest hash is not canonical'
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

    Invoke-P1ATest 'nvcc parser accepts canonical versions and rejects malformed identity' {
        $fixtures = @(
            [pscustomobject]@{ text = @'
nvcc: NVIDIA (R) Cuda compiler driver
Copyright (c) 2005-2026 NVIDIA Corporation
Built on Thu_May_21_12:00:00_Pacific_Daylight_Time_2026
Cuda compilation tools, release 12.8, V12.8.93
Build cuda_12.8.r12.8/compiler.35583870_0
'@; toolkit_release = '12.8'; compiler_version = '12.8.93'; major = 12; minor = 8; build = 93 }
            [pscustomobject]@{ text = @'
nvcc: NVIDIA (R) Cuda compiler driver
Cuda compilation tools, release 12.9, V12.9.41
'@; toolkit_release = '12.9'; compiler_version = '12.9.41'; major = 12; minor = 9; build = 41 }
            [pscustomobject]@{ text = @'
nvcc: NVIDIA (R) Cuda compiler driver
Cuda compilation tools, release 12.10, V12.10.7
'@; toolkit_release = '12.10'; compiler_version = '12.10.7'; major = 12; minor = 10; build = 7 }
            [pscustomobject]@{ text = @'
nvcc: NVIDIA (R) Cuda compiler driver
Cuda compilation tools, release 13.1, V13.1.80
'@; toolkit_release = '13.1'; compiler_version = '13.1.80'; major = 13; minor = 1; build = 80 }
        )
        foreach ($fixture in $fixtures) {
            $parsed = ConvertFrom-P1BNvccVersion -Text $fixture.text
            Assert-P1AEqual $parsed.toolkit_release $fixture.toolkit_release `
                'nvcc toolkit release parsed incorrectly'
            Assert-P1AEqual $parsed.compiler_version $fixture.compiler_version `
                'nvcc compiler version parsed incorrectly'
            Assert-P1AEqual $parsed.major $fixture.major 'nvcc major parsed incorrectly'
            Assert-P1AEqual $parsed.minor $fixture.minor 'nvcc minor parsed incorrectly'
            Assert-P1AEqual $parsed.build $fixture.build 'nvcc build parsed incorrectly'
        }
        Assert-P1AThrows { ConvertFrom-P1BNvccVersion -Text `
                'Cuda compilation tools, release 13.1' } '(?i)(nvcc|version|parse|V13\.1)'
        Assert-P1AThrows { ConvertFrom-P1BNvccVersion -Text `
                'Cuda compilation tools, release 13.1, V12.9.41' } `
            '(?i)(nvcc|version|mismatch|parse)'
        Assert-P1AThrows { ConvertFrom-P1BNvccVersion -Text '' } `
            '(?i)(empty|nvcc|version|parse)'
    }

    Invoke-P1ATest 'nvcc target parser requires exact Blackwell SASS and PTX targets' {
        $valid = Get-P1BNvccTargets `
            -ArchitectureText "compute_75`ncompute_90`ncompute_120`n" `
            -CodeText "sm_75`nsm_90`nsm_120`n"
        Assert-P1AEqual ($valid.architectures -join ',') `
            'compute_120,compute_75,compute_90' 'nvcc architecture targets changed'
        Assert-P1AEqual ($valid.code -join ',') 'sm_120,sm_75,sm_90' `
            'nvcc code targets changed'
        Assert-P1AEqual ($valid.sass -join ',') 'sm_120' 'required native target changed'
        Assert-P1AEqual ($valid.ptx -join ',') 'compute_120' 'required PTX target changed'

        Assert-P1AThrows { Get-P1BNvccTargets `
                -ArchitectureText "compute_90`ncompute_120" `
                -CodeText "sm_90`n" } '(?i)(sm_120|SASS|target)'
        Assert-P1AThrows { Get-P1BNvccTargets `
                -ArchitectureText "compute_90`n" `
                -CodeText "sm_90`nsm_120" } '(?i)(compute_120|PTX|target)'
        Assert-P1AThrows { Get-P1BNvccTargets `
                -ArchitectureText "compute_90`ncompute_121" `
                -CodeText "sm_90`nsm_121" } '(?i)(compute_120|sm_120|target)'
    }

    Invoke-P1ATest 'CUDA compile argv freezes dual targets and the qualified host boundary' {
        $cudaRoot = 'C:\fixture\CUDA\v13.1'
        $compilerDirectory = 'C:\fixture\VS 2022\VC\bin\Hostx64\x64'
        $sourcePath = 'C:\fixture\probe source.cu'
        $mixedOutput = 'C:\fixture\probe-mixed.exe'
        $ptxOutput = 'C:\fixture\probe-ptx.exe'
        $hybridLinkerOptions = '--linker-options=/WX,/NODEFAULTLIB:LIBCMT'
        $mixed = @(& $module {
                param($Source, $Output, $Cuda, $Compiler)
                @(New-P1BNvccArguments -Kind Mixed -SourcePath $Source `
                        -OutputPath $Output -CudaRoot $Cuda -CompilerDirectory $Compiler `
                        -ToolkitMajor 13)
            } $sourcePath $mixedOutput $cudaRoot $compilerDirectory)
        $ptxOnly = @(& $module {
                param($Source, $Output, $Cuda, $Compiler)
                @(New-P1BNvccArguments -Kind PtxOnly -SourcePath $Source `
                        -OutputPath $Output -CudaRoot $Cuda -CompilerDirectory $Compiler `
                        -ToolkitMajor 13)
            } $sourcePath $ptxOutput $cudaRoot $compilerDirectory)

        foreach ($fixture in @(
                [pscustomobject]@{ kind = 'mixed'; argv = $mixed; output = $mixedOutput }
                [pscustomobject]@{ kind = 'ptx-only'; argv = $ptxOnly; output = $ptxOutput }
            )) {
            Assert-P1ATest ($fixture.argv -contains '-m64') `
                "$($fixture.kind) compile omitted x64"
            Assert-P1ATest ($fixture.argv -contains '-std=c++17') `
                "$($fixture.kind) compile omitted C++17"
            Assert-P1ATest ($fixture.argv -contains '--cudart=hybrid') `
                "$($fixture.kind) CUDA 13 compile did not select hybrid cudart"
            Assert-P1ATest ($fixture.argv -notcontains '--cudart=shared') `
                "$($fixture.kind) CUDA 13 compile selected deprecated shared cudart"
            Assert-P1ATest ($fixture.argv -contains '--compiler-options=/EHsc,/W4,/WX,/MD') `
                "$($fixture.kind) compile weakened the MSVC warning/runtime contract"
            $linkerOptions = @($fixture.argv | Where-Object {
                    [string]$_ -like '--linker-options=*'
                })
            Assert-P1AEqual $linkerOptions.Count 1 `
                "$($fixture.kind) CUDA 13 compile has an ambiguous linker contract"
            Assert-P1AEqual $linkerOptions[0] $hybridLinkerOptions `
                "$($fixture.kind) CUDA 13 compile does not freeze the /MD conflict exclusions"
            $ignoredCrtDefaults = @([regex]::Matches(
                    ($fixture.argv -join "`n"),
                    '(?i)/NODEFAULTLIB:[^,\r\n]+'
                ) | ForEach-Object { $_.Value.ToUpperInvariant() })
            Assert-P1AEqual ($ignoredCrtDefaults -join ',') '/NODEFAULTLIB:LIBCMT' `
                "$($fixture.kind) CUDA 13 compile suppresses an unaudited CRT default"
            Assert-P1ATest ($fixture.argv -contains $compilerDirectory) `
                "$($fixture.kind) compile omitted the qualified compiler directory"
            Assert-P1ATest ($fixture.argv -contains $sourcePath) `
                "$($fixture.kind) compile omitted the probe source"
            Assert-P1ATest ($fixture.argv -contains $fixture.output) `
                "$($fixture.kind) compile omitted its output path"
            foreach ($library in @('cuda.lib', 'cublas.lib', 'cublasLt.lib')) {
                Assert-P1ATest (@($fixture.argv | Where-Object {
                            [string]$_ -match ('(?i)[\\/]' + [regex]::Escape($library) + '$')
                        }).Count -eq 1) `
                    "$($fixture.kind) compile does not link exactly one $library"
            }
            Assert-P1ATest (@($fixture.argv | Where-Object {
                        [string]$_ -match '(?i)[\\/]cudart\.lib$'
                    }).Count -eq 0) `
                "$($fixture.kind) compile explicitly links cudart.lib"
            Assert-P1ATest (($fixture.argv -join ' ') -notmatch `
                    '(?i)--allow-unsupported-compiler') `
                "$($fixture.kind) compile bypasses host-compiler support"
        }
        $mixedGencode = @($mixed | Where-Object { $_ -like '-gencode=*' })
        Assert-P1AEqual $mixedGencode.Count 1 `
            'mixed compile does not use one canonical dual-code gencode argument'
        Assert-P1AEqual $mixedGencode[0] `
            '-gencode=arch=compute_120,code=[sm_120,compute_120]' `
            'mixed compile does not freeze native SM120 plus compute_120 PTX'
        $ptxGencode = @($ptxOnly | Where-Object { $_ -like '-gencode=*' })
        Assert-P1AEqual $ptxGencode.Count 1 `
            'PTX-only compile does not use one canonical gencode argument'
        Assert-P1AEqual $ptxGencode[0] '-gencode=arch=compute_120,code=compute_120' `
            'PTX-only compile target changed'

        $cuda12 = @(& $module {
                param($Source, $Output, $Cuda, $Compiler)
                @(New-P1BNvccArguments -Kind Mixed -SourcePath $Source `
                        -OutputPath $Output -CudaRoot $Cuda -CompilerDirectory $Compiler `
                        -ToolkitMajor 12)
            } $sourcePath $mixedOutput $cudaRoot $compilerDirectory)
        Assert-P1ATest ($cuda12 -contains '--cudart=shared') `
            'CUDA 12 compile did not select shared cudart'
        Assert-P1ATest ($cuda12 -notcontains '--cudart=hybrid') `
            'CUDA 12 compile selected unsupported hybrid cudart'
        Assert-P1ATest ($cuda12 -contains '--linker-options=/WX') `
            'CUDA 12 compile does not promote linker warnings to errors'
        Assert-P1AEqual @($cuda12 | Where-Object {
                [string]$_ -like '--linker-options=*'
            }).Count 1 'CUDA 12 compile has an ambiguous linker contract'
        Assert-P1ATest (($cuda12 -join "`n") -notmatch '(?i)/NODEFAULTLIB:') `
            'CUDA 12 compile inherited CUDA 13 hybrid CRT exclusions'
        Assert-P1ATest (@($cuda12 | Where-Object {
                    [string]$_ -match '(?i)[\\/]cudart\.lib$'
                }).Count -eq 0) `
            'CUDA 12 compile explicitly links cudart.lib'
    }

    Invoke-P1ATest 'CUDA child environment retains qualified Program Files identities only' {
        $fixtureRoot = New-P1ATestDirectory -Parent $temporaryRoot `
            -Name 'cuda-child-environment'
        $temporary = New-P1ATestDirectory -Parent $fixtureRoot -Name 'temporary'
        $canary = New-P1ATestDirectory -Parent $fixtureRoot -Name 'canary'
        $marker = New-P1ATestDirectory -Parent $fixtureRoot -Name 'marker'
        $toolkitRoot = New-P1ATestDirectory -Parent $fixtureRoot -Name 'toolkit'
        $qualifiedProgramFiles = 'C:\Qualified Program Files'
        $qualifiedProgramW6432 = 'D:\Qualified ProgramW6432'
        $vs = [pscustomobject]@{
            environment = [ordered]@{
                Path = 'C:\Qualified VS\bin'
                ProgramFiles = $qualifiedProgramFiles
                ProgramW6432 = $qualifiedProgramW6432
                PYTHONPATH = 'C:\sensitive-python-path'
                RUSTC_WRAPPER = 'C:\sensitive-wrapper.exe'
                P1B_UNRELATED_FIXTURE = 'must-not-survive'
                P1B_SECRET_FIXTURE = 'must-not-survive'
            }
            tools = @{
                cl = Join-Path $fixtureRoot 'vs\cl.exe'
                rc = Join-Path $fixtureRoot 'sdk\rc.exe'
            }
        }
        $toolkit = [pscustomobject]@{ root = $toolkitRoot }

        $child = & $module {
            param($Vs, $Toolkit, $Temporary, $Canary, $Marker)
            New-P1BCudaChildEnvironment -Vs $Vs -Toolkit $Toolkit `
                -TemporaryRoot $Temporary -CanaryRoot $Canary -MarkerRoot $Marker
        } $vs $toolkit $temporary $canary $marker

        Assert-P1AEqual ([string]$child['ProgramFiles']) $qualifiedProgramFiles `
            'CUDA child environment replaced or removed qualified ProgramFiles'
        Assert-P1AEqual ([string]$child['ProgramW6432']) $qualifiedProgramW6432 `
            'CUDA child environment replaced or removed qualified ProgramW6432'
        Assert-P1ATest (-not $child.ContainsKey('P1B_UNRELATED_FIXTURE')) `
            'CUDA child environment admitted an unrelated VS variable'
        Assert-P1ATest (-not $child.ContainsKey('P1B_SECRET_FIXTURE')) `
            'CUDA child environment admitted a secret-shaped VS variable'
        Assert-P1ATest ($child.ContainsKey('PYTHONPATH') -and
            $null -eq $child['PYTHONPATH']) `
            'CUDA child environment did not clear PYTHONPATH'
        Assert-P1ATest ($child.ContainsKey('RUSTC_WRAPPER') -and
            $null -eq $child['RUSTC_WRAPPER']) `
            'CUDA child environment did not clear RUSTC_WRAPPER'
    }

    Invoke-P1ATest 'CUDA artifact inspection distinguishes mixed SASS from PTX-only fallback' {
        $elf = 'ELF file 1: probe.sm_120.cubin'
        $ptxList = 'PTX file 1: probe.sm_120.ptx'
        $ptx = ".version 8.8`n.target sm_120`n.address_size 64`n" +
            '.visible .entry _Z12add_sentinelPi()'
        $encodedInstruction = `
            '/*0000*/ MOV R1, c[0x0][0x28] ; /* 0x00000a00ff017624 */'
        $sentinelFunction = 'Function : _Z12add_sentinelPi'
        $mixedSass = "code for sm_120`n$sentinelFunction`n$encodedInstruction"
        $ptxOnlyMetadata = @'
Fatbin ptx code:
================
arch = sm_120
code version = [8,8]
'@

        $mixed = & $module {
            param($Elf, $PtxList, $Sass, $Ptx)
            Assert-P1BArtifactInspection -Kind Mixed -ListElfText $Elf `
                -ListPtxText $PtxList -SassText $Sass -PtxText $Ptx
        } $elf $ptxList $mixedSass $ptx
        Assert-P1AEqual ($mixed.embedded_sass -join ',') 'sm_120' `
            'mixed inspection lost native SM120 evidence'
        Assert-P1AEqual ($mixed.embedded_ptx -join ',') 'compute_120' `
            'mixed inspection lost compute_120 PTX evidence'

        $ptxOnly = & $module {
            param($PtxList, $SassMetadata, $Ptx)
            Assert-P1BArtifactInspection -Kind PtxOnly `
                -ListElfText 'No ELF file found' -ListPtxText $PtxList `
                -SassText $SassMetadata -PtxText $Ptx
        } $ptxList $ptxOnlyMetadata $ptx
        Assert-P1AEqual @($ptxOnly.embedded_sass).Count 0 `
            'PTX-only metadata was misclassified as embedded SASS'
        Assert-P1AEqual ($ptxOnly.embedded_ptx -join ',') 'compute_120' `
            'PTX-only inspection lost compute_120 PTX evidence'

        Assert-P1AThrows { & $module {
                param($PtxList, $Sass, $Ptx)
                Assert-P1BArtifactInspection -Kind Mixed `
                    -ListElfText 'No ELF file found' -ListPtxText $PtxList `
                    -SassText $Sass -PtxText $Ptx
            } $ptxList $mixedSass $ptx } '(?i)(SASS|ELF|sm_120|mixed)'
        Assert-P1AThrows { & $module {
                param($Elf, $PtxList, $Ptx)
                Assert-P1BArtifactInspection -Kind Mixed `
                    -ListElfText $Elf -ListPtxText $PtxList `
                    -SassText "code for sm_120`nFunction : _Z12add_sentinelPi" `
                    -PtxText $Ptx
            } $elf $ptxList $ptx } '(?i)(SASS|instruction|encoded|mixed)'
        Assert-P1AThrows { & $module {
                param($Elf, $PtxList, $Ptx)
                Assert-P1BArtifactInspection -Kind Mixed `
                    -ListElfText $Elf -ListPtxText $PtxList `
                    -SassText "code for sm_120`nFunction : _Z12add_sentinelPi`n/*0000*/ MOV R1, R2 ;" `
                    -PtxText $Ptx
            } $elf $ptxList $ptx } '(?i)(SASS|instruction|encoded|mixed)'
        Assert-P1AThrows { & $module {
                param($Elf, $PtxList, $Function, $Instruction, $Ptx)
                Assert-P1BArtifactInspection -Kind Mixed `
                    -ListElfText $Elf -ListPtxText $PtxList `
                    -SassText "$Function`n$Instruction" -PtxText $Ptx
            } $elf $ptxList $sentinelFunction $encodedInstruction $ptx } `
            '(?i)(SASS|code for|instruction|mixed)'
        Assert-P1AThrows { & $module {
                param($PtxList, $SassMetadata, $Ptx)
                Assert-P1BArtifactInspection -Kind Mixed `
                    -ListElfText 'No ELF file found' -ListPtxText $PtxList `
                    -SassText $SassMetadata -PtxText $Ptx
            } $ptxList $ptxOnlyMetadata $ptx } '(?i)(SASS|ELF|sm_120|mixed)'
        Assert-P1AThrows { & $module {
                param($Elf, $PtxList, $Sass, $Ptx)
                Assert-P1BArtifactInspection -Kind PtxOnly `
                    -ListElfText $Elf -ListPtxText $PtxList `
                    -SassText $Sass -PtxText $Ptx
            } $elf $ptxList $mixedSass $ptx } '(?i)(PTX-only|SASS|unexpected|ELF)'
        Assert-P1AThrows { & $module {
                param($Elf, $PtxList, $Sass, $Ptx)
                Assert-P1BArtifactInspection -Kind Mixed `
                    -ListElfText ($Elf + "`nELF file 2: probe.sm_121.cubin") `
                    -ListPtxText $PtxList -SassText $Sass -PtxText $Ptx
            } $elf $ptxList $mixedSass $ptx } '(?i)(SASS|ELF|120|mixed)'
        Assert-P1AThrows { & $module {
                param($Elf, $PtxList, $Sass, $Ptx)
                Assert-P1BArtifactInspection -Kind Mixed `
                    -ListElfText ($Elf + "`nELF file 2: unexpected.cubin") `
                    -ListPtxText $PtxList -SassText $Sass -PtxText $Ptx
            } $elf $ptxList $mixedSass $ptx } '(?i)(SASS|ELF|120|mixed)'
        Assert-P1AThrows { & $module {
                param($Elf, $PtxList, $Sass, $Ptx)
                Assert-P1BArtifactInspection -Kind Mixed -ListElfText $Elf `
                    -ListPtxText ($PtxList + "`nPTX file 2: probe.sm_121.ptx") `
                    -SassText $Sass -PtxText $Ptx
            } $elf $ptxList $mixedSass $ptx } '(?i)(PTX|120|inspect)'
        Assert-P1AThrows { & $module {
                param($Elf, $PtxList, $Sass, $Ptx)
                Assert-P1BArtifactInspection -Kind Mixed -ListElfText $Elf `
                    -ListPtxText ($PtxList + "`nPTX file 2: unexpected.ptx") `
                    -SassText $Sass -PtxText $Ptx
            } $elf $ptxList $mixedSass $ptx } '(?i)(PTX|120|inspect)'
        Assert-P1AThrows { & $module {
                param($Elf, $PtxList, $Sass)
                Assert-P1BArtifactInspection -Kind Mixed -ListElfText $Elf `
                    -ListPtxText $PtxList -SassText $Sass `
                    -PtxText ".version 8.8`n.target sm_120`n.address_size 64"
            } $elf $ptxList $mixedSass } '(?i)(PTX|entry|sentinel|inspect)'
        Assert-P1AThrows { & $module {
                Assert-P1BArtifactInspection -Kind Mixed `
                    -ListElfText 'ELF file: sm_121' -ListPtxText 'PTX file' `
                    -SassText 'code for sm_121 /*0000*/ NOP; /* 0x0000000000000000 */' `
                    -PtxText ".version 8.8`n.target sm_121"
            } } '(?i)(PTX|SASS|120|target|inspect)'
    }

    Invoke-P1ATest 'CUDA PE inspection requires math libraries and permits hybrid runtime imports' {
        $minimalDependents = @'
Image has the following dependencies:
    KERNEL32.dll
    cublas64_13.dll
    cublasLt64_13.dll
'@
        $minimalImports = @(& $module {
                param($Text)
                @(Assert-P1BPeInspection -HeadersText '8664 machine (x64)' `
                        -DependentsText $Text -RuntimeLinkage Hybrid)
            } $minimalDependents)
        Assert-P1AEqual ($minimalImports -join ',') `
            'cublas64_13.dll,cublaslt64_13.dll,kernel32.dll' `
            'hybrid-runtime PE import normalization changed'

        $explicitRuntimeDependents = $minimalDependents + "`n" + @'
    nvcuda.dll
    cudart64_13.dll
'@
        $explicitRuntimeImports = @(& $module {
                param($Text)
                @(Assert-P1BPeInspection -HeadersText '8664 machine (x64)' `
                        -DependentsText $Text -RuntimeLinkage Hybrid)
            } $explicitRuntimeDependents)
        Assert-P1AEqual ($explicitRuntimeImports -join ',') `
            'cublas64_13.dll,cublaslt64_13.dll,cudart64_13.dll,kernel32.dll,nvcuda.dll' `
            'PE validation rejected optional driver/runtime imports'
        Assert-P1AThrows { & $module {
                param($Text)
                Assert-P1BPeInspection -HeadersText '14C machine (x86)' `
                    -DependentsText $Text -RuntimeLinkage Hybrid
            } $minimalDependents } '(?i)(x64|machine|architecture|PE)'
        Assert-P1AThrows { & $module {
                Assert-P1BPeInspection -HeadersText '8664 machine (x64)' `
                    -DependentsText "KERNEL32.dll`ncublas64_13.dll" `
                    -RuntimeLinkage Hybrid
            } } '(?i)(cublaslt|math|boundary|import)'
        Assert-P1AThrows { & $module {
                Assert-P1BPeInspection -HeadersText '8664 machine (x64)' `
                    -DependentsText "KERNEL32.dll`ncublasLt64_13.dll" `
                    -RuntimeLinkage Hybrid
            } } '(?i)(cublas|math|boundary|import)'
        Assert-P1AThrows { & $module {
                param($Text)
                Assert-P1BPeInspection -HeadersText '8664 machine (x64)' `
                    -DependentsText ($Text + "`npython311.dll") `
                    -RuntimeLinkage Hybrid
            } $minimalDependents } '(?i)(unapproved|python|import)'
        Assert-P1AThrows { & $module {
                param($Text)
                Assert-P1BPeInspection -HeadersText '8664 machine (x64)' `
                    -DependentsText ($Text + "`ncudnn64_9.dll") `
                    -RuntimeLinkage Hybrid
            } $minimalDependents } '(?i)(unapproved|cudnn|import)'

        Assert-P1AThrows { & $module {
                param($Text)
                Assert-P1BPeInspection -HeadersText '8664 machine (x64)' `
                    -DependentsText $Text -RuntimeLinkage Shared
            } $minimalDependents } '(?i)(cudart|shared|runtime|import)'
        $sharedDependents = @'
Image has the following dependencies:
    KERNEL32.dll
    cudart64_12.dll
    cublas64_12.dll
    cublasLt64_12.dll
'@
        $sharedImports = @(& $module {
                param($Text)
                @(Assert-P1BPeInspection -HeadersText '8664 machine (x64)' `
                        -DependentsText $Text -RuntimeLinkage Shared)
            } $sharedDependents)
        Assert-P1AEqual ($sharedImports -join ',') `
            'cublas64_12.dll,cublaslt64_12.dll,cudart64_12.dll,kernel32.dll' `
            'shared-runtime PE validation did not require exactly one cudart import'
    }

    Invoke-P1ATest 'nvidia-smi parser permits other GPUs and selects exactly one target' {
        $parsed = ConvertFrom-P1BNvidiaSmi -Text `
            ("0, NVIDIA GeForce RTX 5090, 610.88, 32607, 12.0`n" +
                '1, NVIDIA GeForce RTX 4090, 610.88, 24564, 8.9')
        Assert-P1AEqual @($parsed.devices).Count 2 'GPU inventory record count changed'
        Assert-P1AEqual $parsed.target_match_count 1 `
            'GPU inventory did not select exactly one target'
        Assert-P1AEqual $parsed.target.index 0 'GPU target index parsed incorrectly'
        Assert-P1AEqual $parsed.target.name 'NVIDIA GeForce RTX 5090' `
            'GPU target name parsed incorrectly'
        Assert-P1AEqual $parsed.target.driver_version '610.88' `
            'driver version parsed incorrectly'
        Assert-P1AEqual $parsed.target.memory_total_mib 32607 `
            'GPU memory parsed incorrectly'
        Assert-P1AEqual $parsed.target.compute_capability_major 12 `
            'GPU CC major parsed incorrectly'
        Assert-P1AEqual $parsed.target.compute_capability_minor 0 `
            'GPU CC minor parsed incorrectly'
        Assert-P1AEqual $parsed.devices[1].name 'NVIDIA GeForce RTX 4090' `
            'inventory parser dropped or rewrote a non-target GPU'

        Assert-P1AThrows { ConvertFrom-P1BNvidiaSmi -Text `
                '1, NVIDIA GeForce RTX 4090, 610.88, 24564, 8.9' } `
            '(?i)(missing|zero|exactly one|5090|target|GPU)'
        Assert-P1AThrows { ConvertFrom-P1BNvidiaSmi -Text `
                ("0, NVIDIA GeForce RTX 5090, 610.88, 32607, 12.0`n" +
                    '1, NVIDIA GeForce RTX 5090, 610.88, 32607, 12.0') } `
            '(?i)(ambiguous|multiple|exactly one|5090|target|GPU)'
        Assert-P1AThrows { ConvertFrom-P1BNvidiaSmi -Text `
                'zero, NVIDIA GeForce RTX 5090, 610.88, 32607, 12.0' } `
            '(?i)(invalid|numeric|nvidia-smi|GPU|parse)'
        Assert-P1AThrows { ConvertFrom-P1BNvidiaSmi -Text '' } `
            '(?i)(empty|nvidia-smi|GPU|parse|target)'
    }

    Invoke-P1ATest 'CUDA toolkit selection rejects 12.7 and deterministically prefers 13.1' {
        $fixtureRoot = New-P1ATestDirectory -Parent $temporaryRoot -Name 'cuda-toolkit-order'
        $cuda127 = New-P1BTestToolkit -Parent $fixtureRoot -Name 'v12.7' -Version '12.7.99'
        $cuda128 = New-P1BTestToolkit -Parent $fixtureRoot -Name 'v12.8' -Version '12.8.93'
        $cuda129 = New-P1BTestToolkit -Parent $fixtureRoot -Name 'v12.9' -Version '12.9.41'
        $cuda1210 = New-P1BTestToolkit -Parent $fixtureRoot -Name 'v12.10' -Version '12.10.7'
        $cuda131 = New-P1BTestToolkit -Parent $fixtureRoot -Name 'v13.1' -Version '13.1.80'

        Assert-P1AThrows { Select-P1BCudaToolkit -Candidates @($cuda127) } `
            '(?i)(12\.8|unsupported|minimum|version)'
        foreach ($candidate in @($cuda128, $cuda129, $cuda1210, $cuda131)) {
            $single = Select-P1BCudaToolkit -Candidates @($candidate)
            Assert-P1AEqual ([IO.Path]::GetFullPath([string]$single.root)) `
                ([IO.Path]::GetFullPath([string]$candidate.root)) `
                "supported CUDA candidate was not selectable: $($candidate.root)"
        }
        $first = Select-P1BCudaToolkit -Candidates @($cuda129, $cuda128, $cuda131, $cuda1210)
        $second = Select-P1BCudaToolkit -Candidates @($cuda131, $cuda1210, $cuda129, $cuda128)
        Assert-P1AEqual ([IO.Path]::GetFullPath([string]$first.root)) `
            ([IO.Path]::GetFullPath([string]$cuda131.root)) `
            'selector did not choose the highest supported CUDA version'
        Assert-P1AEqual ([IO.Path]::GetFullPath([string]$second.root)) `
            ([IO.Path]::GetFullPath([string]$first.root)) `
            'CUDA selection depends on discovery order'

        $cuda128.explicit = $true
        $withExplicitOlder = Select-P1BCudaToolkit -Candidates @($cuda131, $cuda128)
        Assert-P1AEqual ([IO.Path]::GetFullPath([string]$withExplicitOlder.root)) `
            ([IO.Path]::GetFullPath([string]$cuda131.root)) `
            'an older explicit discovery source overrode the highest qualifying toolkit'

        $duplicate = [pscustomobject]@{
            root = $cuda131.root
            source = 'fixture:duplicate-canonical-root'
            explicit = $true
        }
        $deduplicated = Select-P1BCudaToolkit -Candidates @($cuda131, $duplicate)
        Assert-P1AEqual ([IO.Path]::GetFullPath([string]$deduplicated.root)) `
            ([IO.Path]::GetFullPath([string]$cuda131.root)) `
            'duplicate canonical roots were not deduplicated'

        $sameVersion = New-P1BTestToolkit -Parent $fixtureRoot -Name 'v13.1-second' `
            -Version '13.1.80'
        $tieA = Select-P1BCudaToolkit -Candidates @($cuda131, $sameVersion)
        $tieB = Select-P1BCudaToolkit -Candidates @($sameVersion, $cuda131)
        Assert-P1AEqual ([IO.Path]::GetFullPath([string]$tieA.root)) `
            ([IO.Path]::GetFullPath([string]$tieB.root)) `
            'same-version CUDA selection depends on discovery order'
    }

    Invoke-P1ATest 'CUDA toolkit selection enforces the required coherent boundary only' {
        $fixtureRoot = New-P1ATestDirectory -Parent $temporaryRoot -Name 'cuda-toolkit-boundary'
        $missingTool = New-P1BTestToolkit -Parent $fixtureRoot -Name 'missing-tool' `
            -Version '13.1.80' -Omit @('bin\ptxas.exe')
        Assert-P1AThrows { Select-P1BCudaToolkit -Candidates @($missingTool) } `
            '(?i)(CUDA|toolkit|missing|complete|found|component)'

        $missingLtHeader = New-P1BTestToolkit -Parent $fixtureRoot -Name 'missing-lt-header' `
            -Version '13.1.80' -Omit @('include\cublasLt.h')
        Assert-P1AThrows { Select-P1BCudaToolkit -Candidates @($missingLtHeader) } `
            '(?i)(CUDA|toolkit|missing|complete|found|component)'

        $missingLtLibrary = New-P1BTestToolkit -Parent $fixtureRoot -Name 'missing-lt-library' `
            -Version '13.1.80' -Omit @('lib\x64\cublasLt.lib')
        Assert-P1AThrows { Select-P1BCudaToolkit -Candidates @($missingLtLibrary) } `
            '(?i)(CUDA|toolkit|missing|complete|found|component)'

        $missingLtDll = New-P1BTestToolkit -Parent $fixtureRoot -Name 'missing-lt-dll' `
            -Version '13.1.80' -Omit @('bin\x64\cublasLt64_13.dll')
        Assert-P1AThrows { Select-P1BCudaToolkit -Candidates @($missingLtDll) } `
            '(?i)(CUDA|toolkit|missing|complete|found|component)'

        $noOptionalLibraries = New-P1BTestToolkit -Parent $fixtureRoot `
            -Name 'no-optional-libraries' -Version '13.1.80'
        $selected = Select-P1BCudaToolkit -Candidates @($noOptionalLibraries)
        Assert-P1AEqual ([IO.Path]::GetFullPath([string]$selected.root)) `
            ([IO.Path]::GetFullPath([string]$noOptionalLibraries.root)) `
            'absence of cuDNN, cuRAND, NVRTC, or NVJitLink incorrectly blocked selection'

        $duplicateRuntime = New-P1BTestToolkit -Parent $fixtureRoot `
            -Name 'duplicate-runtime' -Version '13.1.80'
        Write-P1AUtf8LfFile -Path (Join-Path $duplicateRuntime.root `
                'bin\x64\cudart64_13_duplicate.dll') -Text 'duplicate'
        Assert-P1AThrows { Select-P1BCudaToolkit -Candidates @($duplicateRuntime) } `
            '(?i)(CUDA|toolkit|runtime|complete|found|ambiguous)'
    }

    Invoke-P1ATest 'CUDA toolkit selection rejects a reparse-point installation root' {
        $fixtureRoot = New-P1ATestDirectory -Parent $temporaryRoot -Name 'cuda-toolkit-reparse'
        $target = New-P1BTestToolkit -Parent $fixtureRoot -Name 'real-toolkit' `
            -Version '13.1.80'
        $junction = Join-Path $fixtureRoot 'junction-toolkit'
        [void](New-Item -ItemType Junction -Path $junction -Target $target.root)
        try {
            $candidate = [pscustomobject]@{
                root = $junction
                source = 'fixture:junction'
                explicit = $true
            }
            Assert-P1AThrows { Select-P1BCudaToolkit -Candidates @($candidate) } `
                '(?i)(reparse|junction|link|canonical|root)'
        }
        finally {
            if (Test-Path -LiteralPath $junction) {
                [IO.Directory]::Delete($junction)
            }
        }
    }

    Invoke-P1ATest 'CUDA device probe validator enforces RTX identity and launch results' {
        $valid = [pscustomobject][ordered]@{
            schema = 'python-slm-cuda-device-probe-v1'
            device_count = 2
            target_match_count = 1
            device_index = 0
            device_name = 'NVIDIA GeForce RTX 5090'
            memory_total_bytes = [int64]32607 * 1MB
            compute_capability_major = 12
            compute_capability_minor = 0
            runtime_version = 13010
            driver_version = 13010
            cublas_version = 130100
            cublaslt_version = 130100
            sentinel = 42
        }
        $normalized = Assert-P1BProbeResult -Value $valid
        Assert-P1AEqual $normalized.device_name $valid.device_name `
            'valid probe result changed GPU identity'
        Assert-P1AEqual $normalized.sentinel 42 'valid probe result changed its sentinel'

        $rejections = @(
            [pscustomobject]@{ id = 'zero-device'; field = 'device_count'; value = 0 }
            [pscustomobject]@{ id = 'zero-target'; field = 'target_match_count'; value = 0 }
            [pscustomobject]@{ id = 'ambiguous-target'; field = 'target_match_count'; value = 2 }
            [pscustomobject]@{ id = 'wrong-name'; field = 'device_name'; value = 'NVIDIA GeForce RTX 4090' }
            [pscustomobject]@{ id = 'wrong-cc'; field = 'compute_capability_minor'; value = 1 }
            [pscustomobject]@{ id = 'old-runtime'; field = 'runtime_version'; value = 12070 }
            [pscustomobject]@{ id = 'driver-behind-runtime'; field = 'driver_version'; value = 13000 }
            [pscustomobject]@{ id = 'missing-cublas'; field = 'cublas_version'; value = 0 }
            [pscustomobject]@{ id = 'missing-cublaslt'; field = 'cublaslt_version'; value = 0 }
            [pscustomobject]@{ id = 'bad-sentinel'; field = 'sentinel'; value = 41 }
        )
        foreach ($rejection in $rejections) {
            $candidate = $valid.PSObject.Copy()
            $candidate.($rejection.field) = $rejection.value
            Assert-P1AThrows { Assert-P1BProbeResult -Value $candidate } `
                '(?i)(CUDA|device|probe|invalid)'
        }
        $unknown = $valid.PSObject.Copy()
        $unknown | Add-Member -NotePropertyName unexpected -NotePropertyValue $true
        Assert-P1AThrows { Assert-P1BProbeResult -Value $unknown } `
            '(?i)(unknown|closed|unexpected|field|property)'
    }

    Invoke-P1ATest 'P1B dependency binds the selected reviewed P1A chain and rejects tampering' {
        $dependency = Get-P1BSelectedP1ADependency -RepositoryRoot $repositoryRoot
        $names = if ($dependency -is [Collections.IDictionary]) {
            @($dependency.Keys | Sort-Object)
        }
        else {
            @($dependency.PSObject.Properties.Name | Sort-Object)
        }
        $expectedNames = @(
            'status', 'review_closure_commit',
            'pointer_path', 'pointer_sha256',
            'acceptance_path', 'acceptance_sha256', 'acceptance_sequence',
            'run_path', 'run_evidence_sha256',
            'seal_path', 'seal_sha256',
            'environment_path', 'environment_sha256',
            'source_identity_sha256',
            'verifier_sha256', 'schema_bundle_sha256'
        ) | Sort-Object
        Assert-P1AEqual ($names -join "`n") ($expectedNames -join "`n") `
            'P1A dependency fields do not match the closed P1B contract'
        Assert-P1AEqual $dependency.status 'PASS' 'P1A dependency is not PASS'
        Assert-P1AEqual $dependency.review_closure_commit `
            '9359c989fa63d4a300abc509e735b7e81a24a2ea' `
            'P1A dependency does not pin the owner-review commit'
        foreach ($field in @(
                'pointer_sha256', 'acceptance_sha256', 'run_evidence_sha256',
                'seal_sha256', 'environment_sha256', 'source_identity_sha256',
                'verifier_sha256', 'schema_bundle_sha256'
            )) {
            Assert-P1ATest ([string]$dependency[$field] -match '^[0-9a-f]{64}$') `
                "P1A dependency $field is not a canonical SHA-256"
        }

        $fakeRepository = New-P1ATestDirectory -Parent $temporaryRoot `
            -Name 'p1a-dependency-tamper'
        $fakeReceipts = Join-Path $fakeRepository 'docs\receipts'
        [void][IO.Directory]::CreateDirectory($fakeReceipts)
        Copy-Item -LiteralPath (Join-Path $repositoryRoot 'docs\receipts\P1A') `
            -Destination $fakeReceipts -Recurse
        $fakePointerPath = Join-Path $fakeRepository 'docs\receipts\P1A\evidence.json'
        $originalPointerText = Get-P1ATestStrictText -Path $fakePointerPath
        $pointer = $originalPointerText | ConvertFrom-Json
        $pointer.acceptance_sha256 = '0' * 64
        Write-P1AJsonFile -Path $fakePointerPath -Value $pointer
        Assert-P1AThrows { Get-P1BSelectedP1ADependency `
                -RepositoryRoot $fakeRepository } '(?i)(P1A|dependency|acceptance|hash|invalid)'

        Write-P1AUtf8LfFile -Path $fakePointerPath -Text $originalPointerText
        $pointer = $originalPointerText | ConvertFrom-Json
        $acceptancePath = Join-Path (Join-Path $fakeRepository 'docs\receipts\P1A') `
            ([string]$pointer.acceptance_path).Replace('/', '\')
        $acceptance = Get-P1ATestStrictText -Path $acceptancePath | ConvertFrom-Json
        $environmentPath = Join-Path (Join-Path $fakeRepository 'docs\receipts\P1A') `
            ([string]$acceptance.environment_path).Replace('/', '\')
        Write-P1AUtf8LfFile -Path $environmentPath -Text `
            ((Get-P1ATestStrictText -Path $environmentPath) + "`n")
        Assert-P1AThrows { Get-P1BSelectedP1ADependency `
                -RepositoryRoot $fakeRepository } '(?i)(P1A|dependency|seal|environment|hash|invalid)'
    }

    Invoke-P1ATest 'P1B publication creates automatic v2 acceptance and advances its chain' {
        $outputRoot = New-P1BTestOutputRoot -Parent $temporaryRoot `
            -Name 'p1b-publication-pass'
        $first = New-P1BTestRun -OutputRoot $outputRoot -Status PASS
        [void](Invoke-P1BTestPublication -OutputRoot $outputRoot -Run $first)
        $firstChain = Assert-P1BTestSelectedAcceptanceChain -OutputRoot $outputRoot
        Assert-P1AEqual $firstChain.count 1 'first P1B publication did not select one acceptance'

        $firstAcceptancePath = Join-Path $outputRoot 'acceptances\00000001.json'
        $firstAcceptance = Get-P1ATestStrictText -Path $firstAcceptancePath | ConvertFrom-Json
        $expectedNames = @(
            'schema', 'phase_id', 'sequence', 'status', 'acceptance_kind',
            'required_approvals', 'run_path', 'run_evidence_sha256', 'seal_path',
            'seal_sha256', 'environment_path', 'environment_sha256',
            'source_identity_sha256', 'previous_acceptance_sha256', 'created_at'
        ) | Sort-Object
        Assert-P1AEqual (($firstAcceptance.PSObject.Properties.Name | Sort-Object) -join "`n") `
            ($expectedNames -join "`n") 'P1B acceptance fields are not closed v2'
        Assert-P1AEqual $firstAcceptance.run_path "runs/$($first.run_id)" `
            'first P1B acceptance selected the wrong run'
        Assert-P1AEqual $firstAcceptance.run_evidence_sha256 `
            (Get-P1ASha256 -Path $first.evidence_path) `
            'first P1B acceptance did not pin terminal evidence'
        Assert-P1AEqual $firstAcceptance.environment_sha256 `
            (Get-P1ASha256 -Path $first.environment_path) `
            'first P1B acceptance did not pin its environment manifest'

        $firstHash = Get-P1ASha256 -Path $firstAcceptancePath
        $second = New-P1BTestRun -OutputRoot $outputRoot -Status PASS
        [void](Invoke-P1BTestPublication -OutputRoot $outputRoot -Run $second)
        $secondChain = Assert-P1BTestSelectedAcceptanceChain -OutputRoot $outputRoot
        Assert-P1AEqual $secondChain.count 2 'second P1B publication did not advance the chain'
        Assert-P1AEqual (Get-P1ASha256 -Path $firstAcceptancePath) $firstHash `
            'second P1B publication mutated the prior acceptance'
        $secondAcceptance = Get-P1ATestStrictText `
            -Path (Join-Path $outputRoot 'acceptances\00000002.json') | ConvertFrom-Json
        Assert-P1AEqual $secondAcceptance.previous_acceptance_sha256 $firstHash `
            'second P1B acceptance does not hash-link its predecessor'
    }

    Invoke-P1ATest 'P1B publication rejects failed, tampered, and false-PASS runs' {
        $outputRoot = New-P1BTestOutputRoot -Parent $temporaryRoot `
            -Name 'p1b-publication-reject'
        $passing = New-P1BTestRun -OutputRoot $outputRoot -Status PASS
        [void](Invoke-P1BTestPublication -OutputRoot $outputRoot -Run $passing)
        $pointerPath = Join-Path $outputRoot 'evidence.json'
        $pointerHash = Get-P1ASha256 -Path $pointerPath
        $acceptanceCount = @(Get-ChildItem -LiteralPath (Join-Path $outputRoot 'acceptances') `
            -File -Filter '*.json').Count

        $failed = New-P1BTestRun -OutputRoot $outputRoot -Status FAIL
        Assert-P1AThrows { [void](Invoke-P1BTestPublication -OutputRoot $outputRoot `
                    -Run $failed) } '(?i)(PASS|failed|evidence|accept)'

        $tampered = New-P1BTestRun -OutputRoot $outputRoot -Status PASS
        Write-P1AUtf8LfFile -Path (Join-Path $tampered.run_root 'commands\C01.stdout.txt') `
            -Text 'tampered transcript'
        Assert-P1AThrows { [void](Invoke-P1BTestPublication -OutputRoot $outputRoot `
                    -Run $tampered) } '(?i)(seal|hash|invalid|accept)'

        $falsePass = New-P1BTestRun -OutputRoot $outputRoot -Status PASS
        [IO.File]::Delete((Join-Path $falsePass.run_root 'SHA256SUMS'))
        $evidence = Get-P1ATestStrictText -Path $falsePass.evidence_path | ConvertFrom-Json
        $evidence.gates.device_probe.status = 'FAIL'
        Write-P1AJsonFile -Path $falsePass.evidence_path -Value $evidence
        [void](New-P1ASeal -RunRoot $falsePass.run_root)
        Assert-P1AThrows { [void](Invoke-P1BTestPublication -OutputRoot $outputRoot `
                    -Run $falsePass) } '(?i)(gate|probe|PASS|invalid|accept)'

        $sourceMismatch = New-P1BTestRun -OutputRoot $outputRoot -Status PASS
        $sourceMismatch.source_hash = 'f' * 64
        Assert-P1AThrows { [void](Invoke-P1BTestPublication -OutputRoot $outputRoot `
                    -Run $sourceMismatch) } '(?i)(source|hash|identity|accept)'

        Assert-P1AEqual (Get-P1ASha256 -Path $pointerPath) $pointerHash `
            'rejected P1B run changed the selected pointer'
        Assert-P1AEqual @(Get-ChildItem -LiteralPath (Join-Path $outputRoot 'acceptances') `
                -File -Filter '*.json').Count $acceptanceCount `
            'rejected P1B run advanced the acceptance chain'
    }

    Invoke-P1ATest 'P1B publication rejects runtime-linkage and compile-contract drift' {
        $outputRoot = New-P1BTestOutputRoot -Parent $temporaryRoot `
            -Name 'p1b-runtime-linkage-reject'

        $modeMismatch = New-P1BTestRun -OutputRoot $outputRoot -Status PASS
        Update-P1ATestSealedEnvironment -Run $modeMismatch -Mutation {
            param($Environment)
            $runtimeDlls = @($Environment.cuda_toolkit.runtime_dlls)
            $Environment.runtime_linkage.mode = 'shared'
            $Environment.runtime_linkage.active_provider = 'toolkit_cudart'
            $Environment.runtime_linkage.staged_runtime_dlls = @(
                [ordered]@{
                    component = 'cudart'
                    sha256 = [string](@($runtimeDlls | Where-Object {
                                    $_.name -match '(?i)^cudart64_'
                                })[0].sha256)
                }
                [ordered]@{
                    component = 'cublas'
                    sha256 = [string](@($runtimeDlls | Where-Object {
                                    $_.name -match '(?i)^cublas64_'
                                })[0].sha256)
                }
                [ordered]@{
                    component = 'cublaslt'
                    sha256 = [string](@($runtimeDlls | Where-Object {
                                    $_.name -match '(?i)^cublaslt64_'
                                })[0].sha256)
                }
            )
            return $Environment
        }
        Assert-P1AThrows { [void](Invoke-P1BTestPublication -OutputRoot $outputRoot `
                    -Run $modeMismatch) } '(?i)(runtime|linkage|mode|toolkit|hybrid|13)'

        $hashMismatch = New-P1BTestRun -OutputRoot $outputRoot -Status PASS
        Update-P1ATestSealedEnvironment -Run $hashMismatch -Mutation {
            param($Environment)
            $Environment.runtime_linkage.staged_runtime_dlls[0].sha256 = 'f' * 64
            return $Environment
        }
        Assert-P1AThrows { [void](Invoke-P1BTestPublication -OutputRoot $outputRoot `
                    -Run $hashMismatch) } '(?i)(runtime|linkage|staged|hash|DLL)'

        $compileMismatch = New-P1BTestRun -OutputRoot $outputRoot -Status PASS
        [IO.File]::Delete((Join-Path $compileMismatch.run_root 'SHA256SUMS'))
        $evidence = Get-P1ATestStrictText -Path $compileMismatch.evidence_path |
            ConvertFrom-Json
        $evidence.commands[0].argv = @(
            @($evidence.commands[0].argv | Where-Object {
                    [string]$_ -notlike '--linker-options=*'
                }) + '--linker-options=/WX'
        )
        Write-P1AJsonFile -Path $compileMismatch.evidence_path -Value $evidence
        [void](New-P1ASeal -RunRoot $compileMismatch.run_root)
        Assert-P1AThrows { [void](Invoke-P1BTestPublication -OutputRoot $outputRoot `
                    -Run $compileMismatch) } '(?i)(compile|runtime|linkage|contract)'

        $overbroadCrtSuppression = New-P1BTestRun -OutputRoot $outputRoot -Status PASS
        [IO.File]::Delete((Join-Path $overbroadCrtSuppression.run_root 'SHA256SUMS'))
        $evidence = Get-P1ATestStrictText -Path $overbroadCrtSuppression.evidence_path |
            ConvertFrom-Json
        $evidence.commands[0].argv = @($evidence.commands[0].argv) +
            '--linker-options=/NODEFAULTLIB:LIBCMTD'
        Write-P1AJsonFile -Path $overbroadCrtSuppression.evidence_path -Value $evidence
        [void](New-P1ASeal -RunRoot $overbroadCrtSuppression.run_root)
        Assert-P1AThrows { [void](Invoke-P1BTestPublication -OutputRoot $outputRoot `
                    -Run $overbroadCrtSuppression) } '(?i)(compile|runtime|linkage|contract|CRT)'

        Assert-P1ATest (-not (Test-Path -LiteralPath (Join-Path $outputRoot 'evidence.json'))) `
            'rejected runtime-linkage manifest advanced the selected pointer'
    }

    Invoke-P1ATest 'P1B publication rejects a tampered selected v2 pointer' {
        $outputRoot = New-P1BTestOutputRoot -Parent $temporaryRoot `
            -Name 'p1b-publication-pointer-tamper'
        $first = New-P1BTestRun -OutputRoot $outputRoot -Status PASS
        [void](Invoke-P1BTestPublication -OutputRoot $outputRoot -Run $first)
        $pointerPath = Join-Path $outputRoot 'evidence.json'
        $pointer = Get-P1ATestStrictText -Path $pointerPath | ConvertFrom-Json
        $pointer.acceptance_sha256 = '0' * 64
        Write-P1AJsonFile -Path $pointerPath -Value $pointer
        $tamperedHash = Get-P1ASha256 -Path $pointerPath
        $acceptanceCount = @(Get-ChildItem -LiteralPath (Join-Path $outputRoot 'acceptances') `
            -File -Filter '*.json').Count

        $second = New-P1BTestRun -OutputRoot $outputRoot -Status PASS
        Assert-P1AThrows { [void](Invoke-P1BTestPublication -OutputRoot $outputRoot `
                    -Run $second) } '(?i)(pointer|hash|selected|invalid|accept)'
        Assert-P1AEqual (Get-P1ASha256 -Path $pointerPath) $tamperedHash `
            'failed P1B pointer validation replaced the tampered pointer'
        Assert-P1AEqual @(Get-ChildItem -LiteralPath (Join-Path $outputRoot 'acceptances') `
                -File -Filter '*.json').Count $acceptanceCount `
            'failed P1B pointer validation advanced the acceptance sequence'
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

    Invoke-P1ATest 'command receipts retain only the executable leaf name' {
        $runRoot = New-P1ATestDirectory -Parent $temporaryRoot `
            -Name 'command-executable-leaf'
        [void][IO.Directory]::CreateDirectory((Join-Path $runRoot 'commands'))
        $context = @{
            RepositoryRoot = $repositoryRoot
            RedactionRoots = @()
            RunRoot = $runRoot
            Commands = [Collections.Generic.List[object]]::new()
        }
        $displayFile = Join-Path $PSHOME 'powershell.exe'
        Assert-P1ATest ($displayFile -match '(?i)^[a-z]:[\\/]') `
            'test executable is not an absolute Windows path'
        $result = [pscustomobject]@{
            stdout = ''
            stderr = ''
            exit_code = 0
            duration_ms = 1
            timed_out = $false
        }

        $record = & $module {
            param($Context, $DisplayFile, $Result)
            Save-P1ACommandResult -Context $Context -Id 'C01' `
                -DisplayFile $DisplayFile -Arguments @('--version') -Result $Result
        } $context $displayFile $result

        Assert-P1AEqual ([string]$record.argv[0]) `
            (Split-Path -Leaf $displayFile) `
            'command receipt persisted more than the executable leaf name'
        Assert-P1ATest (($record | ConvertTo-Json -Depth 8 -Compress) -notmatch `
                [regex]::Escape($displayFile)) `
            'command receipt leaked the absolute executable path'
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

    Invoke-P1ATest 'Cuda entrypoint preserves terminal JSON stream and exit semantics' {
        $fakeRepository = New-P1ATestDirectory -Parent $temporaryRoot -Name 'cuda-entrypoint-repository'
        $fakeScripts = Join-Path $fakeRepository 'scripts'
        $fakeLibrary = Join-Path $fakeScripts 'lib'
        [void][IO.Directory]::CreateDirectory($fakeLibrary)
        $fakeEntrypoint = Join-Path $fakeScripts 'verify-env.ps1'
        $fakeModule = Join-Path $fakeLibrary 'VerifyEnv.psm1'
        [IO.File]::Copy($entrypointFullPath, $fakeEntrypoint, $false)
        $fakeModuleSource = @'
function Protect-P1AText {
    param([string]$Text, [string]$RepositoryRoot, [string[]]$ExtraRoots)
    return $Text
}
function Invoke-P1AVerification {
    param([string]$Mode, [string]$OutputRoot, [string]$RepositoryRoot)
    if ($Mode -cne 'Cuda') { throw 'fixture expected Cuda mode' }
    $runId = '20260811T000000000Z-111111111111111111111111'
    if ($env:P1B_TEST_OUTCOME -ceq 'PASS') {
        return [pscustomobject][ordered]@{
            status = 'PASS'
            phase_id = 'P1B'
            run_id = $runId
            evidence_path = "docs/receipts/P1B/runs/$runId/evidence.json"
            acceptance_path = 'docs/receipts/P1B/acceptances/00000001.json'
        }
    }
    return [pscustomobject][ordered]@{
        status = 'FAIL'
        phase_id = 'P1B'
        run_id = $runId
        evidence_path = "docs/receipts/P1B/runs/$runId/evidence.json"
        errors = @([pscustomobject][ordered]@{
                code = 'TEST_POLICY_FAILURE'
                category = 3
                message = 'fixture CUDA policy failure'
                remediation = 'repair fixture'
            })
    }
}
Export-ModuleMember -Function Protect-P1AText, Invoke-P1AVerification
'@
        Write-P1AUtf8LfFile -Path $fakeModule -Text $fakeModuleSource

        $powerShell = Join-Path $PSHOME 'powershell.exe'
        $arguments = @(
            '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $fakeEntrypoint,
            '-Mode', 'Cuda', '-OutputRoot', 'docs\receipts\P1B'
        )
        $success = Invoke-P1AProcess -FilePath $powerShell -ArgumentList $arguments `
            -WorkingDirectory $repositoryRoot -Environment @{ P1B_TEST_OUTCOME = 'PASS' } `
            -TimeoutSeconds 30
        Assert-P1AEqual $success.exit_code 0 `
            "Cuda PASS entrypoint returned nonzero; stdout=<$($success.stdout)> stderr=<$($success.stderr)>"
        Assert-P1ATest (-not [bool]$success.timed_out) 'Cuda PASS entrypoint timed out'
        Assert-P1ATest ([string]::IsNullOrWhiteSpace([string]$success.stderr)) `
            'Cuda PASS entrypoint wrote diagnostics to stderr'
        $successLines = @([string]$success.stdout -split '\r?\n' | Where-Object {
                -not [string]::IsNullOrWhiteSpace($_)
            })
        Assert-P1AEqual $successLines.Count 1 `
            'Cuda PASS entrypoint did not emit exactly one terminal JSON object'
        $successJson = $successLines[0] | ConvertFrom-Json
        Assert-P1AEqual $successJson.status 'PASS' 'Cuda terminal success status changed'
        Assert-P1AEqual $successJson.phase_id 'P1B' 'Cuda terminal success phase changed'
        Assert-P1ATest ([string]$successJson.evidence_path -match `
                '^docs/receipts/P1B/runs/.+/evidence\.json$') `
            'Cuda terminal success omitted its evidence path'

        $failure = Invoke-P1AProcess -FilePath $powerShell -ArgumentList $arguments `
            -WorkingDirectory $repositoryRoot -Environment @{ P1B_TEST_OUTCOME = 'FAIL' } `
            -TimeoutSeconds 30
        Assert-P1AEqual $failure.exit_code 3 'Cuda failure returned the wrong exit category'
        Assert-P1ATest (-not [bool]$failure.timed_out) 'Cuda failure entrypoint timed out'
        Assert-P1ATest ([string]::IsNullOrWhiteSpace([string]$failure.stdout)) `
            'failure entrypoint wrote a success payload to stdout'

        $stderrLines = @([string]$failure.stderr -split '\r?\n' | Where-Object {
                -not [string]::IsNullOrWhiteSpace($_)
            })
        Assert-P1AEqual $stderrLines.Count 2 `
            'Cuda entrypoint did not emit one diagnostic and one terminal error'
        $diagnostic = $stderrLines[0] | ConvertFrom-Json
        $terminal = $stderrLines[1] | ConvertFrom-Json
        Assert-P1AEqual $diagnostic.schema 'python-slm-script-diagnostic-v1' `
            'Cuda diagnostic schema changed'
        Assert-P1AEqual $diagnostic.phase_id 'P1B' 'Cuda diagnostic phase changed'
        Assert-P1AEqual $diagnostic.code 'TEST_POLICY_FAILURE' `
            'Cuda diagnostic did not expose the fixture failure'
        Assert-P1AEqual $diagnostic.category 3 'Cuda diagnostic category changed'
        Assert-P1AEqual $terminal.schema 'python-slm-script-error-v1' `
            'Cuda terminal error schema changed'
        Assert-P1AEqual $terminal.code 'QUALIFICATION_FAILED' `
            'Cuda terminal error code changed'
        Assert-P1AEqual $terminal.category 3 'Cuda terminal exit category changed'
        Assert-P1AEqual $terminal.run_id $diagnostic.run_id `
            'Cuda terminal error did not identify the failed run'
        Assert-P1ATest ([string]$terminal.evidence_path -match `
                ('^docs/receipts/P1B/runs/' + [regex]::Escape($terminal.run_id) + '/evidence\.json$')) `
            'terminal error did not expose the repository-relative evidence path'
    }

    Invoke-P1ATest 'concrete Cuda failure seals v2 evidence without selecting it' {
        $fakeRepository = New-P1ATestDirectory -Parent $temporaryRoot `
            -Name 'cuda-failure-repository'
        $fakeScripts = Join-Path $fakeRepository 'scripts'
        $fakeLibrary = Join-Path $fakeScripts 'lib'
        $outputRoot = Join-Path $fakeRepository 'docs\receipts\P1B'
        [void][IO.Directory]::CreateDirectory($fakeLibrary)
        [void][IO.Directory]::CreateDirectory($outputRoot)
        $fakeEntrypoint = Join-Path $fakeScripts 'verify-env.ps1'
        $fakeModule = Join-Path $fakeLibrary 'VerifyEnv.psm1'
        $fakeCudaImplementation = Join-Path $fakeLibrary 'VerifyCuda.ps1'
        [IO.File]::Copy($entrypointFullPath, $fakeEntrypoint, $false)
        [IO.File]::Copy($moduleFullPath, $fakeModule, $false)
        [IO.File]::Copy($p1bImplementationFullPath, $fakeCudaImplementation, $false)
        $pointerPath = Join-Path $outputRoot 'evidence.json'
        Write-P1AUtf8LfFile -Path $pointerPath -Text 'preexisting P1B pointer sentinel'
        $pointerHash = Get-P1ASha256 -Path $pointerPath

        $powerShell = Join-Path $PSHOME 'powershell.exe'
        $result = Invoke-P1AProcess -FilePath $powerShell -ArgumentList @(
            '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $fakeEntrypoint,
            '-Mode', 'Cuda', '-OutputRoot', 'docs\receipts\P1B'
        ) -WorkingDirectory $repositoryRoot -Environment @{} -TimeoutSeconds 60
        Assert-P1ATest ($result.exit_code -ge 1 -and $result.exit_code -le 5) `
            "concrete Cuda failure used an invalid exit category: $($result.exit_code)"
        Assert-P1ATest (-not [bool]$result.timed_out) `
            'concrete Cuda failure timed out'
        Assert-P1ATest ([string]::IsNullOrWhiteSpace([string]$result.stdout)) `
            'concrete Cuda failure wrote success JSON to stdout'
        $stderrLines = @([string]$result.stderr -split '\r?\n' | Where-Object {
                -not [string]::IsNullOrWhiteSpace($_)
            })
        Assert-P1ATest ($stderrLines.Count -ge 2) `
            "concrete Cuda failure omitted diagnostic or terminal JSON; stderr=<$($result.stderr)>"
        $records = @($stderrLines | ForEach-Object { $_ | ConvertFrom-Json })
        $terminal = $records[-1]
        Assert-P1AEqual $terminal.schema 'python-slm-script-error-v1' `
            'concrete Cuda terminal error schema changed'
        Assert-P1AEqual $terminal.phase_id 'P1B' `
            'concrete Cuda terminal error phase changed'
        Assert-P1AEqual $terminal.code 'QUALIFICATION_FAILED' `
            'concrete Cuda terminal error code changed'
        Assert-P1AEqual $terminal.category $result.exit_code `
            'concrete Cuda terminal error and process exit category disagree'

        Assert-P1AEqual (Get-P1ASha256 -Path $pointerPath) $pointerHash `
            'failed Cuda attempt moved or replaced the selected pointer'
        $acceptanceRoot = Join-Path $outputRoot 'acceptances'
        Assert-P1ATest (-not (Test-Path -LiteralPath $acceptanceRoot) -or
            @(Get-ChildItem -LiteralPath $acceptanceRoot -File -ErrorAction SilentlyContinue).Count -eq 0) `
            'failed Cuda attempt created an acceptance'
        $runsRoot = Join-Path $outputRoot 'runs'
        $runs = @(Get-ChildItem -LiteralPath $runsRoot -Directory -ErrorAction SilentlyContinue)
        Assert-P1AEqual $runs.Count 1 `
            'concrete Cuda failure did not create exactly one immutable run'
        $evidencePath = Join-Path $runs[0].FullName 'evidence.json'
        Assert-P1ATest (Test-Path -LiteralPath $evidencePath -PathType Leaf) `
            'concrete Cuda failure omitted terminal evidence.json'
        $evidence = Get-P1ATestStrictText -Path $evidencePath | ConvertFrom-Json
        Assert-P1AEqual $evidence.schema 'python-slm-phase-evidence-v2' `
            'concrete Cuda failure did not use v2 evidence'
        Assert-P1AEqual $evidence.phase_id 'P1B' 'concrete Cuda failure phase changed'
        Assert-P1AEqual $evidence.mode 'Cuda' 'concrete Cuda failure mode changed'
        Assert-P1AEqual $evidence.status 'FAIL' `
            'concrete Cuda failure overstated its status'
        Assert-P1ATest (@($evidence.errors | Where-Object { $null -ne $_ }).Count -ge 1) `
            'concrete Cuda failure sealed no diagnostic error'
        Assert-P1AEqual $terminal.run_id $evidence.run_id `
            'terminal error does not identify the concrete failed run'
        Assert-P1ATest ([string]$terminal.evidence_path -match `
                ('^docs/receipts/P1B/runs/' + [regex]::Escape($evidence.run_id) +
                    '/evidence\.json$')) `
            'terminal error does not point to concrete failed evidence'
        Assert-P1ATest (Test-P1ASeal -RunRoot $runs[0].FullName) `
            'concrete Cuda failed-run seal does not verify'
        Assert-P1AEqual $evidence.seal.coverage_rule 'all_run_files_except_seal' `
            'concrete Cuda failure used an incomplete seal coverage rule'
        Assert-P1ATest ((Get-P1ATestStrictText -Path `
                    (Join-Path $runs[0].FullName 'SHA256SUMS')) -match `
                '(?m)  evidence\.json$') `
            'concrete Cuda failed-run seal omitted terminal evidence'
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

Set-StrictMode -Version Latest

function New-P1BEmptyDependency {
    return [ordered]@{
        status = 'NOT_RUN'
        pointer_path = $null
        pointer_sha256 = $null
        acceptance_path = $null
        acceptance_sha256 = $null
        acceptance_sequence = $null
        run_path = $null
        run_evidence_sha256 = $null
        seal_path = $null
        seal_sha256 = $null
        environment_path = $null
        environment_sha256 = $null
        source_identity_sha256 = $null
        verifier_sha256 = $null
        schema_bundle_sha256 = $null
        review_closure_commit = $null
    }
}

function Write-P1BTerminalRun {
    param(
        [Parameter(Mandatory)][string]$RunRoot,
        [Parameter(Mandatory)][string]$RunId,
        [Parameter(Mandatory)][DateTime]$Started,
        [Parameter(Mandatory)][Collections.IList]$Commands,
        [Parameter(Mandatory)][Collections.IList]$Errors,
        [Parameter(Mandatory)][Collections.IDictionary]$Gates,
        [Parameter(Mandatory)][Collections.IDictionary]$P0,
        [Parameter(Mandatory)][Collections.IDictionary]$P1A,
        [AllowNull()][Collections.IDictionary]$EnvironmentReference,
        [Parameter(Mandatory)][string]$SourceIdentitySha256,
        [Parameter(Mandatory)][bool]$CleanupAttempted,
        [Parameter(Mandatory)][bool]$TemporaryRemoved,
        [Parameter(Mandatory)][ValidateSet('PASS', 'FAIL')][string]$Status
    )

    $finished = [DateTime]::UtcNow
    $evidence = [ordered]@{
        schema = 'python-slm-phase-evidence-v2'
        phase_id = 'P1B'
        run_id = $RunId
        mode = 'Cuda'
        status = $Status
        started_at = $Started.ToString('o')
        finished_at = $finished.ToString('o')
        duration_ms = [int64]($finished - $Started).TotalMilliseconds
        invocation = [ordered]@{
            argv = @('powershell', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
                'scripts/verify-env.ps1', '-Mode', 'Cuda', '-OutputRoot', 'docs/receipts/P1B')
            cwd = '${REPO}'
        }
        source_identity_sha256 = $SourceIdentitySha256
        p0_dependency = $P0
        p1a_dependency = $P1A
        environment = $EnvironmentReference
        commands = @($Commands)
        gates = $Gates
        errors = @($Errors)
        cleanup = [ordered]@{
            attempted = $CleanupAttempted
            temporary_root_removed = $TemporaryRemoved
        }
        seal = [ordered]@{
            path = 'SHA256SUMS'
            entries = @(Get-ChildItem -LiteralPath $RunRoot -Recurse -File).Count + 1
            coverage_rule = 'all_run_files_except_seal'
        }
    }
    $evidencePath = Join-Path $RunRoot 'evidence.json'
    Write-P1AJsonFile -Path $evidencePath -Value $evidence -CreateNew
    [void](New-P1ASeal -RunRoot $RunRoot)
    if (-not (Test-P1ASeal -RunRoot $RunRoot)) {
        throw '[EVIDENCE_SEAL_FAILED] P1B terminal run seal did not verify'
    }
    return $evidencePath
}

function Get-P1BVerifierBundle {
    param(
        [Parameter(Mandatory)][string]$RepositoryRoot,
        [Parameter(Mandatory)][string]$ScriptPath
    )
    $schemaFiles = @(Get-ChildItem -LiteralPath (Join-Path $RepositoryRoot 'docs\schemas') -File |
        Sort-Object Name | ForEach-Object { $_.Name + '  ' + (Get-P1ASha256 $_.FullName) })
    $schemaBundle = Get-P1ATextSha256 -Text ($schemaFiles -join $script:Lf)
    $verifierFiles = @(
        $ScriptPath,
        (Join-Path $PSScriptRoot 'VerifyEnv.psm1'),
        (Join-Path $PSScriptRoot 'VerifyCuda.ps1'),
        (Join-Path $RepositoryRoot 'scripts\tests\verify-env.tests.ps1')
    )
    $verifierBundle = Get-P1ATextSha256 -Text ((@($verifierFiles | ForEach-Object {
                    (ConvertTo-P1ANormalizedPath -Path $_ -BasePath $RepositoryRoot) + '  ' +
                    (Get-P1ASha256 -Path $_)
                } | Sort-Object)) -join $script:Lf)
    return [pscustomobject][ordered]@{
        verifier_sha256 = $verifierBundle
        schema_bundle_sha256 = $schemaBundle
    }
}

function Get-P1BVsEnvironment {
    param(
        [Parameter(Mandatory)][hashtable]$Context,
        [Parameter(Mandatory)][string]$TemporaryRoot,
        [Parameter(Mandatory)]$P1AEnvironment
    )
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere -PathType Leaf)) {
        throw '[VSWHERE_NOT_FOUND] supported Visual Studio discovery tool was not found'
    }
    $query = @('-version', '[17.0,18.0)', '-products', '*', '-requires',
        'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-format', 'json', '-utf8')
    $queryCommand = Invoke-P1ARecordedCommand -Context $Context -FilePath $vswhere `
        -ArgumentList $query -RecordedStdout 'VS2022 candidates discovered; raw installation paths intentionally omitted.'
    Assert-P1ACommand $queryCommand 'VSWHERE_FAILED' 'Visual Studio 2022 discovery failed'
    $instance = Select-P1AVs2022Instance -Instances @($queryCommand.result.stdout | ConvertFrom-Json)
    if ([string]$instance.installationVersion -cne [string]$P1AEnvironment.visual_studio.installation_version -or
        [string]$instance.instanceId -cne [string]$P1AEnvironment.visual_studio.instance_id) {
        throw '[P1A_TOOLCHAIN_DRIFT] selected Visual Studio instance differs from P1A'
    }
    $Context.RedactionRoots = @($Context.RedactionRoots) + @([string]$instance.installationPath)
    $vsDevCmd = Join-Path ([string]$instance.installationPath) 'Common7\Tools\VsDevCmd.bat'
    if (-not (Test-Path -LiteralPath $vsDevCmd -PathType Leaf)) {
        throw '[VSDEVCMD_NOT_FOUND] selected VS2022 instance has no VsDevCmd.bat'
    }
    $capture = Join-Path $TemporaryRoot 'capture-vs-env.cmd'
    Write-P1AUtf8LfFile -Path $capture -Text @'
@echo off
setlocal DisableDelayedExpansion
call "%~1" -no_logo -arch=amd64 -host_arch=amd64 -startdir=none >nul
if errorlevel 1 exit /b %errorlevel%
set
'@
    $cmd = Join-Path $env:SystemRoot 'System32\cmd.exe'
    $captureCommand = Invoke-P1ARecordedCommand -Context $Context -FilePath $cmd `
        -ArgumentList @('/d', '/c', $capture, $vsDevCmd) `
        -RecordedStdout 'VS2022 x64 developer environment captured; raw environment intentionally omitted.'
    Assert-P1ACommand $captureCommand 'VSDEVCMD_FAILED' 'VS2022 x64 developer environment initialization failed'
    $environment = @{}
    $seen = [Collections.Generic.Dictionary[string,string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($line in ($captureCommand.result.stdout -split '\r?\n')) {
        $separator = $line.IndexOf('=')
        if ($separator -le 0) { continue }
        $name = $line.Substring(0, $separator)
        $value = $line.Substring($separator + 1)
        if ($seen.ContainsKey($name) -and $seen[$name] -cne $value) {
            throw '[CHILD_ENVIRONMENT_CONFLICT] VsDevCmd emitted conflicting case variants'
        }
        $seen[$name] = $value
        $environment[$name] = $value
    }
    if ($environment['VSCMD_ARG_TGT_ARCH'] -cne 'x64' -or
        $environment['VSCMD_ARG_HOST_ARCH'] -cne 'x64') {
        throw '[VS_ARCH_MISMATCH] VsDevCmd did not produce x64 host and target identities'
    }
    $tools = @{}
    foreach ($name in @('cl', 'link', 'lib', 'dumpbin', 'rc', 'mt')) {
        $tools[$name] = Get-P1APathCommand -Name $name -Environment $environment
        if ([string]::IsNullOrWhiteSpace([string]$tools[$name])) {
            throw "[MSVC_TOOL_NOT_FOUND] required P1A tool was not resolved: $name"
        }
    }
    $sdkRoot = [string]$environment['WindowsSdkDir']
    $sdkVersion = ([string]$environment['WindowsSDKVersion']).TrimEnd('\', '/')
    $ucrtVersion = ([string]$environment['UCRTVersion']).TrimEnd('\', '/')
    if ([string]::IsNullOrWhiteSpace($sdkRoot) -or
        $sdkVersion -cne [string]$P1AEnvironment.windows_sdk.version -or
        $ucrtVersion -cne [string]$P1AEnvironment.windows_sdk.ucrt_version) {
        throw '[P1A_TOOLCHAIN_DRIFT] selected Windows SDK differs from P1A'
    }
    $Context.RedactionRoots = @($Context.RedactionRoots) + @($sdkRoot)
    $p1aTools = @{}
    foreach ($tool in @($P1AEnvironment.visual_studio.tools)) { $p1aTools[[string]$tool.name] = $tool }
    foreach ($name in @('cl', 'link', 'lib', 'dumpbin')) {
        if (-not (Test-P1APathWithin -Path $tools[$name] -Root ([string]$instance.installationPath)) -or
            -not $p1aTools.ContainsKey($name) -or
            (Get-P1ASha256 -Path $tools[$name]) -cne [string]$p1aTools[$name].sha256) {
            throw "[P1A_TOOLCHAIN_DRIFT] $name identity differs from P1A"
        }
    }
    $p1aSdkTools = @{}
    foreach ($tool in @($P1AEnvironment.windows_sdk.tools)) {
        $p1aSdkTools[[string]$tool.name] = $tool
    }
    foreach ($name in @('rc', 'mt')) {
        if (-not (Test-P1APathWithin -Path $tools[$name] -Root $sdkRoot) -or
            -not $p1aSdkTools.ContainsKey($name) -or
            (Get-P1ASha256 -Path $tools[$name]) -cne [string]$p1aSdkTools[$name].sha256) {
            throw "[P1A_TOOLCHAIN_DRIFT] $name identity differs from P1A"
        }
    }
    $expectedSdkPaths = @(
        (Join-Path $sdkRoot "Include\$sdkVersion\um\Windows.h"),
        (Join-Path $sdkRoot "Include\$sdkVersion\ucrt\stdlib.h"),
        (Join-Path $sdkRoot "Lib\$sdkVersion\um\x64\kernel32.lib"),
        (Join-Path $sdkRoot "Lib\$sdkVersion\ucrt\x64\ucrt.lib"),
        (Join-Path $sdkRoot "bin\$sdkVersion\x64\rc.exe"),
        (Join-Path $sdkRoot "bin\$sdkVersion\x64\mt.exe")
    )
    foreach ($path in $expectedSdkPaths) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw '[P1A_TOOLCHAIN_DRIFT] a P1A-qualified Windows SDK file is missing'
        }
    }
    $normalizedSdkPaths = @($expectedSdkPaths | ForEach-Object {
            '${WINDOWS_KITS}/' + (ConvertTo-P1ANormalizedPath -Path $_ -BasePath $sdkRoot)
        })
    if ((@($normalizedSdkPaths) -join "`n") -cne
        (@($P1AEnvironment.windows_sdk.validated_paths) -join "`n")) {
        throw '[P1A_TOOLCHAIN_DRIFT] Windows SDK validated path identity differs from P1A'
    }
    return [pscustomobject][ordered]@{
        instance = $instance
        query = $query
        environment = $environment
        tools = $tools
        sdk_root = $sdkRoot
        sdk_version = $sdkVersion
        ucrt_version = $ucrtVersion
    }
}

function New-P1BCudaChildEnvironment {
    param(
        [Parameter(Mandatory)]$Vs,
        [Parameter(Mandatory)]$Toolkit,
        [Parameter(Mandatory)][string]$TemporaryRoot,
        [Parameter(Mandatory)][string]$CanaryRoot,
        [Parameter(Mandatory)][string]$MarkerRoot
    )
    $environment = @{}
    foreach ($name in (Get-P1ACanonicalEnvironment).Keys) { $environment[[string]$name] = $null }
    $allow = '^(?i:Path|PATHEXT|SystemRoot|WINDIR|ComSpec|PROCESSOR_ARCHITECTURE|NUMBER_OF_PROCESSORS|ProgramFiles(?:\(x86\))?|ProgramW6432|INCLUDE|LIB|LIBPATH|VSINSTALLDIR|VCINSTALLDIR|VCToolsInstallDir|VCToolsVersion|VisualStudioVersion|VSCMD_.*|WindowsSdkDir|WindowsSDKVersion|WindowsSdkBinPath|WindowsSdkVerBinPath|UCRTVersion|UniversalCRTSdkDir|ExtensionSdkDir|FrameworkDir.*|FrameworkVersion.*|NETFXSDKDir|DevEnvDir)$'
    foreach ($name in @($Vs.environment.Keys | ForEach-Object { [string]$_ })) {
        if ($name -match $allow) { $environment[$name] = [string]$Vs.environment[$name] }
    }
    $environment['USERPROFILE'] = $TemporaryRoot
    $environment['HOME'] = $TemporaryRoot
    $environment['TEMP'] = $TemporaryRoot
    $environment['TMP'] = $TemporaryRoot
    $environment['CUDA_PATH'] = [string]$Toolkit.root
    $environment['CUDA_CACHE_PATH'] = Join-Path $TemporaryRoot 'cuda-cache'
    $environment['P1B_CANARY_DIR'] = $MarkerRoot
    foreach ($name in @('PYTHONHOME', 'PYTHONPATH', 'PIP_CONFIG_FILE', 'RUSTC_WRAPPER',
            'RUSTC_WORKSPACE_WRAPPER', 'SCCACHE_DIR', 'CL', '_CL_', 'LINK', '_LINK_')) {
        $environment[$name] = $null
    }
    $pathParts = [Collections.Generic.List[string]]::new()
    foreach ($path in @(
            $CanaryRoot,
            (Join-Path ([string]$Toolkit.root) 'bin\x64'),
            (Join-Path ([string]$Toolkit.root) 'bin'),
            (Split-Path -Parent ([string]$Vs.tools['cl'])),
            (Split-Path -Parent ([string]$Vs.tools['rc'])),
            (Join-Path $env:SystemRoot 'System32')
        )) {
        if (-not [string]::IsNullOrWhiteSpace($path) -and -not $pathParts.Contains($path)) {
            $pathParts.Add([IO.Path]::GetFullPath($path).TrimEnd('\', '/'))
        }
    }
    $environment['Path'] = $pathParts -join ';'
    return $environment
}

function Invoke-P1BArtifactInspectionCommands {
    param(
        [Parameter(Mandatory)][ValidateSet('Mixed', 'PtxOnly')][string]$Kind,
        [Parameter(Mandatory)][hashtable]$Context,
        [Parameter(Mandatory)]$Toolkit,
        [Parameter(Mandatory)]$Vs,
        [Parameter(Mandatory)][string]$Executable,
        [Parameter(Mandatory)][hashtable]$Environment,
        [Parameter(Mandatory)]$CompileCommand,
        [Parameter(Mandatory)][ValidateSet('Hybrid', 'Shared')][string]$RuntimeLinkage
    )
    $cuobjdump = [string]$Toolkit.tools['cuobjdump']
    $listElf = Invoke-P1ARecordedCommand -Context $Context -FilePath $cuobjdump `
        -ArgumentList @('--list-elf', $Executable) -Environment $Environment
    Assert-P1ACommand $listElf 'CUDA_INSPECTION_FAILED' 'cuobjdump --list-elf failed'
    $listPtx = Invoke-P1ARecordedCommand -Context $Context -FilePath $cuobjdump `
        -ArgumentList @('--list-ptx', $Executable) -Environment $Environment
    Assert-P1ACommand $listPtx 'CUDA_INSPECTION_FAILED' 'cuobjdump --list-ptx failed'
    $dumpSass = Invoke-P1ARecordedCommand -Context $Context -FilePath $cuobjdump `
        -ArgumentList @('--dump-sass', $Executable) -Environment $Environment -RecordDigestOnly
    Assert-P1ACommand $dumpSass 'CUDA_INSPECTION_FAILED' 'cuobjdump --dump-sass failed'
    $dumpPtx = Invoke-P1ARecordedCommand -Context $Context -FilePath $cuobjdump `
        -ArgumentList @('--dump-ptx', $Executable) -Environment $Environment -RecordDigestOnly
    Assert-P1ACommand $dumpPtx 'CUDA_INSPECTION_FAILED' 'cuobjdump --dump-ptx failed'
    $headers = Invoke-P1ARecordedCommand -Context $Context -FilePath $Vs.tools['dumpbin'] `
        -ArgumentList @('/HEADERS', $Executable) -Environment $Environment
    Assert-P1ACommand $headers 'CUDA_PE_INSPECTION_FAILED' 'dumpbin /HEADERS failed'
    $dependents = Invoke-P1ARecordedCommand -Context $Context -FilePath $Vs.tools['dumpbin'] `
        -ArgumentList @('/DEPENDENTS', $Executable) -Environment $Environment
    Assert-P1ACommand $dependents 'CUDA_PE_INSPECTION_FAILED' 'dumpbin /DEPENDENTS failed'
    $targets = Assert-P1BArtifactInspection -Kind $Kind `
        -ListElfText $listElf.result.stdout -ListPtxText $listPtx.result.stdout `
        -SassText $dumpSass.result.stdout -PtxText $dumpPtx.result.stdout
    $imports = @(Assert-P1BPeInspection -HeadersText $headers.result.stdout `
        -DependentsText $dependents.result.stdout -RuntimeLinkage $RuntimeLinkage)
    return [ordered]@{
        compile_command_id = [string]$CompileCommand.id
        executable_sha256 = Get-P1ASha256 -Path $Executable
        run_command_id = ''
        pe_headers_command_id = [string]$headers.id
        pe_dependents_command_id = [string]$dependents.id
        elf_command_id = [string]$listElf.id
        ptx_list_command_id = [string]$listPtx.id
        sass_dump_command_id = [string]$dumpSass.id
        ptx_dump_command_id = [string]$dumpPtx.id
        embedded_sass = @($targets.embedded_sass)
        embedded_ptx = @($targets.embedded_ptx)
        pe_imports = @($imports)
        result = 'PASS'
    }
}

function Invoke-P1BQualificationStages {
    param(
        [Parameter(Mandatory)][hashtable]$Context,
        [Parameter(Mandatory)][string]$RepositoryRoot,
        [Parameter(Mandatory)][string]$ScriptPath,
        [Parameter(Mandatory)][string]$TemporaryRoot,
        [Parameter(Mandatory)][string]$ArtifactsRoot,
        [Parameter(Mandatory)][Collections.IDictionary]$P1A,
        [Parameter(Mandatory)]$InputStart,
        [Parameter(Mandatory)][string]$CargoLockSha256
    )
    $bundles = Get-P1BVerifierBundle -RepositoryRoot $RepositoryRoot -ScriptPath $ScriptPath
    if ([string]$P1A.verifier_sha256 -cne $bundles.verifier_sha256 -or
        [string]$P1A.schema_bundle_sha256 -cne $bundles.schema_bundle_sha256) {
        throw '[P1A_BUNDLE_MISMATCH] selected P1A did not qualify the current verifier and schemas'
    }
    Set-P1AGate $Context.Gates cpu_regression PASS 'selected P1A verifier and schema bundle identities match P1B'

    $p1aEnvironmentPath = Join-Path (Join-Path $RepositoryRoot 'docs\receipts\P1A') `
        (([string]$P1A.environment_path).Substring('docs/receipts/P1A/'.Length).Replace('/', '\'))
    $p1aEnvironment = [IO.File]::ReadAllText($p1aEnvironmentPath, $script:Utf8NoBom) | ConvertFrom-Json

    $git = Get-P1AApplicationPath -Name 'git.exe'
    $headCommand = Invoke-P1ARecordedCommand -Context $Context -FilePath $git -ArgumentList @('rev-parse', 'HEAD')
    Assert-P1ACommand $headCommand 'GIT_HEAD_FAILED' 'could not resolve repository HEAD'
    $head = $headCommand.result.stdout.Trim()
    if ($head -notmatch '^[0-9a-f]{40}$') { throw '[GIT_HEAD_FAILED] repository HEAD was not a full object ID' }
    $statusCommand = Invoke-P1ARecordedCommand -Context $Context -FilePath $git `
        -ArgumentList @('status', '--porcelain=v1', '--untracked-files=all', '--', '.',
            ':(exclude)docs/receipts/P1A', ':(exclude)docs/receipts/P1B') -RecordDigestOnly
    Assert-P1ACommand $statusCommand 'GIT_STATUS_FAILED' 'could not inspect repository status'

    $rustc = Get-P1AApplicationPath -Name 'rustc.exe'
    $cargo = Get-P1AApplicationPath -Name 'cargo.exe'
    $rustCommand = Invoke-P1ARecordedCommand -Context $Context -FilePath $rustc -ArgumentList @('-Vv')
    Assert-P1ACommand $rustCommand 'RUSTC_FAILED' 'rustc -Vv failed'
    $rust = ConvertFrom-P1ARustcVersion -Text $rustCommand.result.stdout
    $cargoCommand = Invoke-P1ARecordedCommand -Context $Context -FilePath $cargo -ArgumentList @('-Vv')
    Assert-P1ACommand $cargoCommand 'CARGO_FAILED' 'cargo -Vv failed'
    $cargoVersionMatch = [regex]::Match($cargoCommand.result.stdout, '(?m)^release:\s*(?<v>\d+\.\d+\.\d+)')
    if (-not $cargoVersionMatch.Success) {
        $cargoVersionMatch = [regex]::Match($cargoCommand.result.stdout, '(?m)^cargo\s+(?<v>\d+\.\d+\.\d+)')
    }
    if ($rust.release -cne [string]$p1aEnvironment.rust.rustc_version -or
        $rust.host -cne [string]$p1aEnvironment.rust.host -or
        -not $cargoVersionMatch.Success -or
        $cargoVersionMatch.Groups['v'].Value -cne [string]$p1aEnvironment.rust.cargo_version -or
        (Get-P1ASha256 -Path $rustc) -cne [string]$p1aEnvironment.rust.rustc.sha256 -or
        (Get-P1ASha256 -Path $cargo) -cne [string]$p1aEnvironment.rust.cargo.sha256) {
        throw '[P1A_TOOLCHAIN_DRIFT] Rust or Cargo identity differs from selected P1A'
    }

    $vs = Get-P1BVsEnvironment -Context $Context -TemporaryRoot $TemporaryRoot `
        -P1AEnvironment $p1aEnvironment
    Set-P1AGate $Context.Gates toolchain PASS 'P1A-qualified Rust, VS2022 x64 MSVC, and Windows SDK identities match'

    $toolkit = Select-P1BCudaToolkit -Candidates @(Get-P1BCudaCandidates)
    $runtimeLinkageMode = if ([int]$toolkit.major -ge 13) { 'Hybrid' } else { 'Shared' }
    $activeRuntimeProvider = if ($runtimeLinkageMode -ceq 'Hybrid') {
        'display_driver'
    }
    else { 'toolkit_cudart' }
    $Context.RedactionRoots = @($Context.RedactionRoots) + @([string]$toolkit.root)
    $roots = @{
        REPO = $RepositoryRoot
        OUTPUT_ROOT = Split-Path -Parent $Context.RunRoot | Split-Path -Parent
        TEMP = $TemporaryRoot
        USERPROFILE = $env:USERPROFILE
        VS_INSTALL = [string]$vs.instance.installationPath
        VC_TOOLS = [string]$vs.environment['VCToolsInstallDir']
        WINDOWS_KITS = [string]$vs.sdk_root
        WINDOWS = $env:SystemRoot
        CARGO_HOME = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
        RUSTUP_HOME = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE '.rustup' }
        CUDA_TOOLKIT = [string]$toolkit.root
    }

    $canaryRoot = Join-Path $TemporaryRoot 'canaries'
    $markerRoot = Join-Path $TemporaryRoot 'markers'
    [void][IO.Directory]::CreateDirectory($canaryRoot)
    [void][IO.Directory]::CreateDirectory($markerRoot)
    $canaries = @('python', 'python3', 'pythonw', 'py', 'pyw', 'pip', 'pip3')
    foreach ($canary in $canaries) {
        $marker = Join-Path $markerRoot ($canary + '.hit')
        Write-P1AUtf8LfFile -Path (Join-Path $canaryRoot ($canary + '.cmd')) `
            -Text "@echo off`n> `"$marker`" echo invoked`nexit /b 97"
    }
    $childEnvironment = New-P1BCudaChildEnvironment -Vs $vs -Toolkit $toolkit `
        -TemporaryRoot $TemporaryRoot -CanaryRoot $canaryRoot -MarkerRoot $markerRoot

    $canarySourcePath = Join-Path $TemporaryRoot 'p1b_python_canary.c'
    $canaryExecutable = Join-Path $TemporaryRoot 'p1b_python_canary.exe'
    $canaryObject = Join-Path $TemporaryRoot 'p1b_python_canary.obj'
    Write-P1AUtf8LfFile -Path $canarySourcePath -CreateNew -Text @'
#include <windows.h>
int main(void) {
    char module[MAX_PATH];
    char directory[MAX_PATH];
    char marker[MAX_PATH];
    DWORD module_length = GetModuleFileNameA(NULL, module, MAX_PATH);
    DWORD directory_length = GetEnvironmentVariableA("P1B_CANARY_DIR", directory, MAX_PATH);
    DWORD base = 0, end = 0, i = 0;
    HANDLE file = INVALID_HANDLE_VALUE;
    if (module_length == 0 || module_length >= MAX_PATH ||
        directory_length == 0 || directory_length >= MAX_PATH) return 97;
    for (i = 0; i < module_length; ++i) {
        if (module[i] == '\\' || module[i] == '/') base = i + 1;
    }
    end = module_length;
    for (i = base; i < module_length; ++i) {
        if (module[i] == '.') { end = i; break; }
    }
    if (directory_length + 1 + (end - base) + 4 >= MAX_PATH) return 97;
    for (i = 0; i < directory_length; ++i) marker[i] = directory[i];
    marker[directory_length++] = '\\';
    for (i = base; i < end; ++i) marker[directory_length++] = module[i];
    marker[directory_length++] = '.';
    marker[directory_length++] = 'h';
    marker[directory_length++] = 'i';
    marker[directory_length++] = 't';
    marker[directory_length] = '\0';
    file = CreateFileA(marker, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS,
        FILE_ATTRIBUTE_NORMAL, NULL);
    if (file != INVALID_HANDLE_VALUE) CloseHandle(file);
    return 97;
}
'@
    $canaryCompile = Invoke-P1ARecordedCommand -Context $Context -FilePath $vs.tools['cl'] `
        -ArgumentList @('/nologo', '/TC', '/W4', '/WX', '/MD', $canarySourcePath,
            ('/Fo' + $canaryObject), ('/Fe' + $canaryExecutable), '/link', 'kernel32.lib') `
        -Environment $childEnvironment -TimeoutSeconds 120
    Assert-P1ACommand $canaryCompile 'PYTHON_CANARY_BUILD_FAILED' 'native Python canary compilation failed'
    foreach ($canary in $canaries) {
        [IO.File]::Copy($canaryExecutable, (Join-Path $canaryRoot ($canary + '.exe')), $false)
    }

    $stagedComponents = if ($runtimeLinkageMode -ceq 'Hybrid') {
        @('cublas', 'cublaslt')
    }
    else { @('cudart', 'cublas', 'cublaslt') }
    $stagedRuntimeDlls = @()
    foreach ($component in $stagedComponents) {
        $sourceDll = [string]$toolkit.runtime_dlls[$component]
        $stagedDll = Join-Path $TemporaryRoot (Split-Path -Leaf $sourceDll)
        [IO.File]::Copy($sourceDll, $stagedDll, $false)
        $sourceHash = Get-P1ASha256 -Path $sourceDll
        if ((Get-P1ASha256 -Path $stagedDll) -cne $sourceHash) {
            throw '[CUDA_RUNTIME_STAGING_FAILED] staged CUDA runtime DLL hash differs from the selected toolkit'
        }
        $stagedRuntimeDlls += [ordered]@{
            component = $component
            sha256 = $sourceHash
        }
    }

    $nvccVersionCommand = Invoke-P1ARecordedCommand -Context $Context `
        -FilePath $toolkit.tools['nvcc'] -ArgumentList @('--version') -Environment $childEnvironment
    Assert-P1ACommand $nvccVersionCommand 'CUDA_NVCC_FAILED' 'nvcc --version failed'
    $nvcc = ConvertFrom-P1BNvccVersion -Text $nvccVersionCommand.result.stdout
    if ($nvcc.major -ne $toolkit.major -or $nvcc.minor -ne $toolkit.minor) {
        throw '[CUDA_MIXED_ROOT] nvcc version does not match the selected CUDA toolkit root'
    }
    $archCommand = Invoke-P1ARecordedCommand -Context $Context -FilePath $toolkit.tools['nvcc'] `
        -ArgumentList @('--list-gpu-arch') -Environment $childEnvironment
    Assert-P1ACommand $archCommand 'CUDA_TARGET_QUERY_FAILED' 'nvcc --list-gpu-arch failed'
    $codeCommand = Invoke-P1ARecordedCommand -Context $Context -FilePath $toolkit.tools['nvcc'] `
        -ArgumentList @('--list-gpu-code') -Environment $childEnvironment
    Assert-P1ACommand $codeCommand 'CUDA_TARGET_QUERY_FAILED' 'nvcc --list-gpu-code failed'
    $targets = Get-P1BNvccTargets -ArchitectureText $archCommand.result.stdout -CodeText $codeCommand.result.stdout
    Set-P1AGate $Context.Gates cuda_toolkit PASS 'coherent CUDA toolkit, required tools, headers, libraries, and runtime DLLs verified'

    $nvidiaSmi = Join-Path $env:SystemRoot 'System32\nvidia-smi.exe'
    $nvcuda = Join-Path $env:SystemRoot 'System32\nvcuda.dll'
    foreach ($path in @($nvidiaSmi, $nvcuda)) {
        if (-not (Test-P1BContainedPathWithoutReparse -Path $path -Root $env:SystemRoot -Leaf)) {
            throw '[CUDA_DRIVER_BOUNDARY_MISSING] NVIDIA driver tool or library is missing or unsafe'
        }
    }
    $smiCommand = Invoke-P1ARecordedCommand -Context $Context -FilePath $nvidiaSmi `
        -ArgumentList @('--query-gpu=index,name,driver_version,memory.total,compute_cap',
            '--format=csv,noheader,nounits') -Environment $childEnvironment
    Assert-P1ACommand $smiCommand 'GPU_INVENTORY_FAILED' 'restricted nvidia-smi inventory failed'
    $inventory = ConvertFrom-P1BNvidiaSmi -Text $smiCommand.result.stdout
    Set-P1AGate $Context.Gates gpu_identity PASS 'exactly one runtime-visible RTX 5090 with compute capability 12.0 selected'

    $probeSource = New-P1BProbeSource
    $probePath = Join-Path $TemporaryRoot 'p1b_cuda_probe.cu'
    $mixedExe = Join-Path $TemporaryRoot 'p1b_cuda_probe_mixed.exe'
    $ptxExe = Join-Path $TemporaryRoot 'p1b_cuda_probe_ptx.exe'
    Write-P1AUtf8LfFile -Path $probePath -Text $probeSource -CreateNew
    $compilerDirectory = Split-Path -Parent ([string]$vs.tools['cl'])
    $mixedCompile = Invoke-P1ARecordedCommand -Context $Context -FilePath $toolkit.tools['nvcc'] `
        -ArgumentList @(New-P1BNvccArguments -Kind Mixed -SourcePath $probePath `
            -OutputPath $mixedExe -CudaRoot $toolkit.root -CompilerDirectory $compilerDirectory `
            -ToolkitMajor ([int]$toolkit.major)) `
        -Environment $childEnvironment -TimeoutSeconds 120
    Assert-P1ACommand $mixedCompile 'CUDA_COMPILE_FAILED' 'mixed SM120/PTX probe compilation failed'
    $ptxCompile = Invoke-P1ARecordedCommand -Context $Context -FilePath $toolkit.tools['nvcc'] `
        -ArgumentList @(New-P1BNvccArguments -Kind PtxOnly -SourcePath $probePath `
            -OutputPath $ptxExe -CudaRoot $toolkit.root -CompilerDirectory $compilerDirectory `
            -ToolkitMajor ([int]$toolkit.major)) `
        -Environment $childEnvironment -TimeoutSeconds 120
    Assert-P1ACommand $ptxCompile 'CUDA_COMPILE_FAILED' 'PTX-only probe compilation failed'
    foreach ($compile in @($mixedCompile, $ptxCompile)) {
        if (([string]$compile.result.stdout -match '(?i)\bwarning\b') -or
            ([string]$compile.result.stderr -match '(?i)\bwarning\b')) {
            throw '[CUDA_COMPILE_WARNING] CUDA probe compilation emitted a warning'
        }
    }

    $mixedInspection = Invoke-P1BArtifactInspectionCommands -Kind Mixed -Context $Context `
        -Toolkit $toolkit -Vs $vs -Executable $mixedExe -Environment $childEnvironment `
        -CompileCommand $mixedCompile -RuntimeLinkage $runtimeLinkageMode
    $ptxInspection = Invoke-P1BArtifactInspectionCommands -Kind PtxOnly -Context $Context `
        -Toolkit $toolkit -Vs $vs -Executable $ptxExe -Environment $childEnvironment `
        -CompileCommand $ptxCompile -RuntimeLinkage $runtimeLinkageMode
    Set-P1AGate $Context.Gates architecture_targets PASS 'mixed artifact contains SM120 SASS plus compute_120 PTX; fallback artifact contains PTX only'

    $mixedRun = Invoke-P1ARecordedCommand -Context $Context -FilePath $mixedExe `
        -ArgumentList @() -Environment $childEnvironment -TimeoutSeconds 120
    Assert-P1ACommand $mixedRun 'CUDA_DEVICE_PROBE_FAILED' 'mixed CUDA probe execution failed'
    $mixedResult = Assert-P1BProbeResult -Value ($mixedRun.result.stdout | ConvertFrom-Json)
    $ptxRun = Invoke-P1ARecordedCommand -Context $Context -FilePath $ptxExe `
        -ArgumentList @() -Environment $childEnvironment -TimeoutSeconds 120
    Assert-P1ACommand $ptxRun 'CUDA_PTX_JIT_FAILED' 'PTX-only CUDA probe execution failed'
    $ptxResult = Assert-P1BProbeResult -Value ($ptxRun.result.stdout | ConvertFrom-Json)
    $mixedInspection.run_command_id = [string]$mixedRun.id
    $ptxInspection.run_command_id = [string]$ptxRun.id
    foreach ($field in @('device_index', 'device_name', 'memory_total_bytes',
            'compute_capability_major', 'compute_capability_minor', 'runtime_version',
            'driver_version', 'cublas_version', 'cublaslt_version', 'sentinel')) {
        if ([string]$mixedResult.$field -cne [string]$ptxResult.$field) {
            throw "[CUDA_PROBE_DIVERGENCE] mixed and PTX-only results differ: $field"
        }
    }
    if ([int]$mixedResult.device_index -ne [int]$inventory.target.index -or
        [string]$mixedResult.device_name -cne [string]$inventory.target.name) {
        throw '[GPU_IDENTITY_MISMATCH] CUDA runtime and nvidia-smi selected different devices'
    }
    Set-P1AGate $Context.Gates driver_runtime PASS 'CUDA driver/runtime API compatibility and cuBLAS/cuBLASLt initialization verified'
    Set-P1AGate $Context.Gates device_probe PASS 'mixed and PTX-only allocation, launch, synchronization, sentinel, cleanup, and reset passed'

    $canaryHits = @(Get-ChildItem -LiteralPath $markerRoot -Filter *.hit -File -ErrorAction SilentlyContinue |
        ForEach-Object { $_.BaseName } | Sort-Object -Unique)
    if ($canaryHits.Count -ne 0) { throw '[PYTHON_INVOKED] CUDA qualification invoked a Python canary' }
    Set-P1AGate $Context.Gates python_isolation PASS 'no Python or pip canary was invoked'

    $toolDescriptors = @()
    foreach ($name in @('nvcc', 'ptxas', 'fatbinary', 'nvlink', 'cuobjdump')) {
        $version = if ($name -ceq 'nvcc') { $nvcc.compiler_version } else { [string]$toolkit.version }
        $toolDescriptors += New-P1AToolDescriptor -Name $name -Version $version `
            -Path $toolkit.tools[$name] -Roots $roots
    }
    $headers = @('cuda', 'cuda_runtime', 'cublas', 'cublaslt') | ForEach-Object {
        New-P1BComponentDescriptor -Path $toolkit.headers[$_] -Roots $roots
    }
    $libraries = @('cuda', 'cudart', 'cublas', 'cublaslt') | ForEach-Object {
        New-P1BComponentDescriptor -Path $toolkit.libraries[$_] -Roots $roots
    }
    $runtimeDlls = @('cudart', 'cublas', 'cublaslt') | ForEach-Object {
        New-P1BComponentDescriptor -Path $toolkit.runtime_dlls[$_] -Roots $roots
    }
    $smiDescriptor = New-P1AToolDescriptor -Name 'nvidia-smi' `
        -Version ([string]$inventory.target.driver_version) -Path $nvidiaSmi -Roots $roots
    $nvcudaVersion = [Diagnostics.FileVersionInfo]::GetVersionInfo($nvcuda).FileVersion
    $nvcudaDescriptor = New-P1AToolDescriptor -Name 'nvcuda' -Version $nvcudaVersion `
        -Path $nvcuda -Roots $roots

    $sourceIdentity = [ordered]@{
        schema = 'python-slm-source-identity-v1'
        head = $head
        dirty = -not [string]::IsNullOrWhiteSpace([string]$statusCommand.result.stdout)
        input_manifest_sha256 = $InputStart.sha256
        cargo_lock_sha256 = $CargoLockSha256
        verifier_bundle_sha256 = $bundles.verifier_sha256
        files = $InputStart.entries
    }
    $environment = [ordered]@{
        schema = 'python-slm-cuda-environment-manifest-v1'
        phase_id = 'P1B'
        run_id = Split-Path -Leaf $Context.RunRoot
        mode = 'Cuda'
        status = 'PASS'
        p0 = [ordered]@{
            receipt_commit = '86fb1e4cc68efeb651e5362c4aca85c2827d8e4d'
            receipt_sha256 = 'f08c6a41658ff287e238d6a96c4f2c874975964202c3eeced2bc0bc21f308904'
            contract_sha256 = 'fc2c60b52fdd7c524e0da06bb03972a4d523c21ad5536cba536185435bd44ad4'
            decision_ledger_sha256 = '8349d8a3e06d96d6921889de5534715e7b2f7439caf7e06558a97652a8890c8d'
        }
        p1a = $P1A
        source = [ordered]@{
            head = $head
            dirty = $sourceIdentity.dirty
            input_manifest_sha256 = $InputStart.sha256
            cargo_lock_sha256 = $CargoLockSha256
            verifier_sha256 = $bundles.verifier_sha256
            schema_bundle_sha256 = $bundles.schema_bundle_sha256
        }
        host = $p1aEnvironment.host
        rust = $p1aEnvironment.rust
        visual_studio = $p1aEnvironment.visual_studio
        windows_sdk = $p1aEnvironment.windows_sdk
        cuda_toolkit = [ordered]@{
            version = [string]$toolkit.version
            root = '${CUDA_TOOLKIT}'
            tools = @($toolDescriptors)
            headers = @($headers)
            libraries = @($libraries)
            runtime_dlls = @($runtimeDlls)
        }
        runtime_linkage = [ordered]@{
            mode = $runtimeLinkageMode.ToLowerInvariant()
            active_provider = $activeRuntimeProvider
            staged_runtime_dlls = @($stagedRuntimeDlls)
        }
        optional_components = [ordered]@{
            cudnn = New-P1BOptionalComponentRecord cudnn $toolkit $roots
            nvrtc = New-P1BOptionalComponentRecord nvrtc $toolkit $roots
            nvjitlink = New-P1BOptionalComponentRecord nvjitlink $toolkit $roots
            compute_sanitizer = New-P1BOptionalComponentRecord compute_sanitizer $toolkit $roots
            curand = New-P1BOptionalComponentRecord curand $toolkit $roots
        }
        driver = [ordered]@{
            driver_version = [string]$inventory.target.driver_version
            cuda_umd_version = $nvcudaVersion
            nvidia_smi = $smiDescriptor
            library = $nvcudaDescriptor
        }
        gpu = [ordered]@{
            index = [int]$mixedResult.device_index
            name = [string]$mixedResult.device_name
            memory_total_bytes = [int64]$mixedResult.memory_total_bytes
            compute_capability_major = [int]$mixedResult.compute_capability_major
            compute_capability_minor = [int]$mixedResult.compute_capability_minor
        }
        architecture_targets = [ordered]@{
            sass = @('sm_120')
            ptx = @('compute_120')
            advertised_architectures = @($targets.architectures)
            advertised_code = @($targets.code)
            mixed = $mixedInspection
            ptx_only = $ptxInspection
        }
        device_probe = [ordered]@{
            source_sha256 = Get-P1ASha256 -Path $probePath
            runtime_version = [int]$mixedResult.runtime_version
            driver_version = [int]$mixedResult.driver_version
            cublas_version = [int]$mixedResult.cublas_version
            cublaslt_version = [int64]$mixedResult.cublaslt_version
            mixed_result = 'PASS'
            ptx_only_result = 'PASS'
            sentinel = 42
            target_match_count = 1
        }
        isolation = [ordered]@{
            temporary_root_absent_before = $true
            temporary_root_outside_repository = $true
            python_canaries = @($canaries)
            canary_hits = @()
            python_invoked = $false
        }
        cleanup = $null
    }
    Write-P1AJsonFile -Path (Join-Path $ArtifactsRoot 'p1a-dependency.json') -Value $P1A -CreateNew
    Write-P1AJsonFile -Path (Join-Path $ArtifactsRoot 'cuda-toolkit.json') `
        -Value ([ordered]@{ schema = 'python-slm-cuda-toolkit-inventory-v1'; status = 'PASS'; toolkit = $environment.cuda_toolkit; runtime_linkage = $environment.runtime_linkage; optional_components = $environment.optional_components }) -CreateNew
    Write-P1AJsonFile -Path (Join-Path $ArtifactsRoot 'gpu-runtime.json') `
        -Value ([ordered]@{ schema = 'python-slm-gpu-runtime-v1'; status = 'PASS'; driver = $environment.driver; gpu = $environment.gpu }) -CreateNew
    Write-P1AJsonFile -Path (Join-Path $ArtifactsRoot 'cuda-probe.json') `
        -Value ([ordered]@{ schema = 'python-slm-cuda-probe-evidence-v1'; status = 'PASS'; device_probe = $environment.device_probe; architecture_targets = $environment.architecture_targets }) -CreateNew
    return [pscustomobject][ordered]@{
        source_identity = $sourceIdentity
        environment = $environment
    }
}

function Invoke-P1BVerification {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$OutputRoot,
        [Parameter(Mandatory)][string]$RepositoryRoot,
        [Parameter(Mandatory)][string]$ScriptPath,
        [Parameter(Mandatory)][DateTime]$Started
    )

    $repository = [IO.Path]::GetFullPath($RepositoryRoot).TrimEnd('\', '/')
    $output = [IO.Path]::GetFullPath($OutputRoot).TrimEnd('\', '/')
    [void][IO.Directory]::CreateDirectory((Join-Path $output 'runs'))
    $runId = New-P1ARunId
    $runRoot = Join-Path (Join-Path $output 'runs') $runId
    $commandsRoot = Join-Path $runRoot 'commands'
    $artifactsRoot = Join-Path $runRoot 'artifacts'
    [void](New-Item -ItemType Directory -Path $runRoot)
    [void](New-Item -ItemType Directory -Path $commandsRoot)
    [void](New-Item -ItemType Directory -Path $artifactsRoot)

    $commands = [Collections.ArrayList]::new()
    $errors = [Collections.ArrayList]::new()
    $gates = New-P1BGates
    $activeGate = 'p0'
    $p0 = [ordered]@{
        status = 'FAIL'
        receipt_commit = '86fb1e4cc68efeb651e5362c4aca85c2827d8e4d'
        receipt_sha256 = 'f08c6a41658ff287e238d6a96c4f2c874975964202c3eeced2bc0bc21f308904'
        contract_sha256 = 'fc2c60b52fdd7c524e0da06bb03972a4d523c21ad5536cba536185435bd44ad4'
        decision_ledger_sha256 = '8349d8a3e06d96d6921889de5534715e7b2f7439caf7e06558a97652a8890c8d'
    }
    $p1a = New-P1BEmptyDependency
    $temporaryBase = $null
    $temporaryRoot = $null
    $cleanupAttempted = $false
    $temporaryRemoved = $true
    $status = 'FAIL'
    $qualification = $null
    $inputStart = $null
    $targetBefore = $null
    $repositoryBefore = $null
    $parentEnvironmentBefore = $null
    $repositoryTarget = Join-Path $repository 'target'
    $cargoLockSha256 = $null
    $sourceIdentityPath = Join-Path $artifactsRoot 'source-identity.json'
    $sourceIdentitySha = $null
    $environmentRef = $null
    $context = @{
        RepositoryRoot = $repository
        RunRoot = $runRoot
        CommandsRoot = $commandsRoot
        Commands = $commands
        CommandNumber = 0
        Gates = $gates
        RedactionRoots = @(
            $output, $env:USERPROFILE, $env:SystemRoot, $env:WINDIR, $env:TEMP, $env:TMP
        ) | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) }
    }

    try {
        if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or
            -not [Environment]::Is64BitProcess -or $env:PROCESSOR_ARCHITECTURE -cne 'AMD64') {
            throw '[UNSUPPORTED_HOST] P1B requires 64-bit Windows on AMD64'
        }
        if ($PSVersionTable.PSVersion.Major -ne 5 -or $PSVersionTable.PSVersion.Minor -ne 1) {
            throw '[UNSUPPORTED_POWERSHELL] P1B requires Windows PowerShell 5.1'
        }
        $temporaryBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\', '/')
        $temporaryRoot = Join-Path $temporaryBase ('python-slm-p1b-' + $runId)
        if (-not (Test-P1APathWithin -Path $temporaryRoot -Root $temporaryBase) -or
            (Test-P1APathWithin -Path $temporaryRoot -Root $repository) -or
            (Test-Path -LiteralPath $temporaryRoot)) {
            throw '[TEMP_PATH_NOT_FRESH] P1B temporary root is unsafe or already exists'
        }
        [void](New-Item -ItemType Directory -Path $temporaryRoot)
        $temporaryRemoved = $false
        $context.RedactionRoots = @($context.RedactionRoots) + @($temporaryRoot)

        $inputStart = Get-P1AInputManifest -RepositoryRoot $repository -ScriptPath $ScriptPath
        $targetBefore = Get-P1ADirectoryFingerprint -Path $repositoryTarget
        $repositoryBefore = Get-P1ARepositoryFingerprint -RepositoryRoot $repository -OutputRoot $output
        $parentEnvironmentBefore = Get-P1AEnvironmentFingerprint
        $cargoLockSha256 = Get-P1ASha256 -Path (Join-Path $repository 'Cargo.lock')
        if ($cargoLockSha256 -cne 'c0a5e1afe02e252a22cca8bf07ab37fb2a25844210d2d5ce2e1e6082e497a28c') {
            throw '[CARGO_LOCK_IDENTITY_MISMATCH] Cargo.lock differs from the approved P0 identity'
        }

        $git = Get-P1AApplicationPath -Name 'git.exe'
        $p0 = Test-P1AP0Dependency -Context $context -GitPath $git -Environment @{}
        $context.CommandNumber = $commands.Count
        Set-P1AGate $gates p0 PASS 'pinned P0 receipt, approvals, hashes, ancestry, and seal verified'

        $activeGate = 'p1a'
        $p1a = Get-P1BSelectedP1ADependency -RepositoryRoot $repository
        Set-P1AGate $gates p1a PASS 'selected P1A pointer, acceptance chain, run seal, and review closure verified'
        $activeGate = 'cuda_toolkit'
        $qualification = Invoke-P1BQualificationStages -Context $context `
            -RepositoryRoot $repository -ScriptPath $ScriptPath -TemporaryRoot $temporaryRoot `
            -ArtifactsRoot $artifactsRoot -P1A $p1a -InputStart $inputStart `
            -CargoLockSha256 $cargoLockSha256
        $status = 'PASS'
    }
    catch {
        $rawMessage = [string]$_.Exception.Message
        $code = if ($rawMessage -match '^\[(?<code>[A-Z0-9_]+)\]\s*(?<body>.*)$') {
            $rawMessage = $Matches['body']
            $Matches['code']
        }
        else { 'UNEXPECTED_VERIFIER_FAILURE' }
        $message = Protect-P1AText -Text $rawMessage -RepositoryRoot $repository `
            -ExtraRoots @($context.RedactionRoots)
        $category = if ($_.Exception.Data.Contains('P1ACategory')) {
            [int]$_.Exception.Data['P1ACategory']
        }
        else { Get-P1AErrorCategory -Code $code }
        Add-P1AError -Errors $errors -Code $code -Category $category -Message $message `
            -Remediation 'Correct the reported CUDA environment prerequisite, then rerun the exact P1B command.'
        $failedGate = if ($code -match '^P1A_') { 'p1a' }
            elseif ($code -match '^(RUST|CARGO|VS|MSVC|WINDOWS_SDK|CHILD_ENVIRONMENT)') { 'toolchain' }
            elseif ($code -match '^(GPU_)') { 'gpu_identity' }
            elseif ($code -match '^(CUDA_(?:DEVICE|PROBE|PTX_JIT))') { 'device_probe' }
            elseif ($code -match '^(CUDA_(?:TARGET|SASS|PTX_INSPECTION|PTX_ONLY|PE|INSPECTION))') { 'architecture_targets' }
            elseif ($code -match '^PYTHON_') { 'python_isolation' }
            else { $activeGate }
        if ($gates.Contains($failedGate)) {
            Set-P1AGate $gates $failedGate FAIL $message
        }
        $status = 'FAIL'
    }
    finally {
        $cleanupAttempted = $true
        try {
            if (-not [string]::IsNullOrWhiteSpace($temporaryRoot) -and
                (Test-Path -LiteralPath $temporaryRoot)) {
                $resolvedTemp = (Resolve-Path -LiteralPath $temporaryRoot -ErrorAction Stop).Path
                if ([string]::IsNullOrWhiteSpace($temporaryBase) -or
                    -not (Test-P1APathWithin -Path $resolvedTemp -Root $temporaryBase) -or
                    (Split-Path -Leaf $resolvedTemp) -cnotmatch '^python-slm-p1b-[0-9]{8}T[0-9]{9}Z-[0-9a-f]{24}$') {
                    throw 'unsafe P1B temporary cleanup target'
                }
                Remove-Item -LiteralPath $resolvedTemp -Recurse -Force -ErrorAction Stop
            }
            $temporaryRemoved = [string]::IsNullOrWhiteSpace($temporaryRoot) -or
                -not (Test-Path -LiteralPath $temporaryRoot)
        }
        catch {
            $temporaryRemoved = $false
            Add-P1AError -Errors $errors -Code 'TEMP_CLEANUP_FAILED' -Category 5 `
                -Message 'P1B temporary root could not be safely removed' `
                -Remediation 'Inspect and remove only the exact verifier-owned temporary directory.'
        }
    }

    $inputsUnchanged = $false
    $targetUnchanged = $false
    $repositoryUnchanged = $false
    $parentEnvironmentUnchanged = $false
    if ($null -ne $inputStart) {
        try {
            $inputFinal = Get-P1AInputManifest -RepositoryRoot $repository -ScriptPath $ScriptPath
            $inputsUnchanged = $inputFinal.sha256 -ceq $inputStart.sha256
            $targetUnchanged = (Get-P1ADirectoryFingerprint -Path $repositoryTarget) -ceq $targetBefore
            $repositoryUnchanged = (Get-P1ARepositoryFingerprint -RepositoryRoot $repository -OutputRoot $output) -ceq $repositoryBefore
            $parentEnvironmentUnchanged = (Get-P1AEnvironmentFingerprint) -ceq $parentEnvironmentBefore
        }
        catch {
            $status = 'FAIL'
            Add-P1AError -Errors $errors -Code 'INPUT_STABILITY_CHECK_FAILED' -Category 3 `
                -Message 'P1B could not verify final repository and environment state' `
                -Remediation 'Restore the repository and rerun the exact P1B command.'
        }
    }
    if (-not $inputsUnchanged -or -not $targetUnchanged -or -not $repositoryUnchanged -or
        -not $parentEnvironmentUnchanged) {
        $status = 'FAIL'
        Set-P1AGate $gates input_stability FAIL 'inputs, target, repository, or parent environment changed'
        if (@($errors | Where-Object { [string]$_['code'] -ceq 'INPUT_STABILITY_CHECK_FAILED' }).Count -eq 0) {
            Add-P1AError -Errors $errors -Code 'INPUT_CHANGED_DURING_RUN' -Category 3 `
                -Message 'P1B inputs, target, repository, or parent environment changed during qualification' `
                -Remediation 'Restore the concurrent change and rerun the exact P1B command.'
        }
    }
    else { Set-P1AGate $gates input_stability PASS 'inputs, target, repository, Cargo.lock, and parent environment remained stable' }
    if ($temporaryRemoved) { Set-P1AGate $gates cleanup PASS 'verifier-owned temporary root was removed' }
    else {
        $status = 'FAIL'
        Set-P1AGate $gates cleanup FAIL 'verifier-owned temporary root was not removed'
    }

    if ($status -eq 'PASS') {
        $qualification.environment.cleanup = [ordered]@{
            temporary_root_removed = $true
            repository_target_unchanged = $true
            parent_environment_unchanged = $true
            inputs_unchanged = $true
        }
        Write-P1AJsonFile -Path $sourceIdentityPath -Value $qualification.source_identity -CreateNew
        $sourceIdentitySha = Get-P1ASha256 -Path $sourceIdentityPath
        $environmentPath = Join-Path $artifactsRoot 'environment.json'
        Write-P1AJsonFile -Path $environmentPath -Value $qualification.environment -CreateNew
        $environmentRef = [ordered]@{
            path = 'artifacts/environment.json'
            sha256 = Get-P1ASha256 -Path $environmentPath
            bytes = [int64](Get-Item -LiteralPath $environmentPath).Length
        }
        $leaks = [Collections.Generic.List[string]]::new()
        $sensitiveValues = @($repository, $output, $env:USERPROFILE, $env:USERNAME,
            $env:COMPUTERNAME) + @($context.RedactionRoots)
        foreach ($file in Get-ChildItem -LiteralPath $runRoot -File -Recurse) {
            $text = [IO.File]::ReadAllText($file.FullName, $script:Utf8NoBom)
            foreach ($value in @($sensitiveValues | Where-Object {
                        -not [string]::IsNullOrWhiteSpace([string]$_)
                    } | Sort-Object -Unique)) {
                if ($text -match [regex]::Escape([string]$value)) {
                    $leaks.Add((ConvertTo-P1ANormalizedPath $file.FullName $runRoot))
                }
            }
            if ($text -match '(?i)(?:^|[\s"''])[A-Z]:[\\/]' -or
                $text -match '(?i)(?:^|[\s"''])\\\\[^\\\s]+' -or
                $text -match '(?i)(?:authorization|api[_-]?key|access[_-]?token|password)\s*[:=]\s*[^\s,;}]+') {
                $leaks.Add((ConvertTo-P1ANormalizedPath $file.FullName $runRoot))
            }
        }
        if ($leaks.Count -ne 0) {
            $status = 'FAIL'
            Set-P1AGate $gates redaction FAIL 'receipt artifacts contained prohibited machine or secret-shaped values'
            Add-P1AError -Errors $errors -Code 'REDACTION_FAILED' -Category 3 `
                -Message 'receipt artifacts contained prohibited machine or secret-shaped values' `
                -Remediation 'Fix path tokenization or output redaction, then rerun.'
            foreach ($relative in @($leaks | Sort-Object -Unique)) {
                $path = Join-Path $runRoot $relative.Replace('/', '\')
                if (Test-P1APathWithin -Path $path -Root $runRoot) {
                    Write-P1AUtf8LfFile -Path $path -Text `
                        $(if ([IO.Path]::GetExtension($path) -ieq '.json') {
                                '{"schema":"python-slm-redacted-artifact-v1","status":"REDACTED"}'
                            }
                            else { 'P1B artifact removed because the redaction gate failed.' })
                }
            }
            foreach ($command in @($commands)) {
                foreach ($stream in @('stdout', 'stderr')) {
                    $path = Join-Path $runRoot ([string]$command[$stream].path).Replace('/', '\')
                    $command[$stream].sha256 = Get-P1ASha256 -Path $path
                    $command[$stream].bytes = [int64](Get-Item -LiteralPath $path).Length
                }
            }
            $environmentRef = $null
            $sourceIdentitySha = Get-P1ASha256 -Path $sourceIdentityPath
        }
        else { Set-P1AGate $gates redaction PASS 'receipts contain only tokenized paths and no host, user, credential, or secret values' }
    }

    if ($status -eq 'PASS') {
        $evidencePath = Write-P1BTerminalRun -RunRoot $runRoot -RunId $runId -Started $Started `
            -Commands $commands -Errors $errors -Gates $gates -P0 $p0 -P1A $p1a `
            -EnvironmentReference $environmentRef -SourceIdentitySha256 $sourceIdentitySha `
            -CleanupAttempted $cleanupAttempted -TemporaryRemoved $temporaryRemoved -Status PASS
        try {
            $publication = Publish-P1BAcceptance -OutputRoot $output -RunId $runId `
                -SourceIdentitySha256 $sourceIdentitySha
        }
        catch {
            $exception = New-P1AFailureException -Code 'EVIDENCE_PUBLICATION_FAILED' -Category 5 `
                -Message 'P1B machine qualification passed, but its automatic acceptance was not published' `
                -Remediation 'Inspect the immutable PASS run and retry publication through the exact P1B command.'
            $exception.Data['P1BEvidencePath'] = "docs/receipts/P1B/runs/$runId/evidence.json"
            throw $exception
        }
        return [pscustomobject]@{
            status = 'PASS'; phase_id = 'P1B'; run_id = $runId
            evidence_path = "docs/receipts/P1B/runs/$runId/evidence.json"
            acceptance_path = $publication.acceptance_path
            pointer_path = $publication.pointer_path
        }
    }

    if (-not (Test-Path -LiteralPath $sourceIdentityPath -PathType Leaf)) {
        Write-P1AJsonFile -Path $sourceIdentityPath -Value ([ordered]@{
                schema = 'python-slm-source-identity-v1'
                status = 'FAIL'
                run_id = $runId
                note = 'source identity was unavailable before qualification failed'
            }) -CreateNew
    }
    foreach ($artifactName in @(
            'p1a-dependency.json', 'environment.json', 'cuda-toolkit.json',
            'gpu-runtime.json', 'cuda-probe.json'
        )) {
        $artifactPath = Join-Path $artifactsRoot $artifactName
        if (-not (Test-Path -LiteralPath $artifactPath -PathType Leaf)) {
            Write-P1AJsonFile -Path $artifactPath -Value ([ordered]@{
                    schema = 'python-slm-p1b-unavailable-artifact-v1'
                    artifact = $artifactName
                    status = 'FAIL'
                    run_id = $runId
                    note = 'artifact was unavailable before qualification failed'
                }) -CreateNew
        }
    }
    $sourceIdentitySha = Get-P1ASha256 -Path $sourceIdentityPath
    $failureLeaks = [Collections.Generic.List[string]]::new()
    $failureSensitiveValues = @($repository, $output, $env:USERPROFILE, $env:USERNAME,
        $env:COMPUTERNAME) + @($context.RedactionRoots)
    foreach ($file in Get-ChildItem -LiteralPath $runRoot -File -Recurse) {
        $text = [IO.File]::ReadAllText($file.FullName, $script:Utf8NoBom)
        foreach ($value in @($failureSensitiveValues | Where-Object {
                    -not [string]::IsNullOrWhiteSpace([string]$_)
                } | Sort-Object -Unique)) {
            if ($text -match [regex]::Escape([string]$value)) {
                $failureLeaks.Add((ConvertTo-P1ANormalizedPath $file.FullName $runRoot))
            }
        }
        if ($text -match '(?i)(?:^|[\s"''])[A-Z]:[\\/]' -or
            $text -match '(?i)(?:^|[\s"''])\\\\[^\\\s]+' -or
            $text -match '(?i)(?:authorization|api[_-]?key|access[_-]?token|password)\s*[:=]\s*[^\s,;}]+') {
            $failureLeaks.Add((ConvertTo-P1ANormalizedPath $file.FullName $runRoot))
        }
    }
    if ($failureLeaks.Count -ne 0) {
        Set-P1AGate $gates redaction FAIL 'failed-run artifacts contained prohibited machine or secret-shaped values'
        if (@($errors | Where-Object { [string]$_['code'] -ceq 'REDACTION_FAILED' }).Count -eq 0) {
            Add-P1AError -Errors $errors -Code 'REDACTION_FAILED' -Category 3 `
                -Message 'failed-run artifacts contained prohibited machine or secret-shaped values' `
                -Remediation 'Fix path tokenization or output redaction, then rerun.'
        }
        foreach ($relative in @($failureLeaks | Sort-Object -Unique)) {
            $path = Join-Path $runRoot $relative.Replace('/', '\')
            if (Test-P1APathWithin -Path $path -Root $runRoot) {
                Write-P1AUtf8LfFile -Path $path -Text `
                    $(if ([IO.Path]::GetExtension($path) -ieq '.json') {
                            '{"schema":"python-slm-redacted-artifact-v1","status":"REDACTED"}'
                        }
                        else { 'P1B failed-run artifact removed because the redaction gate failed.' })
            }
        }
        foreach ($command in @($commands)) {
            foreach ($stream in @('stdout', 'stderr')) {
                $path = Join-Path $runRoot ([string]$command[$stream].path).Replace('/', '\')
                $command[$stream].sha256 = Get-P1ASha256 -Path $path
                $command[$stream].bytes = [int64](Get-Item -LiteralPath $path).Length
            }
        }
        $sourceIdentitySha = Get-P1ASha256 -Path $sourceIdentityPath
    }
    elseif ([string]$gates.redaction.status -eq 'NOT_RUN') {
        Set-P1AGate $gates redaction PASS 'failed-run receipts contain no prohibited machine, user, credential, or secret values'
    }
    Write-P1AJsonFile -Path (Join-Path $artifactsRoot 'failure-summary.json') `
        -Value ([ordered]@{ status = 'FAIL'; errors = @($errors) }) -CreateNew
    $evidencePath = Write-P1BTerminalRun -RunRoot $runRoot -RunId $runId -Started $Started `
        -Commands $commands -Errors $errors -Gates $gates -P0 $p0 -P1A $p1a `
        -EnvironmentReference $environmentRef -SourceIdentitySha256 $sourceIdentitySha `
        -CleanupAttempted $cleanupAttempted -TemporaryRemoved $temporaryRemoved -Status FAIL
    return [pscustomobject]@{
        status = 'FAIL'
        phase_id = 'P1B'
        run_id = $runId
        evidence_path = "docs/receipts/P1B/runs/$runId/evidence.json"
        errors = @($errors)
    }
}

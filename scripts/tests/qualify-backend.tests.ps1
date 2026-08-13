Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$modulePath = Join-Path $repositoryRoot 'scripts\lib\QualifyBackend.psm1'
$module = Import-Module -Name $modulePath -Force -PassThru
$script:Passed = 0
$script:Failed = 0
$script:Failures = [Collections.Generic.List[string]]::new()
$moduleAudit=[pscustomobject][ordered]@{method='toolhelp32';audited_process_count=1;successful_snapshots=1;failed_snapshots=0;last_error=$null}

function Invoke-P2Test {
    param([Parameter(Mandatory)][string]$Name, [Parameter(Mandatory)][scriptblock]$Body)
    try { & $Body; $script:Passed++; Write-Output "PASS $Name" }
    catch { $script:Failed++; $script:Failures.Add("FAIL $Name :: $($_.Exception.Message)") }
}

function Assert-P2Test { param([bool]$Condition, [string]$Message) if (-not $Condition) { throw $Message } }

function Copy-P2TestObject { param($Value) return (($Value|ConvertTo-Json -Depth 64 -Compress)|ConvertFrom-Json) }

function New-P2TestFixtureHashes {
    param([Parameter(Mandatory)][string]$Workload)
    $table=@{
        allocation=@(25165824,'9b707d439be74adc6b224c46fdba78b82ee590b083fc28d7933be0cc99037bee',0,$null)
        correctness=@(527,'3a0438fe000928d4384c01166604631d32f2ff4f9e341ba58bd81912c3fecddc',899,'501883a6afa454243ac78dee6a6940301ba4eee81ea8b33fde86a997a351d182')
        projection=@(6291456,'813ffd0eb83b9b71cf9b17361969f0c87da3b1647248f4bee6350cf6c9b42f2f',589824,'a28d78c9179c8ff579fe6fdfe6f10871ca855a692272eb6d28e079dc8581678c')
        'ffn-expansion'=@(6291456,'09032e0d9115ef46bda8803823901958e28755b5f5c752ec3d0b5435ce67ec2d',1867776,'2024e09787f70025d046affc45827142f8efcd11c7344a5f11cae36a7e3f4f79')
    }
    $v=$table[$Workload]
    return [pscustomobject][ordered]@{algorithm='sha256';a_sha256=$v[1];b_sha256=$v[3];a_elements=[int64]$v[0];b_elements=[int64]$v[2]}
}

function New-P2TestTiming {
    param([Parameter(Mandatory)][string]$Workload)
    $shape=if($Workload-ceq'projection'){@(8192,768,768)}else{@(8192,768,2432)}
    $samples=@(1..200|ForEach-Object{[int64](25000000+$_)})
    $forwardFlops=[int64]2*$shape[0]*$shape[1]*$shape[2];$fwbwFlops=[int64]6*$shape[0]*$shape[1]*$shape[2]
    $series={param([int64]$flops)[pscustomobject][ordered]@{samples_ns=$samples;sample_count=200;elapsed_ns=[int64]5000000000
            p50_ns=$samples[99];p95_ns=$samples[189];flop_count=$flops;gflops=[double]$flops/$samples[99]}}
    return [pscustomobject][ordered]@{shape=[pscustomobject][ordered]@{m=$shape[0];k=$shape[1];n=$shape[2]};warmup_iterations=50
        forward=(& $series $forwardFlops);forward_backward=(& $series $fwbwFlops);context_ns=[int64]1;jit_ns=[int64]1;first_result_ns=[int64]1}
}

function New-P2TestCandidateResult {
    param([ValidateSet('cpu-smoke','correctness','benchmark')][string]$Mode,
        [ValidateSet('allocation','correctness','projection','ffn-expansion')][string]$Workload,
        [ValidateSet('burn-cubecl','candle','cudarc-fallback')][string]$CandidateId='candle')
    $crate=@{'burn-cubecl'=@('burn','0.21.0');candle=@('candle-core','0.11.0');'cudarc-fallback'=@('cudarc','0.19.8')}[$CandidateId]
    $features=if($Mode-ceq'cpu-smoke'){if($CandidateId-ceq'burn-cubecl'){@('autodiff','flex')}else{@()}}
        elseif($CandidateId-ceq'burn-cubecl'){@('autodiff','autotune','cuda','fusion')}
        elseif($CandidateId-ceq'candle'){@('cuda')}else{@('cublas','cublaslt','cuda-13010','driver')}
    $accumulation=if($CandidateId-ceq'candle'-and$Mode-cne'cpu-smoke'){'runtime_getter=candle_core::cuda::gemm_reduced_precision_bf16;observed=false;compute=CUBLAS_COMPUTE_32F'}
        elseif($CandidateId-ceq'burn-cubecl'-and$Mode-cne'cpu-smoke'){'crate=cubek-matmul@0.2.0;crate-checksum-sha256=83a4cea5f0f439907dc953c7638a6204b3f055f1bcbd10db91dfc5faa030ac1c;source-sha256=f74fe23b585d7b359d7c04f5a9e586c517684ff4e788807ee67897383160c655;locator=cubek-matmul-0.2.0/src/definition/spec.rs:85-91;assertion=BF16_MATMUL_PRECISION_ACCUMULATOR_F32;cfg=not-macos'}
        elseif($CandidateId-ceq'cudarc-fallback'){'cuBLASLt CUDA_R_16BF inputs with CUBLAS_COMPUTE_32F'}else{'unverified-until-cuda-run'}
    $provenance=[pscustomobject][ordered]@{crate_name=$crate[0];crate_version=$crate[1];feature_set=$features
        device=$(if($Mode-ceq'cpu-smoke'){'CPU'}else{'CUDA device 0'});device_ordinal=$(if($Mode-ceq'cpu-smoke'){$null}else{0})
        explicit_synchronization=$true;fp32_accumulation_evidence=$accumulation;framework_rng_used=$false}
    $memory=if($Mode-ceq'cpu-smoke'){$null}else{[pscustomobject][ordered]@{free_bytes_after_context=[int64]1000;free_bytes_after_allocation=[int64]900
            free_bytes_after_forward=$(if($Workload-ceq'allocation'){$null}else{[int64]800});free_bytes_after_backward=$(if($Workload-ceq'allocation'){$null}else{[int64]700})}}
    $allocation=$null;$correctness=$null;$timing=$null
    if($Mode-ceq'correctness'-and$Workload-ceq'allocation'){$allocation=[pscustomobject][ordered]@{shape=@(16,2048,768);elements=[int64]25165824
            input_sha256='9b707d439be74adc6b224c46fdba78b82ee590b083fc28d7933be0cc99037bee';output_sha256='9b707d439be74adc6b224c46fdba78b82ee590b083fc28d7933be0cc99037bee';bitwise_equal=$true}}
    if($Mode-ceq'correctness'-and$Workload-ceq'correctness'){
        $metric={param($elements,$cosine)[pscustomobject][ordered]@{elements=$elements;max_absolute_error=0.0;max_relative_error=0.0;relative_l2=0.0;cosine_similarity=$cosine;envelope_violation_count=0}}
        $correctness=[pscustomobject][ordered]@{shape=[pscustomobject][ordered]@{m=17;k=31;n=29};accumulation='fp32';output_dtype='bf16';loss_dtype='fp32'
            forward=(& $metric 493 1.0);loss=[pscustomobject][ordered]@{actual=1.0;reference=1.0;absolute_error=0.0;allowed_error=0.01001;passed=$true}
            grad_a=(& $metric 527 1.0);grad_b=(& $metric 899 1.0);nan_count=0;infinite_count=0;envelope_violation_count=0}
    }
    if($Mode-ceq'benchmark'){$timing=New-P2TestTiming $Workload}
    return [pscustomobject][ordered]@{schema='python-slm-backend-candidate-result-v1';candidate_id=$CandidateId;mode=$Mode;status='PASS';workload=$Workload
        fixture_hashes=(New-P2TestFixtureHashes -Workload $Workload);allocation=$allocation;correctness=$correctness;timing=$timing;memory=$memory;provenance=$provenance;diagnostics=@()}
}

function New-P2ProtocolCommand {
    param([string]$Id,[object[]]$Argv,[string]$Cwd='${REPO}')
    return [pscustomobject][ordered]@{id=$Id;argv=@($Argv);cwd=$Cwd}
}

function New-P2ProtocolInvocation {
    param([string]$CommandId,[string]$Mode,[string]$Workload,[AllowNull()]$Round)
    return [pscustomobject][ordered]@{command_id=$CommandId;mode=$Mode;workload=$Workload;round=$Round}
}

function New-P2AcceptedProtocolFixture {
    $manifest='${REPO}/experiments/p2-backends/Cargo.toml';$commands=[Collections.Generic.List[object]]::new();$n=1
    foreach($argv in @(@('cargo.exe','fmt','--all','--','--check'),@('cargo.exe','clippy','--locked','--all-targets','--features','cpu-reference','--','-D','warnings'),
            @('cargo.exe','test','--locked','--features','cpu-reference'))){$commands.Add((New-P2ProtocolCommand ('C'+$n.ToString('00')) $argv));$n++}
    foreach($candidate in @('burn-cubecl','candle')){foreach($mode in @('cpu','cuda')){$target="`${TEMP}/$mode-target/$candidate";$argv=@('cargo.exe','build','-vv','--locked','--offline','--release','--manifest-path',$manifest,
                    '--package',$(if($candidate-ceq'burn-cubecl'){'p2-burn-cubecl'}else{'p2-candle'}),'--no-default-features','--features',$mode)
            if($mode-ceq'cuda'){$argv+=@('--target','x86_64-pc-windows-msvc')};$argv+=@('--target-dir',$target);$commands.Add((New-P2ProtocolCommand ('C'+$n.ToString('00')) $argv));$n++}}
    $package='p2-candle'
    foreach($tree in @(@('cuda','features'),@('cuda','normal,build'),@('cpu','features'))){$id=if($tree[1]-ceq'normal,build'){'C1008'}else{'C'+$n.ToString('00')}
        $commands.Add((New-P2ProtocolCommand $id @('cargo.exe','tree','--locked','--offline','--manifest-path',$manifest,'--package',$package,'--no-default-features','--features',$tree[0],
                    '--target','x86_64-pc-windows-msvc','-e',$tree[1],'--prefix','none','--format','{p}|{f}')));if($tree[1]-cne'normal,build'){$n++}}
    $commands.Add((New-P2ProtocolCommand ('C'+$n.ToString('00')) @('cargo.exe','run','--locked','--offline','--release','--manifest-path',$manifest,'--package','p2-backend-common','--bin','p2-fixtures','--target-dir','${TEMP}/fixture-target','--','--output','${TEMP}/fixtures')));$n++
    foreach($path in @('${TEMP}/cpu-target/candle/release/p2-candle.exe','${TEMP}/cuda-target/candle/x86_64-pc-windows-msvc/release/p2-candle.exe')){$commands.Add((New-P2ProtocolCommand ('C'+$n.ToString('00')) @('dumpbin.exe','/HEADERS','/DEPENDENTS',$path)));$n++}
    $refs=[ordered]@{cpu_smoke=New-P2ProtocolInvocation ('C'+$n.ToString('00')) cpu-smoke correctness $null};$n++
    $refs.allocation=New-P2ProtocolInvocation ('C'+$n.ToString('00')) correctness allocation $null;$n++
    $refs.correctness=New-P2ProtocolInvocation ('C'+$n.ToString('00')) correctness correctness $null;$n++
    $bench=[Collections.Generic.List[object]]::new();foreach($spec in @(@(2,'projection'),@(3,'projection'),@(2,'ffn-expansion'),@(3,'ffn-expansion'))){$ref=New-P2ProtocolInvocation ('C'+$n.ToString('00')) benchmark $spec[1] $spec[0];$bench.Add($ref);$n++}
    foreach($ref in @($refs.cpu_smoke,$refs.allocation,$refs.correctness)+@($bench)){$suffix=if($null-eq$ref.round){"$($ref.mode)-$($ref.workload)"}else{"round-$($ref.round)-$($ref.workload)"}
        $commands.Add((New-P2ProtocolCommand $ref.command_id @('p2-candle.exe','--mode',$ref.mode,'--workload',$ref.workload,'--fixture-dir','${TEMP}/fixtures','--output',"`${TEMP}/invocations/candle/$suffix/raw-result.json") "`${TEMP}/invocations/candle/$suffix"))}
    $burn=[pscustomobject]@{candidate_id='burn-cubecl';status='FAIL';cpu_smoke=$null;allocation=$null;correctness=$null;benchmark_rounds=@()}
    $candle=[pscustomobject]@{candidate_id='candle';status='PASS';cpu_smoke=$refs.cpu_smoke;allocation=$refs.allocation;correctness=$refs.correctness;benchmark_rounds=@($bench)}
    return [pscustomobject]@{commands=@($commands);aggregates=@($burn,$candle)}
}

try {
    Invoke-P2Test 'nearest-rank percentiles are deterministic' {
        Assert-P2Test ((Get-P2NearestRankPercentile -Values @(1, 9, 3, 7, 5) -Percentile 0.5) -eq 5) 'p50 mismatch'
        Assert-P2Test ((Get-P2NearestRankPercentile -Values @(1, 9, 3, 7, 5) -Percentile 0.95) -eq 9) 'p95 mismatch'
    }
    Invoke-P2Test 'empty process argv renders as an empty command line' {
        $rendered=&$module { ConvertTo-P2CommandLine -Arguments @() }
        Assert-P2Test ($rendered -ceq '') 'empty process argv was rejected or rendered nonempty'
    }
    Invoke-P2Test 'selection applies the greater-than-five-percent band' {
        $a = [pscustomobject]@{ candidate_id = 'burn-cubecl'; status = 'PASS'; comparison = [pscustomobject]@{
                geomean_fwbw_p50_ns = 100; geomean_fwbw_p95_ns = 120; observed_peak_bytes = 1000; locked_dependency_count = 10 } }
        $b = [pscustomobject]@{ candidate_id = 'candle'; status = 'PASS'; comparison = [pscustomobject]@{
                geomean_fwbw_p50_ns = 106; geomean_fwbw_p95_ns = 100; observed_peak_bytes = 900; locked_dependency_count = 8 } }
        $selected = Select-P2Candidate -Candidates @($a, $b)
        Assert-P2Test ($selected.selected_candidate_id -ceq 'burn-cubecl') 'p50 winner mismatch'
    }
    Invoke-P2Test 'selection excludes diagnostic fallback' {
        $fallback = [pscustomobject]@{ candidate_id = 'cudarc-fallback'; status = 'PASS'; comparison = [pscustomobject]@{} }
        $selected = Select-P2Candidate -Candidates @($fallback)
        Assert-P2Test ($selected.status -ceq 'FAIL') 'diagnostic fallback selected'
    }
    Invoke-P2Test 'output root rejects traversal' {
        $threw = $false
        try { [void](Resolve-P2OutputRoot -RepositoryRoot $repositoryRoot -OutputRoot 'docs\receipts\P2\..\P1B') }
        catch { $threw = $true }
        Assert-P2Test $threw 'traversal was accepted'
    }
    Invoke-P2Test 'redaction removes credentials and repository path' {
        $safe = Protect-P2Text -Text ("$repositoryRoot token=secret-value") -RepositoryRoot $repositoryRoot
        Assert-P2Test ($safe -notmatch [regex]::Escape($repositoryRoot)) 'repository path leaked'
        Assert-P2Test ($safe -notmatch 'secret-value') 'secret leaked'
    }
    Invoke-P2Test 'sanitizer redacts generic email addresses' {
        $safe = Protect-P2Text -Text 'authors=owner@example.com,build.bot+cuda@example.org' -RepositoryRoot $repositoryRoot
        Assert-P2Test ($safe -notmatch '(?i)[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}') 'generic email address leaked'
        Assert-P2Test ($safe -cne 'authors=owner@example.com,build.bot+cuda@example.org') 'generic email addresses were not sanitized'
        $cargo = Protect-P2Text -Text 'set CARGO_PKG_AUTHORS="Build Author <author@example.com>:Other Person <other@example.org>"&& set NEXT=1' -RepositoryRoot $repositoryRoot
        Assert-P2Test ($cargo -ceq 'set CARGO_PKG_AUTHORS=<redacted-authors>&& set NEXT=1') 'Cargo author identity list was not fully redacted'
    }
    Invoke-P2Test 'WinPS5 process runner canonicalizes the parent environment' {
        $result = Invoke-P2Process -FilePath $env:ComSpec -ArgumentList @('/d', '/c', 'echo P2_PROCESS_OK & ping -n 2 127.0.0.1 >nul') `
            -WorkingDirectory $repositoryRoot -Environment @{ P2_CHILD_MARKER = 'present' } -TimeoutSeconds 10
        Assert-P2Test (-not $result.timed_out) 'process runner timed out'
        Assert-P2Test ($result.exit_code -eq 0) 'process runner failed'
        Assert-P2Test ($result.stdout.Trim() -ceq 'P2_PROCESS_OK') 'process stdout mismatch'
        Assert-P2Test ($result.module_audit.method-ceq'toolhelp32'-and$result.module_audit.audited_process_count-ge1-and$result.module_audit.successful_snapshots-ge$result.module_audit.audited_process_count-and$result.module_audit.failed_snapshots-eq0-and@($result.loaded_modules).Count-gt0) 'process module audit was not complete'
    }
    Invoke-P2Test 'process module audit covers a helper that outlives its parent' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-module-tree-'+[Guid]::NewGuid().ToString('N'))
        try{
            [void][IO.Directory]::CreateDirectory($root);$child=Join-Path $root 'child.ps1';$parent=Join-Path $root 'parent.ps1';$ready=Join-Path $root 'child.ready'
            Write-P2Utf8LfFile $child @'
param([Parameter(Mandatory)][string]$Ready)
Start-Sleep -Milliseconds 200
Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public static class P2LoadKnownDll {[DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)]public static extern IntPtr LoadLibrary(string path);}'
$handle=[P2LoadKnownDll]::LoadLibrary((Join-Path $env:SystemRoot 'System32\winhttp.dll'))
if($handle-eq[IntPtr]::Zero){exit 7}
[IO.File]::WriteAllText($Ready,'ready')
Start-Sleep -Milliseconds 700
'@ -CreateNew
            Write-P2Utf8LfFile $parent @'
param([Parameter(Mandatory)][string]$Child,[Parameter(Mandatory)][string]$Ready)
$exe=(Get-Command powershell.exe -ErrorAction Stop).Source
$start=[Diagnostics.ProcessStartInfo]::new($exe,"-NoProfile -ExecutionPolicy Bypass -File `"$Child`" -Ready `"$Ready`"")
$start.UseShellExecute=$false;$null=[Diagnostics.Process]::Start($start)
$deadline=[DateTime]::UtcNow.AddSeconds(5)
while(-not(Test-Path -LiteralPath $Ready)){if([DateTime]::UtcNow-ge$deadline){exit 8};Start-Sleep -Milliseconds 25}
'@ -CreateNew
            $powershell=(Get-Command powershell.exe -ErrorAction Stop).Source
            $result=Invoke-P2Process -FilePath $powershell -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File',$parent,'-Child',$child,'-Ready',$ready) `
                -WorkingDirectory $repositoryRoot -TimeoutSeconds 20
            $known=[IO.Path]::GetFullPath((Join-Path $env:SystemRoot 'System32\winhttp.dll'))
            Assert-P2Test (-not$result.timed_out-and$result.exit_code-eq0-and-not$result.unexpected_descendants) 'helper process tree did not drain cleanly'
            Assert-P2Test ($result.module_audit.audited_process_count-ge2-and$result.module_audit.failed_snapshots-eq0) `
                "helper PID audit mismatch (audited=$($result.module_audit.audited_process_count), successful=$($result.module_audit.successful_snapshots), failed=$($result.module_audit.failed_snapshots), error=$($result.module_audit.last_error))"
            Assert-P2Test ($known-in@($result.loaded_modules)) 'helper-only known DLL was absent from the process-tree audit'
        }finally{if(Test-Path $root){Remove-Item $root -Recurse -Force}}
    }
    Invoke-P2Test 'candidate failure emitter preserves schema-null command identity' {
        $failure=&$module {Get-P2CandidateFailure -Message 'runtime provenance failed' -CommandId $null -Code RUNTIME_PROVENANCE_FAILED -Category 3}
        Assert-P2Test ($null-eq$failure.command_id) 'null candidate command identity became an empty string'
        $threw=$false;try{$null=&$module {Get-P2CandidateFailure -Message 'bad' -CommandId ''}}catch{$threw=$true}
        Assert-P2Test $threw 'explicit empty candidate command identity was accepted'
    }
    Invoke-P2Test 'fresh WinPS5 import resolves monitor construction and provenance' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-fresh-native-'+[Guid]::NewGuid().ToString('N'))
        try{
            [void][IO.Directory]::CreateDirectory($root);$probe=Join-Path $root 'probe.ps1'
            $source=@'
param([Parameter(Mandatory)][string]$ModulePath)
$ErrorActionPreference='Stop'
$m=Import-Module -Name $ModulePath -Force -PassThru
&$m {
    Initialize-P2NativeInterop
    $monitor=[P2NvmlMonitor]::new()
    try {
        $path=[P2NvmlMonitor]::EnsureQualifiedLibrary()
        $record=New-P2NvmlLibraryRecord -Path $path -WindowsRoot $env:SystemRoot
        if([string]$record.path-cne'${WINDOWS}/System32/nvml.dll'-or[string]$record.sha256-cnotmatch'^[0-9a-f]{64}$'){
            throw 'fresh-process NVML provenance is invalid'
        }
        $record|ConvertTo-Json -Compress
    }
    finally{$monitor.Dispose()}
}
$m=Import-Module -Name $ModulePath -Force -PassThru
&$m { Initialize-P2NativeInterop;$monitor=[P2NvmlMonitor]::new();$monitor.Dispose() }
'@
            Write-P2Utf8LfFile -Path $probe -Text $source -CreateNew
            $powershell=(Get-Command powershell.exe -ErrorAction Stop).Source
            $result=Invoke-P2Process -FilePath $powershell -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File',$probe,'-ModulePath',$modulePath) `
                -WorkingDirectory $repositoryRoot -TimeoutSeconds 30
            Assert-P2Test (-not$result.timed_out-and$result.exit_code-eq0) "fresh WinPS5 native import failed: $($result.stderr)"
            $record=$result.stdout|ConvertFrom-Json
            Assert-P2Test ([string]$record.path-ceq'${WINDOWS}/System32/nvml.dll') 'fresh WinPS5 NVML provenance path is wrong'
        }finally{if(Test-Path $root){Remove-Item $root -Recurse -Force}}
    }
    Invoke-P2Test 'qualification initializes native interop before its first NVML reference' {
        $source=[IO.File]::ReadAllText($modulePath,[Text.UTF8Encoding]::new($false,$true))
        $start=$source.IndexOf('function Invoke-P2Qualification',[StringComparison]::Ordinal)
        $end=$source.IndexOf('function Get-P2SelectedP1BDependency',[StringComparison]::Ordinal)
        Assert-P2Test ($start-ge0-and$end-gt$start) 'qualification source boundary was not found'
        $body=$source.Substring($start,$end-$start)
        $initialize=$body.IndexOf('Initialize-P2NativeInterop',[StringComparison]::Ordinal)
        $nvml=$body.IndexOf('[P2NvmlMonitor]',[StringComparison]::Ordinal)
        Assert-P2Test ($initialize-ge0-and$nvml-gt$initialize) 'qualification references NVML before native interop initialization'
    }
    Invoke-P2Test 'run seal covers sorted files and detects tampering' {
        $root = Join-Path ([IO.Path]::GetTempPath()) ('p2-seal-' + [Guid]::NewGuid().ToString('N'))
        try {
            [void][IO.Directory]::CreateDirectory((Join-Path $root 'artifacts'))
            Write-P2Utf8LfFile -Path (Join-Path $root 'z.txt') -Text 'z' -CreateNew
            Write-P2Utf8LfFile -Path (Join-Path $root 'artifacts\a.txt') -Text 'a' -CreateNew
            $seal = New-P2Seal -RunRoot $root
            Assert-P2Test ($seal.entries -eq 2) 'seal entry count mismatch'
            Assert-P2Test (Test-P2Seal -RunRoot $root) 'new seal did not validate'
            $lines = [IO.File]::ReadAllLines((Join-Path $root 'SHA256SUMS'))
            Assert-P2Test ($lines[0] -match '  artifacts/a\.txt$') 'seal was not sorted'
            Write-P2Utf8LfFile -Path (Join-Path $root 'z.txt') -Text 'changed'
            Assert-P2Test (-not (Test-P2Seal -RunRoot $root)) 'tampered seal validated'
        }
        finally { if (Test-Path $root) { Remove-Item -LiteralPath $root -Recurse -Force } }
    }
    Invoke-P2Test 'seal detects files added after sealing' {
        $root = Join-Path ([IO.Path]::GetTempPath()) ('p2-seal-extra-' + [Guid]::NewGuid().ToString('N'))
        try {
            [void][IO.Directory]::CreateDirectory($root)
            Write-P2Utf8LfFile -Path (Join-Path $root 'a.txt') -Text 'a' -CreateNew
            [void](New-P2Seal -RunRoot $root)
            Write-P2Utf8LfFile -Path (Join-Path $root 'b.txt') -Text 'b' -CreateNew
            Assert-P2Test (-not (Test-P2Seal -RunRoot $root)) 'unsealed extra file accepted'
        }
        finally { if (Test-Path $root) { Remove-Item -LiteralPath $root -Recurse -Force } }
    }
    Invoke-P2Test 'timing validation enforces samples window and nearest-rank percentiles' {
        $samples = @(1..200 | ForEach-Object { [int64](25000000 + $_) })
        $forwardFlops=[int64]2*8192*768*768;$backwardFlops=[int64]6*8192*768*768
        $forward = [pscustomobject]@{ samples_ns=$samples;sample_count=200;elapsed_ns=5000000000;p50_ns=$samples[99];p95_ns=$samples[189];flop_count=$forwardFlops;gflops=([double]$forwardFlops/$samples[99]) }
        $backward = [pscustomobject]@{ samples_ns=$samples;sample_count=200;elapsed_ns=5000000000;p50_ns=$samples[99];p95_ns=$samples[189];flop_count=$backwardFlops;gflops=([double]$backwardFlops/$samples[99]) }
        $result = [pscustomobject]@{ status='PASS';mode='benchmark';workload='projection';timing=[pscustomobject]@{
                shape=[pscustomobject]@{m=8192;k=768;n=768};warmup_iterations=50;context_ns=1;jit_ns=1;first_result_ns=1
                forward=$forward;forward_backward=$backward } }
        Assert-P2Test (Test-P2TimingResult -Result $result) 'valid timing rejected'
        $result.timing.forward.p95_ns++
        Assert-P2Test (-not (Test-P2TimingResult -Result $result)) 'wrong percentile accepted'
    }
    Invoke-P2Test 'timing validation rejects short windows and excessive sample counts' {
        $samples = @(1..200 | ForEach-Object { [int64]$_ })
        $ff=[int64]2*8192*768*768;$fb=[int64]6*8192*768*768
        $forward=[pscustomobject]@{samples_ns=$samples;sample_count=200;elapsed_ns=4999999999;p50_ns=100;p95_ns=190;flop_count=$ff;gflops=([double]$ff/100)}
        $backward=[pscustomobject]@{samples_ns=$samples;sample_count=200;elapsed_ns=4999999999;p50_ns=100;p95_ns=190;flop_count=$fb;gflops=([double]$fb/100)}
        $result=[pscustomobject]@{status='PASS';mode='benchmark';workload='projection';timing=[pscustomobject]@{
                shape=[pscustomobject]@{m=8192;k=768;n=768};warmup_iterations=50;context_ns=1;jit_ns=1;first_result_ns=1;forward=$forward;forward_backward=$backward}}
        Assert-P2Test (-not (Test-P2TimingResult $result)) 'short timing window accepted'
    }
    Invoke-P2Test 'candidate selection falls through p50 band to p95' {
        $a=[pscustomobject]@{candidate_id='burn-cubecl';status='PASS';comparison=[pscustomobject]@{
                geomean_fwbw_p50_ns=100;geomean_fwbw_p95_ns=100;observed_peak_bytes=1000;locked_dependency_count=20}}
        $b=[pscustomobject]@{candidate_id='candle';status='PASS';comparison=[pscustomobject]@{
                geomean_fwbw_p50_ns=105;geomean_fwbw_p95_ns=106;observed_peak_bytes=900;locked_dependency_count=10}}
        $selected=Select-P2Candidate @($a,$b)
        Assert-P2Test ($selected.selected_candidate_id -ceq 'burn-cubecl' -and $selected.criterion -ceq 'geomean_fwbw_p95_ns') 'p95 tiebreak failed'
    }
    Invoke-P2Test 'candidate selection uses dependency count then stable ID' {
        $base=[pscustomobject]@{geomean_fwbw_p50_ns=100;geomean_fwbw_p95_ns=100;observed_peak_bytes=1000;locked_dependency_count=10}
        $a=[pscustomobject]@{candidate_id='burn-cubecl';status='PASS';comparison=$base}
        $b=[pscustomobject]@{candidate_id='candle';status='PASS';comparison=($base|Select-Object *)}
        $b.comparison.locked_dependency_count=9
        Assert-P2Test ((Select-P2Candidate @($a,$b)).selected_candidate_id -ceq 'candle') 'dependency tiebreak failed'
        $b.comparison.locked_dependency_count=10
        Assert-P2Test ((Select-P2Candidate @($a,$b)).selected_candidate_id -ceq 'burn-cubecl') 'stable ID tiebreak failed'
    }
    Invoke-P2Test 'ABBA order requires exact global round sequence' {
        $rows=@(
            [pscustomobject]@{candidate_id='burn-cubecl';round=1;workload='projection'},[pscustomobject]@{candidate_id='candle';round=2;workload='projection'},
            [pscustomobject]@{candidate_id='candle';round=3;workload='projection'},[pscustomobject]@{candidate_id='burn-cubecl';round=4;workload='projection'},
            [pscustomobject]@{candidate_id='burn-cubecl';round=1;workload='ffn-expansion'},[pscustomobject]@{candidate_id='candle';round=2;workload='ffn-expansion'},
            [pscustomobject]@{candidate_id='candle';round=3;workload='ffn-expansion'},[pscustomobject]@{candidate_id='burn-cubecl';round=4;workload='ffn-expansion'})
        Assert-P2Test (Assert-P2AbbaOrder $rows) 'canonical ABBA rejected'
        $burn=@($rows|Where-Object candidate_id -eq burn-cubecl|Sort-Object -Property @(
                @{Expression={[int]$_.round}},@{Expression={if($_.workload-ceq'projection'){0}else{1}}}))
        Assert-P2Test ((@($burn|ForEach-Object{"$($_.round):$($_.workload)"})-join',')-ceq'1:projection,1:ffn-expansion,4:projection,4:ffn-expansion') 'candidate aggregate order is not canonical'
        $rows[0].candidate_id='candle';$threw=$false;try{[void](Assert-P2AbbaOrder $rows)}catch{$threw=$true}
        Assert-P2Test $threw 'wrong ABBA order accepted'
    }
    Invoke-P2Test 'runtime provenance rejects CUDA DLL outside qualified roots' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-dll-'+[Guid]::NewGuid().ToString('N'))
        try{[void][IO.Directory]::CreateDirectory($root);$dll=Join-Path $root 'cudart64_13.dll';Write-P2Utf8LfFile $dll 'fake' -CreateNew
            $p=Get-P2LoadedModuleProvenance -LoadedModules @($dll) -CudaToolkitRoot (Join-Path $root 'toolkit') -WindowsRoot $env:SystemRoot -ModuleAudit $moduleAudit -DriverPackageAnchor '${WINDOWS}/System32/DriverStore/FileRepository/fake'
            Assert-P2Test (-not $p.all_allowed) 'foreign CUDA DLL accepted'}finally{if(Test-Path $root){Remove-Item $root -Recurse -Force}}
    }
    Invoke-P2Test 'CPU isolation detects CUDA and Python evidence' {
        $e=Test-P2CpuIsolationEvidence -CommandResults @([pscustomobject]@{raw_stdout='running python.exe';raw_stderr='link cudart64_13.dll'}) -TargetFiles @('x.ptx')
        Assert-P2Test ($e.status -ceq 'FAIL' -and $e.cuda_or_python_discovered) 'isolation violation missed'
    }
    Invoke-P2Test 'CPU isolation rejects Python DLL and import-library linkage' {
        foreach($evidence in @('    python311.dll', 'link.exe /DEFAULTLIB:python3.lib', 'rustc.exe native=libpython312.dll')){
            $e=Test-P2CpuIsolationEvidence -CommandResults @([pscustomobject]@{raw_stdout=$evidence;raw_stderr=''}) -TargetFiles @('clean.obj')
            Assert-P2Test ($e.status -ceq 'FAIL' -and $e.cuda_or_python_discovered) "Python link evidence was missed: $evidence"
        }
    }
    Invoke-P2Test 'CPU isolation ignores Python prose package names and public URLs' {
        foreach($evidence in @(
                'CARGO_PKG_DESCRIPTION="Split a string into shell words, like Python''s shlex."',
                'tree-sitter-python v0.23.6',
                'CARGO_PKG_REPOSITORY=https://github.com/tree-sitter/tree-sitter-python')){
            $e=Test-P2CpuIsolationEvidence -CommandResults @([pscustomobject]@{raw_stdout=$evidence;raw_stderr=''}) -TargetFiles @('clean.obj')
            Assert-P2Test ($e.status -ceq 'PASS' -and -not$e.cuda_or_python_discovered -and @($e.forbidden_hits).Count-eq0) "benign Python text was rejected: $evidence"
        }
    }
    Invoke-P2Test 'CPU isolation retains executable canary and DLL detections' {
        foreach($evidence in @('running python.exe --version','P2_CANARY_HIT','loading python311.dll')){
            $e=Test-P2CpuIsolationEvidence -CommandResults @([pscustomobject]@{raw_stdout=$evidence;raw_stderr=''}) -TargetFiles @('clean.obj')
            Assert-P2Test ($e.status -ceq 'FAIL' -and $e.cuda_or_python_discovered) "true Python evidence was missed: $evidence"
        }
    }
    Invoke-P2Test 'dependency policy rejects git sources and patches' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-deps-'+[Guid]::NewGuid().ToString('N'))
        try{[void][IO.Directory]::CreateDirectory($root);$manifest=Join-Path $root 'Cargo.toml';$lock=Join-Path $root 'Cargo.lock'
            Write-P2Utf8LfFile $manifest '[patch.crates-io]' -CreateNew;Write-P2Utf8LfFile $lock 'version = 4' -CreateNew
            $threw=$false;try{[void](Test-P2DependencyPolicy $manifest $lock)}catch{$threw=$true};Assert-P2Test $threw 'patch accepted'}finally{if(Test-Path $root){Remove-Item $root -Recurse -Force}}
    }
    Invoke-P2Test 'process timeout terminates its Job Object tree' {
        $result=Invoke-P2Process -FilePath $env:ComSpec -ArgumentList @('/d','/c','ping -n 20 127.0.0.1 >nul') `
            -WorkingDirectory $repositoryRoot -TimeoutSeconds 1
        Assert-P2Test ($result.timed_out -and $result.process_tree_terminated) 'timed-out process tree survived'
    }
    Invoke-P2Test 'rapid clean process exits drain Job accounting without false descendants' {
        foreach($iteration in 1..12){
            $result=Invoke-P2Process -FilePath $env:ComSpec -ArgumentList @('/d','/c','exit /b 0') `
                -WorkingDirectory $repositoryRoot -TimeoutSeconds 10
            Assert-P2Test (-not$result.timed_out-and$result.process_tree_terminated-and-not$result.unexpected_descendants) "clean process exit $iteration was classified as a descendant"
        }
    }
    Invoke-P2Test 'bounded transient child drains without a false persistent-descendant result' {
        $script='$null=Start-Process -FilePath $env:ComSpec -ArgumentList @(''/d'',''/c'',''ping -n 4 127.0.0.1 >nul'') -WindowStyle Hidden -PassThru; exit 0'
        $result=Invoke-P2Process -FilePath (Get-Command powershell.exe).Source `
            -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-Command',$script) `
            -WorkingDirectory $repositoryRoot -TimeoutSeconds 15
        Assert-P2Test (-not$result.timed_out-and$result.process_tree_terminated-and-not$result.unexpected_descendants) 'bounded transient child was classified as persistent'
    }
    Invoke-P2Test 'persistent descendant is detected and terminated after the drain window' {
        $script='$null=Start-Process -FilePath $env:ComSpec -ArgumentList @(''/d'',''/c'',''ping -n 20 127.0.0.1 >nul'') -WindowStyle Hidden -PassThru; exit 0'
        $result=Invoke-P2Process -FilePath (Get-Command powershell.exe).Source `
            -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-Command',$script) `
            -WorkingDirectory $repositoryRoot -TimeoutSeconds 15
        Assert-P2Test (-not$result.timed_out-and$result.unexpected_descendants-and$result.process_tree_terminated) 'persistent descendant escaped Job detection/termination'
    }
    Invoke-P2Test 'qualified MSVC vctip cleanup classification is path and process exact' {
        $toolsRoot='C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207'
        $valid=[pscustomobject]@{pid=42;name='vctip';path=(Join-Path $toolsRoot 'bin\Hostx64\x64\vctip.exe')}
        $accepted=&$module {param($record,$root) Test-P2QualifiedVctipProcessSet -Processes @($record) -Environment @{VCToolsInstallDir=$root}} $valid $toolsRoot
        Assert-P2Test $accepted 'qualified vctip process was rejected'
        $upper=[pscustomobject]@{pid=42;name='VCTIP';path=(Join-Path $toolsRoot 'bin\Hostx64\x64\VCTIP.EXE')}
        $accepted=&$module {param($record,$root) Test-P2QualifiedVctipProcessSet -Processes @($record) -Environment @{VCToolsInstallDir=$root}} $upper $toolsRoot
        Assert-P2Test $accepted 'uppercase qualified VCTIP process was rejected'
        foreach($invalid in @(
                [pscustomobject]@{pid=42;name='powershell';path=$valid.path},
                [pscustomobject]@{pid=42;name='vctip';path='C:\Windows\System32\vctip.exe'},
                [pscustomobject]@{pid=42;name='vctip';path=$null})) {
            $accepted=&$module {param($record,$root) Test-P2QualifiedVctipProcessSet -Processes @($record) -Environment @{VCToolsInstallDir=$root}} $invalid $toolsRoot
            Assert-P2Test (-not$accepted) 'non-qualified vctip cleanup record was accepted'
        }
    }
    Invoke-P2Test 'invocation projection is empty-safe and rejects malformed wrappers' {
        $empty=&$module { ConvertTo-P2InvocationProjection -Invocations @() }
        Assert-P2Test (@($empty.results).Count-eq0-and@($empty.references).Count-eq0-and@($empty.runtime_provenance).Count-eq0) 'empty invocation projection failed'
        $valid=[pscustomobject]@{result=[pscustomobject]@{status='PASS'};reference=[pscustomobject]@{path='candidate-results/burn.json'}
            runtime_provenance=[pscustomobject]@{audit_method='toolhelp32';audited_process_count=1;successful_snapshot_count=1;failed_snapshot_count=0;loaded_modules=@();qualified_roots=@();all_allowed=$true}}
        $projected=&$module {param($value)ConvertTo-P2InvocationProjection -Invocations @($value)} $valid
        Assert-P2Test (@($projected.results).Count-eq1-and@($projected.references).Count-eq1-and@($projected.runtime_provenance).Count-eq1) 'valid invocation projection failed'
        $invalid=[pscustomobject]@{result=$valid.result;reference=$valid.reference};$threw=$false
        try{$null=&$module {param($value)ConvertTo-P2InvocationProjection -Invocations @($value)} $invalid}catch{$threw=$true}
        Assert-P2Test $threw 'malformed invocation wrapper was accepted'
    }
    Invoke-P2Test 'failed candidate aggregation accepts canonical empty benchmark evidence' {
        $runtime=&$module { Merge-P2RuntimeProvenance -Records @() }
        Assert-P2Test ((-not$runtime.all_allowed)-and$runtime.audit_method-ceq'toolhelp32'-and$runtime.audited_process_count-eq0-and@($runtime.loaded_modules).Count-eq0-and@($runtime.qualified_roots).Count-eq0) 'empty runtime provenance merge failed closed'
        $failure=[pscustomobject]@{code='CANDIDATE_RESULT_FAILED';category=5;message='candidate failed before benchmarks';command_id='C18'}
        $aggregate=&$module {param($runtime,$failure) New-P2CandidateAggregate -CandidateId burn-cubecl -CpuSmoke $null -Allocation $null -Correctness $null `
            -BenchmarkRounds @() -NvmlMeasurements @() -Summary $null -RuntimeProvenance $runtime -Failures @($failure)} $runtime $failure
        Assert-P2Test ($aggregate.status-ceq'FAIL'-and@($aggregate.benchmark_rounds).Count-eq0-and@($aggregate.nvml_measurements).Count-eq0-and@($aggregate.failures).Count-eq1) 'empty failed candidate aggregate was rejected or malformed'
    }
    Invoke-P2Test 'policy emitter matches renamed warmup contract and closed schema IDs' {
        $policy=New-P2Policy
        Assert-P2Test ($policy.schema -ceq 'python-slm-backend-qualification-policy-v1') 'policy schema mismatch'
        Assert-P2Test ($policy.timing.warmups_per_timed_operation -eq 50) 'warmup contract mismatch'
        Assert-P2Test (-not $policy.timing.Contains('warmups_discarded')) 'stale warmup property present'
        $schemaFiles=@(Get-ChildItem (Join-Path $repositoryRoot 'docs\schemas\P2') -Filter '*.schema.json')
        Assert-P2Test ($schemaFiles.Count -eq 7) 'schema bundle count mismatch'
        foreach($file in $schemaFiles){$schema=Get-Content $file.FullName -Raw|ConvertFrom-Json;Assert-P2Test ($schema.additionalProperties -eq $false) "$($file.Name) not closed"}
    }
    Invoke-P2Test 'candidate timing rejects wrong shape FLOPs nonfinite and sample boundaries' {
        $samples=@(1..200|ForEach-Object{[int64](25000000+$_)});$ff=[int64]2*8192*768*768;$fb=3*$ff
        $mk={param($flops)[pscustomobject]@{samples_ns=$samples;sample_count=200;elapsed_ns=5000000000;p50_ns=$samples[99];p95_ns=$samples[189];flop_count=$flops;gflops=([double]$flops/$samples[99])}}
        $result=[pscustomobject]@{status='PASS';mode='benchmark';workload='projection';timing=[pscustomobject]@{shape=[pscustomobject]@{m=8192;k=768;n=768};warmup_iterations=50;context_ns=0;jit_ns=0;first_result_ns=0;forward=(& $mk $ff);forward_backward=(& $mk $fb)}}
        Assert-P2Test (Test-P2TimingResult $result) 'baseline timing invalid'
        $result.timing.shape.n=769;Assert-P2Test (-not(Test-P2TimingResult $result)) 'wrong shape accepted';$result.timing.shape.n=768
        $result.timing.forward.flop_count++;Assert-P2Test (-not(Test-P2TimingResult $result)) 'wrong FLOPs accepted';$result.timing.forward.flop_count=$ff
        $result.timing.forward.gflops=[double]::NaN;Assert-P2Test (-not(Test-P2TimingResult $result)) 'NaN accepted'
    }
    Invoke-P2Test 'candidate result validator accepts every frozen PASS shape' {
        foreach($case in @(@('cpu-smoke','correctness'),@('correctness','allocation'),@('correctness','correctness'),@('benchmark','projection'),@('benchmark','ffn-expansion'))){
            $result=New-P2TestCandidateResult -Mode $case[0] -Workload $case[1]
            $null=Assert-P2CandidateResult -Result $result -CandidateId candle -Mode $case[0] -Workload $case[1]
        }
        $burn=New-P2TestCandidateResult -Mode benchmark -Workload projection -CandidateId burn-cubecl
        $null=Assert-P2CandidateResult $burn burn-cubecl benchmark projection
        $fallback=New-P2TestCandidateResult -Mode correctness -Workload correctness -CandidateId cudarc-fallback
        $null=Assert-P2CandidateResult $fallback cudarc-fallback correctness correctness
    }
    Invoke-P2Test 'candidate result validator rejects nested schema and policy mutations' {
        $base=New-P2TestCandidateResult -Mode benchmark -Workload projection
        $mutations=@(
            {param($v)$v.provenance|Add-Member extra_field 'x'},
            {param($v)$v.memory=$null},
            {param($v)$v.provenance.feature_set=@('cuda','cudnn')},
            {param($v)$v.allocation=[pscustomobject]@{}},
            {param($v)$v.timing.shape.m='8192'},
            {param($v)$v.fixture_hashes.a_sha256='0'*64},
            {param($v)$v.timing.forward|Add-Member unknown 1}
        )
        foreach($mutation in $mutations){
            $value=Copy-P2TestObject $base;&$mutation $value;$threw=$false
            try{$null=Assert-P2CandidateResult $value candle benchmark projection}catch{$threw=$true}
            Assert-P2Test $threw 'schema/policy mutation was accepted'
        }
        $correctness=New-P2TestCandidateResult -Mode correctness -Workload correctness
        $correctness.correctness.loss.allowed_error=99.0;$threw=$false
        try{$null=Assert-P2CandidateResult $correctness candle correctness correctness}catch{$threw=$true}
        Assert-P2Test $threw 'inflated loss tolerance was accepted'
    }
    Invoke-P2Test 'failed candidate results remain structurally closed' {
        $valid=New-P2TestCandidateResult -Mode correctness -Workload correctness
        $valid.status='FAIL';$valid.diagnostics=@([pscustomobject][ordered]@{code='CUDA_FAILURE';message='bounded candidate failure'})
        $null=Assert-P2CandidateResult $valid candle correctness correctness
        $mutations=@(
            {param($v)$v.correctness.forward|Add-Member unknown 1},
            {param($v)$v.memory.free_bytes_after_context='1000'},
            {param($v)$v.provenance.explicit_synchronization='true'},
            {param($v)$v.fixture_hashes.a_sha256='bad'},
            {param($v)$v.correctness.loss.passed='true'}
        )
        foreach($mutation in $mutations){$value=Copy-P2TestObject $valid;&$mutation $value;$threw=$false
            try{$null=Assert-P2CandidateResult $value candle correctness correctness}catch{$threw=$true}
            Assert-P2Test $threw 'malformed failed candidate result was accepted'}
    }
    Invoke-P2Test 'failed aggregate remains closed and schema-equivalent beside a passing peer' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-failed-aggregate-'+[Guid]::NewGuid().ToString('N'))
        try{[void][IO.Directory]::CreateDirectory($root)
            $base=[pscustomobject][ordered]@{schema='python-slm-backend-candidate-aggregate-v1';candidate_id='candle';role='framework';status='FAIL'
                cpu_smoke=$null;allocation=$null;correctness=$null;benchmark_rounds=@();nvml_measurements=@();summary=$null
                runtime_provenance=[pscustomobject][ordered]@{audit_method='toolhelp32';audited_process_count=1;successful_snapshot_count=1;failed_snapshot_count=0;loaded_modules=@();qualified_roots=@();all_allowed=$true}
                failures=@([pscustomobject][ordered]@{code='CANDIDATE_FAILED';category=5;message='bounded failure';command_id='C42'})}
            $null=&$module {param($v,$r)Assert-P2CandidateAggregate $v $r} $base $root
            $mutations=@(
                {param($v)$v.role='diagnostic_fallback'},
                {param($v)$v.runtime_provenance|Add-Member extra 1},
                {param($v)$v.runtime_provenance.loaded_modules=@([pscustomobject]@{path='${TEMP}/cudart.dll';sha256='0'*64})},
                {param($v)$v.summary=[pscustomobject]@{geomean_fwbw_p50_ns=1;geomean_fwbw_p95_ns=1;observed_peak_bytes=0;locked_dependency_count=0;max_nvml_gap_ms=0;foreign_process_count=0}},
                {param($v)$v.nvml_measurements=@([pscustomobject]@{command_id='C42';round=2;workload='projection';baseline_samples=20;baseline_interval_ms=50;sample_interval_ms=20;total_bytes=1000;baseline_bytes=100;peak_bytes=200;delta_bytes=100;sample_count=1;max_gap_ms=101;baseline_foreign_process_count=0;maximum_foreign_process_count=0;error=$null})})
            foreach($mutation in $mutations){$value=Copy-P2TestObject $base;&$mutation $value;$threw=$false;try{$null=&$module {param($v,$r)Assert-P2CandidateAggregate $v $r} $value $root}catch{$threw=$true}
                Assert-P2Test $threw 'malformed failed aggregate was accepted'}
            $notRun=&$module {New-P2NotRunAggregate candle framework};$notRun.runtime_provenance=[pscustomobject]@{audit_method='toolhelp32';audited_process_count=1;successful_snapshot_count=1;failed_snapshot_count=0;loaded_modules=@();qualified_roots=@();all_allowed=$true};$threw=$false
            try{$null=&$module {param($v,$r)Assert-P2CandidateAggregate $v $r} $notRun $root}catch{$threw=$true};Assert-P2Test $threw 'NOT_RUN aggregate retained runtime evidence'
        }finally{if(Test-Path $root){Remove-Item $root -Recurse -Force}}
    }
    Invoke-P2Test 'recorded commands bind environment configuration and detect tampering' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-config-'+[Guid]::NewGuid().ToString('N'))
        try{[void][IO.Directory]::CreateDirectory($root)
            $result=Invoke-P2RecordedCommand -Id C01 -FilePath $env:ComSpec -ArgumentList @('/d','/c','exit 0') `
                -WorkingDirectory $repositoryRoot -RunRoot $root -RepositoryRoot $repositoryRoot `
                -Environment @{CARGO_NET_OFFLINE='true';RUSTFLAGS=$null} -ExpectedExitCodes @(0) -TimeoutSeconds 10
            $config=Join-Path $root 'commands\C01.configuration.json'
            Assert-P2Test ((Get-P2Sha256 $config)-ceq[string]$result.record.configuration_sha256) 'configuration hash was not bound'
            Assert-P2Test ((@($result.record.environment_delta_names)-join',')-ceq'CARGO_NET_OFFLINE,RUSTFLAGS') 'environment names were not canonical'
            Write-P2Utf8LfFile -Path $config -Text ((Get-Content $config -Raw).Replace('true','false'))
            Assert-P2Test ((Get-P2Sha256 $config)-cne[string]$result.record.configuration_sha256) 'configuration tampering retained its hash'
        }finally{if(Test-Path $root){Remove-Item $root -Recurse -Force}}
    }
    Invoke-P2Test 'accepted command environment enforces offline isolation and canary paths' {
        $cleared=@('RUSTC_WRAPPER','RUSTC_WORKSPACE_WRAPPER','RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUSTDOCFLAGS','CARGO_ENCODED_RUSTDOCFLAGS',
            'PYTHONHOME','PYTHONPATH','PYTHONNOUSERSITE','VIRTUAL_ENV','CONDA_PREFIX','PIP_CONFIG_FILE')
        $base=[ordered]@{CARGO_NET_OFFLINE='true';CARGO_INCREMENTAL='0';CARGO_TERM_COLOR='never';CARGO_TARGET_DIR='${TEMP}/root-target'
            Path='${TEMP}/cpu-canaries;${TEMP}/python-canaries;${WINDOWS}/System32'}
        foreach($name in $cleared){$base[$name]='<CLEARED>'};foreach($name in @('CUDA_PATH','CUDA_COMPUTE_CAP','CUDA_HOME','CUDA_ROOT','CUDA_TOOLKIT_ROOT_DIR','CUDNN_PATH','CUDNN_ROOT','NVCC','NVCC_PREPEND_FLAGS','NVCC_APPEND_FLAGS')){$base[$name]='<CLEARED>'}
        $command=[pscustomobject]@{argv=@('cargo.exe','test','--locked','--features','cpu-reference');cwd='${REPO}'}
        $configuration=[pscustomobject]@{effective_build_environment=[pscustomobject]$base}
        Assert-P2Test (&$module {param($c,$f)Assert-P2CommandEnvironmentPolicy $c $f} $command $configuration) 'canonical CPU environment was rejected'
        $mutations=@(
            {param($v)$v.CARGO_NET_OFFLINE='false'},
            {param($v)$v.CARGO_INCREMENTAL='1'},
            {param($v)$v.RUSTC_WRAPPER='wrapper.exe'},
            {param($v)$v|Add-Member CARGO_BUILD_TARGET 'x86_64-unknown-linux-gnu'},
            {param($v)$v.CARGO_TARGET_DIR='${REPO}/target'},
            {param($v)$v.CUDA_PATH='${CUDA_TOOLKIT}'},
            {param($v)$v.Path='${TEMP}/python-canaries;${WINDOWS}/System32'},
            {param($v)$v.Path='${TEMP}/cpu-canaries;${TEMP}/python-canaries;${CUDA_TOOLKIT}/bin'})
        foreach($mutation in $mutations){$value=Copy-P2TestObject ([pscustomobject]$base);&$mutation $value;$threw=$false
            try{$null=&$module {param($c,$v)Assert-P2CommandEnvironmentPolicy $c ([pscustomobject]@{effective_build_environment=$v})} $command $value}catch{$threw=$true}
            Assert-P2Test $threw 'command environment mutation was accepted'}
        $gpu=Copy-P2TestObject ([pscustomobject]$base);$gpu.PSObject.Properties.Remove('CARGO_TARGET_DIR');$gpu.CUDA_PATH='${CUDA_TOOLKIT}';$gpu.CUDA_COMPUTE_CAP='120'
        foreach($name in @('CUDA_HOME','CUDA_ROOT','CUDA_TOOLKIT_ROOT_DIR','CUDNN_PATH','CUDNN_ROOT','NVCC','NVCC_PREPEND_FLAGS','NVCC_APPEND_FLAGS')){$gpu.PSObject.Properties.Remove($name)}
        $gpu.Path='${TEMP}/python-canaries;${CUDA_TOOLKIT}/bin;${WINDOWS}/System32'
        foreach($name in @('USERPROFILE','HOME','TEMP','TMP')){$gpu|Add-Member $name '${TEMP}/invocations/candle/round-2-projection'}
        $gpu|Add-Member CUDA_CACHE_PATH '${TEMP}/invocations/candle/round-2-projection/cuda-cache'
        $gpuCommand=[pscustomobject]@{argv=@('p2-candle.exe','--mode','benchmark','--workload','projection','--fixture-dir','${TEMP}/fixtures','--output','${TEMP}/invocations/candle/round-2-projection/raw-result.json');cwd='${TEMP}/invocations/candle/round-2-projection'}
        Assert-P2Test (&$module {param($c,$v)Assert-P2CommandEnvironmentPolicy $c ([pscustomobject]@{effective_build_environment=$v})} $gpuCommand $gpu) 'canonical GPU invocation environment was rejected'
        foreach($mutation in @({param($v)$v.CUDA_CACHE_PATH='${TEMP}/shared-cache'},{param($v)$v.CUDA_COMPUTE_CAP='119'})){
            $value=Copy-P2TestObject $gpu;&$mutation $value;$threw=$false;try{$null=&$module {param($c,$v)Assert-P2CommandEnvironmentPolicy $c ([pscustomobject]@{effective_build_environment=$v})} $gpuCommand $value}catch{$threw=$true}
            Assert-P2Test $threw 'invalid GPU cache/compute-capability environment was accepted'}
    }
    Invoke-P2Test 'command protocol permits only immediate derived graph auxiliaries' {
        $ids=@('C01','C02','C1002','C03');$primary=0;$last=$null;$accepted=$true
        foreach($id in $ids){if($id-cmatch'^C1(?<parent>[0-9]{3})$'){if($last-cne('C'+([int]$Matches.parent).ToString('00'))){$accepted=$false}}
            else{$primary++;if($id-cne('C'+$primary.ToString('00'))){$accepted=$false};$last=$id}}
        Assert-P2Test $accepted 'canonical primary/auxiliary sequence was rejected'
        $ids=@('C01','C1002','C02');$primary=0;$last=$null;$accepted=$true
        foreach($id in $ids){if($id-cmatch'^C1(?<parent>[0-9]{3})$'){if($last-cne('C'+([int]$Matches.parent).ToString('00'))){$accepted=$false}}
            else{$primary++;if($id-cne('C'+$primary.ToString('00'))){$accepted=$false};$last=$id}}
        Assert-P2Test (-not$accepted) 'orphan auxiliary command was accepted'
    }
    Invoke-P2Test 'accepted command protocol rejects argv feature and ABBA mutations' {
        $fixture=New-P2AcceptedProtocolFixture
        Assert-P2Test (&$module {param($c,$a)Assert-P2AcceptedCommandProtocol $c $a} $fixture.commands $fixture.aggregates) 'canonical accepted protocol was rejected'
        $mutations=@(
            {param($v)$build=@($v.commands|Where-Object{$_.argv-contains'p2-candle' -and $_.argv-contains'--locked'})[0];$build.argv=@($build.argv|Where-Object{$_-cne'--locked'})},
            {param($v)$build=@($v.commands|Where-Object{$_.argv-contains'p2-candle' -and $_.argv-contains'cuda'})[0];$index=[array]::IndexOf($build.argv,'cuda');$build.argv[$index]='cudnn'},
            {param($v)$ref=$v.aggregates[1].benchmark_rounds[0];$ref.workload='ffn-expansion'}
        )
        foreach($mutation in $mutations){$value=Copy-P2TestObject $fixture;&$mutation $value;$threw=$false
            try{$null=&$module {param($c,$a)Assert-P2AcceptedCommandProtocol $c $a} $value.commands $value.aggregates}catch{$threw=$true}
            Assert-P2Test $threw 'accepted command protocol mutation was ignored'}
    }
    Invoke-P2Test 'candidate-local timeout OOM and crash require sealed health recovery' {
        foreach($status in @('TIMEOUT','OOM','CRASH')){$command=[pscustomobject]@{id='C42';argv=@('p2-candle.exe','--mode','benchmark');expectation_met=$false;exit_code=$null;status=$status}
            $owners=@{C42='candle'};$recovered=[Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal);[void]$recovered.Add('C42')
            Assert-P2Test (&$module {param($c,$o,$r)Assert-P2CommandOutcome $c $o $r} $command $owners $recovered) "$status recovery was rejected"
            $missing=[Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal);$threw=$false
            try{$null=&$module {param($c,$o,$r)Assert-P2CommandOutcome $c $o $r} $command $owners $missing}catch{$threw=$true}
            Assert-P2Test $threw "$status without recovery was accepted"
            $build=[pscustomobject]@{id='C43';argv=@('cargo.exe','build','--package','p2-candle');expectation_met=$false;exit_code=$null;status=$status}
            $buildOwners=@{C43='candle'};$buildRecovery=[Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal);[void]$buildRecovery.Add('C43')
            Assert-P2Test (&$module {param($c,$o,$r)Assert-P2CommandOutcome $c $o $r} $build $buildOwners $buildRecovery) "$status candidate build recovery was rejected"
            $threw=$false;try{$null=&$module {param($c,$o,$r)Assert-P2CommandOutcome $c $o $r} $build $buildOwners $missing}catch{$threw=$true}
            Assert-P2Test $threw "$status candidate build without health recovery was accepted"
        }
        $rootFailure=[pscustomobject]@{id='C01';argv=@('cargo.exe','test');expectation_met=$false;exit_code=1;status='FAIL'};$threw=$false
        try{$null=&$module {param($c,$o,$r)Assert-P2CommandOutcome $c $o $r} $rootFailure @{} ([Collections.Generic.HashSet[string]]::new())}catch{$threw=$true}
        Assert-P2Test $threw 'infrastructure failure was treated as candidate-local'
    }
    Invoke-P2Test 'native CUDA health recovery checks allocation and context cleanup statuses' {
        $source=Get-Content $modulePath -Raw
        foreach($proof in @('int freeRc=Free(ptr)','result="CU_FREE_"+freeRc','int clearRc=SetCurrent(IntPtr.Zero)','result="CU_CLEAR_CURRENT_"+clearRc',
                'int releaseRc=Release(dev)','result="CU_RELEASE_"+releaseRc')){
            Assert-P2Test ($source.Contains($proof)) "CUDA health cleanup status is not checked: $proof"}
    }
    Invoke-P2Test 'failed unreferenced candidate invocation is classified by aggregate ownership' {
        $fixture=New-P2AcceptedProtocolFixture;$failedId='C99';$suffix='correctness-allocation'
        $fixture.commands+=@(New-P2ProtocolCommand $failedId @('p2-burn-cubecl.exe','--mode','correctness','--workload','allocation','--fixture-dir','${TEMP}/fixtures',
                    '--output',"`${TEMP}/invocations/burn-cubecl/$suffix/raw-result.json") "`${TEMP}/invocations/burn-cubecl/$suffix")
        Assert-P2Test (&$module {param($c,$a,$o)Assert-P2AcceptedCommandProtocol $c $a $o} $fixture.commands $fixture.aggregates @{C99='burn-cubecl'}) 'owned failed candidate invocation was rejected'
        $threw=$false;try{$null=&$module {param($c,$a,$o)Assert-P2AcceptedCommandProtocol $c $a $o} $fixture.commands $fixture.aggregates @{C99='candle'}}catch{$threw=$true}
        Assert-P2Test $threw 'failed candidate invocation with wrong owner was accepted'
    }
    Invoke-P2Test 'failed-peer benchmark attempts retain exact global ABBA slots and paths' {
        $schedule=@(
            @('burn-cubecl',1,'projection'),@('candle',2,'projection'),@('candle',3,'projection'),@('burn-cubecl',4,'projection'),
            @('burn-cubecl',1,'ffn-expansion'),@('candle',2,'ffn-expansion'),@('candle',3,'ffn-expansion'),@('burn-cubecl',4,'ffn-expansion'))
        $rows=[Collections.Generic.List[object]]::new();$order=0
        foreach($slot in $schedule){$order++;$candidate=$slot[0];$round=[int]$slot[1];$workload=$slot[2];$suffix="round-$round-$workload"
            $command=New-P2ProtocolCommand ("C$($order+50)") @("p2-$candidate.exe",'--mode','benchmark','--workload',$workload,'--fixture-dir','${TEMP}/fixtures','--output',"`${TEMP}/invocations/$candidate/$suffix/raw-result.json") "`${TEMP}/invocations/$candidate/$suffix"
            $parsed=&$module {param($c,$i)Get-P2AttemptedBenchmarkSlot $c $i} $command $candidate
            $rows.Add([pscustomobject]@{candidate_id=$parsed.candidate_id;round=$parsed.round;workload=$parsed.workload;order=$order})}
        Assert-P2Test (&$module {param($r)Assert-P2AttemptedBenchmarkSchedule $r} @($rows)) 'canonical failed-peer schedule was rejected'
        $bad=Copy-P2TestObject @($rows);$bad[0].round=4;$threw=$false;try{$null=&$module {param($r)Assert-P2AttemptedBenchmarkSchedule $r} $bad}catch{$threw=$true}
        Assert-P2Test $threw 'swapped failed-peer round was accepted'
        $command=New-P2ProtocolCommand C99 @('p2-burn-cubecl.exe','--mode','benchmark','--workload','projection','--fixture-dir','${TEMP}/wrong','--output','${TEMP}/invocations/burn-cubecl/round-1-projection/raw-result.json') '${TEMP}/invocations/burn-cubecl/round-1-projection'
        $threw=$false;try{$null=&$module {param($c)Get-P2AttemptedBenchmarkSlot $c burn-cubecl} $command}catch{$threw=$true};Assert-P2Test $threw 'mis-targeted failed-peer fixture path was accepted'
    }
    Invoke-P2Test 'global transcript audit rejects versioned Python tools and linkage' {
        foreach($text in @('running python.exe --version','C:\tools\python3.13.exe build.py','pip3 install x','loading libpython313.dll','P2_CANARY_HIT')){
            Assert-P2Test (&$module {param($t)Test-P2PythonTranscriptViolation $t} $text) "global Python evidence was missed: $text"}
        Assert-P2Test (-not(&$module {Test-P2PythonTranscriptViolation 'cargo build --locked --offline'})) 'clean global transcript was rejected'
    }
    Invoke-P2Test 'global transcript audit ignores Python prose package names and public URLs' {
        foreach($text in @(
                'CARGO_PKG_DESCRIPTION="Split a string into shell words, like Python''s shlex."',
                'tree-sitter-python v0.23.6',
                'CARGO_PKG_REPOSITORY=https://github.com/tree-sitter/tree-sitter-python')){
            Assert-P2Test (-not(&$module {param($t)Test-P2PythonTranscriptViolation $t} $text)) "benign global Python text was rejected: $text"
        }
    }
    Invoke-P2Test 'runtime provenance requires GPU boundary but CPU forbids it' {
        $emptyGpu=Get-P2LoadedModuleProvenance -LoadedModules @() -CudaToolkitRoot $repositoryRoot -WindowsRoot $env:SystemRoot -ModuleAudit $moduleAudit -DriverPackageAnchor '${WINDOWS}/System32/DriverStore/FileRepository/fake' -CandidateId burn-cubecl
        Assert-P2Test (-not$emptyGpu.all_allowed) 'empty GPU provenance accepted'
        $emptyCpu=Get-P2LoadedModuleProvenance -LoadedModules @() -CudaToolkitRoot $repositoryRoot -WindowsRoot $env:SystemRoot -ModuleAudit $moduleAudit -DriverPackageAnchor '${WINDOWS}/System32/DriverStore/FileRepository/fake' -CandidateId burn-cubecl -CpuMode
        Assert-P2Test $emptyCpu.all_allowed 'clean CPU provenance rejected'
        $failedAudit=Copy-P2TestObject $moduleAudit;$failedAudit.successful_snapshots=0;$failedAudit.failed_snapshots=1;$failedAudit.last_error='Win32Exception'
        $failed=Get-P2LoadedModuleProvenance -LoadedModules @() -CudaToolkitRoot $repositoryRoot -WindowsRoot $env:SystemRoot -ModuleAudit $failedAudit -DriverPackageAnchor '${WINDOWS}/System32/DriverStore/FileRepository/fake' -CandidateId burn-cubecl -CpuMode
        Assert-P2Test (-not$failed.all_allowed) 'failed module snapshot audit was accepted'
    }
    Invoke-P2Test 'live driver inventory is P1B anchored signed and complete' {
        $manifest=Get-Content (Join-Path $repositoryRoot 'docs\receipts\P1B\runs\20260811T174734119Z-7e7135b7cb794eb791c0e607\artifacts\environment.json') -Raw|ConvertFrom-Json
        $null=&$module {Initialize-P2NativeInterop;[P2NvmlMonitor]::EnsureQualifiedLibrary();[P2CudaHealth]::Probe()}
        $inventory=&$module {param($v)Get-P2QualifiedDriverModuleInventory $v $env:SystemRoot} $manifest
        Assert-P2Test (&$module {param($i)Assert-P2DriverModuleInventory $i} $inventory) 'closed driver inventory validator rejected the live inventory'
        Assert-P2Test ([string]$inventory.package_anchor-cmatch'^\$\{WINDOWS\}/System32/DriverStore/FileRepository/[^/]+$') 'driver package anchor is not canonical'
        $paths=@($inventory.observed_modules.path)
        Assert-P2Test ('${WINDOWS}/System32/nvcuda.dll'-in$paths-and@($paths|Where-Object{$_-match'/nvcuda64\.dll$'}).Count-eq1) 'driver inventory lacks required CUDA loader pair'
        Assert-P2Test (@($inventory.observed_modules).Count-ge2-and@($paths|Sort-Object -Unique).Count-eq$paths.Count) 'observed driver module inventory is empty or duplicated'
        foreach($record in @($inventory.observed_modules)){
            $resolved=&$module {param($t,$a)Resolve-P2RetainedRuntimeModule $t $env:CUDA_PATH $env:SystemRoot $a} $record.path $inventory.package_anchor
            Assert-P2Test ((Get-P2Sha256 $resolved)-ceq[string]$record.sha256) 'driver inventory record does not live-rehash'
        }
        $duplicate=Copy-P2TestObject $inventory;$duplicate.observed_modules=@($duplicate.observed_modules)+@($duplicate.observed_modules[0])
        $threw=$false;try{$null=&$module {param($i)Assert-P2DriverModuleInventory $i} $duplicate}catch{$threw=$true}
        Assert-P2Test $threw 'duplicate host driver inventory path was accepted'
        $missing=Copy-P2TestObject $inventory;$missing.observed_modules=@($missing.observed_modules|Where-Object path -ne '${WINDOWS}/System32/nvcuda.dll')
        $threw=$false;try{$null=&$module {param($i)Assert-P2DriverModuleInventory $i} $missing}catch{$threw=$true}
        Assert-P2Test $threw 'host driver inventory without nvcuda.dll was accepted'
    }
    Invoke-P2Test 'retained runtime resolver rejects second packages and arbitrary System32 modules' {
        $manifest=Get-Content (Join-Path $repositoryRoot 'docs\receipts\P1B\runs\20260811T174734119Z-7e7135b7cb794eb791c0e607\artifacts\environment.json') -Raw|ConvertFrom-Json
        $inventory=&$module {param($v)Initialize-P2NativeInterop;$null=[P2NvmlMonitor]::EnsureQualifiedLibrary();$null=[P2CudaHealth]::Probe();Get-P2QualifiedDriverModuleInventory $v $env:SystemRoot} $manifest
        foreach($token in @('${WINDOWS}/System32/DriverStore/FileRepository/other-package/nvcuda64.dll','${WINDOWS}/System32/kernel32.dll')){
            $threw=$false;try{$null=&$module {param($t,$a)Resolve-P2RetainedRuntimeModule $t $env:CUDA_PATH $env:SystemRoot $a} $token $inventory.package_anchor}catch{$threw=$true}
            Assert-P2Test $threw "unqualified retained runtime token was accepted: $token"
        }
    }
    Invoke-P2Test 'acceptance runtime validator rejects hash package and System32 mutations' {
        $manifest=Get-Content (Join-Path $repositoryRoot 'docs\receipts\P1B\runs\20260811T174734119Z-7e7135b7cb794eb791c0e607\artifacts\environment.json') -Raw|ConvertFrom-Json
        $inventory=&$module {param($v)Initialize-P2NativeInterop;$null=[P2NvmlMonitor]::EnsureQualifiedLibrary();$null=[P2CudaHealth]::Probe();Get-P2QualifiedDriverModuleInventory $v $env:SystemRoot} $manifest
        $expected=@{};foreach($record in @($inventory.observed_modules)){$expected[[string]$record.path]=[string]$record.sha256}
        $records=Copy-P2TestObject @($inventory.observed_modules)
        Assert-P2Test (&$module {param($r,$e,$i)Assert-P2RetainedRuntimeModules $r burn-cubecl $e $i $env:CUDA_PATH $env:SystemRoot} $records $expected $inventory) 'acceptance runtime validator rejected live retained modules'
        $wrong=Copy-P2TestObject $records;$wrong[0].sha256='0'*64
        $threw=$false;try{$null=&$module {param($r,$e,$i)Assert-P2RetainedRuntimeModules $r burn-cubecl $e $i $env:CUDA_PATH $env:SystemRoot} $wrong $expected $inventory}catch{$threw=$true}
        Assert-P2Test $threw 'acceptance runtime validator accepted a wrong live hash'
        $second=Copy-P2TestObject $records;$packageIndex=0..($second.Count-1)|Where-Object{$second[$_].path-match'/nvcuda64\.dll$'}|Select-Object -First 1
        $second[$packageIndex].path='${WINDOWS}/System32/DriverStore/FileRepository/other-package/nvcuda64.dll'
        $threw=$false;try{$null=&$module {param($r,$e,$i)Assert-P2RetainedRuntimeModules $r burn-cubecl $e $i $env:CUDA_PATH $env:SystemRoot} $second $expected $inventory}catch{$threw=$true}
        Assert-P2Test $threw 'acceptance runtime validator accepted a second driver package'
        $system=Copy-P2TestObject $records;$systemIndex=0..($system.Count-1)|Where-Object{$system[$_].path-ceq'${WINDOWS}/System32/nvcuda.dll'}|Select-Object -First 1
        $system[$systemIndex].path='${WINDOWS}/System32/kernel32.dll';$system[$systemIndex].sha256=Get-P2Sha256 (Join-Path $env:SystemRoot 'System32\kernel32.dll')
        $threw=$false;try{$null=&$module {param($r,$e,$i)Assert-P2RetainedRuntimeModules $r burn-cubecl $e $i $env:CUDA_PATH $env:SystemRoot} $system $expected $inventory}catch{$threw=$true}
        Assert-P2Test $threw 'acceptance runtime validator accepted an arbitrary System32 DLL'
    }
    Invoke-P2Test 'Candle runtime provenance rejects dynamically loaded cuDNN' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-cudnn-'+[Guid]::NewGuid().ToString('N'))
        try{[void][IO.Directory]::CreateDirectory($root);$dll=Join-Path $root 'cudnn64_9.dll';Write-P2Utf8LfFile $dll 'fake' -CreateNew
            $p=Get-P2LoadedModuleProvenance -LoadedModules @($dll) -CudaToolkitRoot $root -WindowsRoot $env:SystemRoot -ModuleAudit $moduleAudit -DriverPackageAnchor '${WINDOWS}/System32/DriverStore/FileRepository/fake' -CandidateId candle
            Assert-P2Test (-not$p.all_allowed) 'Candle dynamically loaded cuDNN was accepted'
        }finally{if(Test-Path $root){Remove-Item $root -Recurse -Force}}
    }
    Invoke-P2Test 'NVML monitor provenance rejects non-System32 library paths' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-nvml-'+[Guid]::NewGuid().ToString('N'))
        try{[void][IO.Directory]::CreateDirectory($root);$dll=Join-Path $root 'nvml.dll';Write-P2Utf8LfFile $dll 'fake' -CreateNew
            $threw=$false;try{$null=&$module {param($p,$w)New-P2NvmlLibraryRecord $p $w} $dll $env:SystemRoot}catch{$threw=$true}
            Assert-P2Test $threw 'non-System32 NVML library was accepted'
        }finally{if(Test-Path $root){Remove-Item $root -Recurse -Force}}
    }
    Invoke-P2Test 'NVML policy rejects any retained baseline or sampling gap over 100 ms' {
        $reference=[pscustomobject]@{command_id='C42';round=1;workload='projection'}
        $measurement=[pscustomobject]@{command_id='C42';round=1;workload='projection';baseline_samples=20;baseline_interval_ms=50;sample_interval_ms=20
            total_bytes=[uint64]1000;baseline_bytes=[uint64]100;peak_bytes=[uint64]200;delta_bytes=[uint64]100;sample_count=1;max_gap_ms=100.0
            baseline_foreign_process_count=0;maximum_foreign_process_count=0;error=$null}
        Assert-P2Test (&$module {param($m,$r)Test-P2NvmlMeasurementPolicy $m $r} $measurement $reference) '100 ms gap boundary rejected'
        $measurement.max_gap_ms=101.0
        Assert-P2Test (-not(&$module {param($m,$r)Test-P2NvmlMeasurementPolicy $m $r} $measurement $reference)) 'gap over 100 ms accepted'
        $measurement.max_gap_ms=100.0;$measurement.maximum_foreign_process_count=1
        Assert-P2Test (-not(&$module {param($m,$r)Test-P2NvmlMeasurementPolicy $m $r} $measurement $reference)) 'foreign GPU work was accepted'
        $measurement.maximum_foreign_process_count=0;$measurement.baseline_foreign_process_count=1
        Assert-P2Test (-not(&$module {param($m,$r)Test-P2NvmlMeasurementPolicy $m $r} $measurement $reference)) 'baseline foreign GPU work was accepted'
    }
    Invoke-P2Test 'between-round drift invalidates the shared measurement run' {
        $results=@(
            (New-P2TestCandidateResult -Mode benchmark -Workload projection),
            (New-P2TestCandidateResult -Mode benchmark -Workload projection),
            (New-P2TestCandidateResult -Mode benchmark -Workload ffn-expansion),
            (New-P2TestCandidateResult -Mode benchmark -Workload ffn-expansion))
        $series=$results[1].timing.forward_backward
        $series.samples_ns=@(1..200|ForEach-Object{[int64](30000000+$_)})
        $series.p50_ns=$series.samples_ns[99];$series.p95_ns=$series.samples_ns[189]
        $series.gflops=[double]$series.flop_count/[double]$series.p50_ns
        $threw=$false;try{$null=Get-P2CandidateComparison -CandidateId candle -BenchmarkResults $results -LockedDependencyCount 149 -ObservedPeakBytes 1}catch{$threw=$true}
        Assert-P2Test $threw 'greater-than-ten-percent between-round drift was accepted'
    }
    Invoke-P2Test 'a passing peer cannot mask a measurement-invalid candidate' {
        $peer=[pscustomobject]@{candidate_id='burn-cubecl';status='PASS'}
        Assert-P2Test ($peer.status-ceq'PASS') 'peer fixture is not passing'
        foreach($reason in @('foreign GPU process observed','NVML sampling gap exceeded 100 ms','between-round p50 drift exceeded ten percent')){
            $threw=$false;$code=$null;$category=$null
            try{&$module {param($m)Stop-P2RunForInvalidMeasurement candle ([InvalidOperationException]::new($m))} $reason}
            catch{$threw=$true;$code=$_.Exception.Data['P2Code'];$category=$_.Exception.Data['P2Category']}
            Assert-P2Test ($threw-and$code-ceq'MEASUREMENT_INVALID'-and[int]$category-eq5) "measurement invalidity was downgraded to candidate-local: $reason"
        }
        $failedInvocation=[pscustomobject]@{result=[pscustomobject]@{status='FAIL'};command=[pscustomobject]@{id='C42'};nvml=[pscustomobject]@{
                BaselineBytes=[uint64]100;PeakUsedBytes=[uint64]200;TotalBytes=[uint64]1000;Samples=10;MaxGapMs=101.0
                BaselineForeignProcessCount=0;MaximumForeignProcessCount=0;Error=$null}}
        $threw=$false;try{$null=&$module {param($v)Get-P2RunNvmlMeasurement $v 1 projection candle} $failedInvocation}catch{$threw=$true;$code=$_.Exception.Data['P2Code']}
        Assert-P2Test ($threw-and$code-ceq'MEASUREMENT_INVALID') 'failed candidate monitor evidence bypassed run-global validation'
    }
    Invoke-P2Test 'dependency policy rejects every inexact direct registry form' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-inexact-'+[Guid]::NewGuid().ToString('N'))
        try{[void][IO.Directory]::CreateDirectory($root);$manifest=Join-Path $root 'Cargo.toml';$lock=Join-Path $root 'Cargo.lock'
            $manifestText = "[workspace.dependencies]`nburn = { version = `"0.21`" }"
            Write-P2Utf8LfFile -Path $manifest -Text $manifestText -CreateNew
            Write-P2Utf8LfFile -Path $lock -Text 'version = 4' -CreateNew
            $threw=$false;try{[void](Test-P2DependencyPolicy -Manifest $manifest -Lock $lock)}catch{$threw=$true};Assert-P2Test $threw 'inexact registry version accepted'}finally{if(Test-Path $root){Remove-Item $root -Recurse -Force}}
    }
    Invoke-P2Test 'dependency policy rejects compound requirements disguised as exact pins' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-compound-'+[Guid]::NewGuid().ToString('N'))
        try{[void][IO.Directory]::CreateDirectory($root);$manifest=Join-Path $root 'Cargo.toml';$lock=Join-Path $root 'Cargo.lock'
            Write-P2Utf8LfFile $manifest "[dependencies]`nburn = `"=0.21.0, <1.0`"" -CreateNew;Write-P2Utf8LfFile $lock 'version = 4' -CreateNew
            $threw=$false;try{$null=Test-P2DependencyPolicy $manifest $lock}catch{$threw=$true};Assert-P2Test $threw 'compound semver requirement was accepted'
        }finally{if(Test-Path $root){Remove-Item $root -Recurse -Force}}
    }
    Invoke-P2Test 'dependency policy accepts only true Cargo workspace dotted-key inheritance' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-workspace-inherit-'+[Guid]::NewGuid().ToString('N'))
        try{[void][IO.Directory]::CreateDirectory((Join-Path $root 'child'))
            $manifest=@'
[workspace]
members = ["child"]

[workspace.dependencies]
burn = { version = "=0.21.0" }
candle-core = { version = "=0.11.0" }
cudarc = { version = "=0.19.8" }
half = { version = "=2.7.1" }
'@
            $child=@'
[package]
name = "fixture"
version = "0.0.0"

[dependencies]
burn.workspace = true
'@
            Write-P2Utf8LfFile (Join-Path $root 'Cargo.toml') $manifest -CreateNew
            Write-P2Utf8LfFile (Join-Path $root 'child\Cargo.toml') $child -CreateNew
            Write-P2Utf8LfFile (Join-Path $root 'Cargo.lock') 'version = 4' -CreateNew
            $result=Test-P2DependencyPolicy (Join-Path $root 'Cargo.toml') (Join-Path $root 'Cargo.lock')
            Assert-P2Test ($result.status-ceq'PASS'-and$result.manifest_count-eq2) 'valid dotted-key workspace inheritance was rejected'
            Write-P2Utf8LfFile (Join-Path $root 'child\Cargo.toml') ($child.Replace('workspace = true','workspace = false'))
            $threw=$false;try{$null=Test-P2DependencyPolicy (Join-Path $root 'Cargo.toml') (Join-Path $root 'Cargo.lock')}catch{$threw=$true}
            Assert-P2Test $threw 'false dotted-key workspace inheritance was accepted'
        }finally{if(Test-Path $root){Remove-Item $root -Recurse -Force}}
    }
    Invoke-P2Test 'checked-in experiment manifests satisfy the exact dependency policy' {
        $root=Join-Path $repositoryRoot 'experiments\p2-backends'
        $result=Test-P2DependencyPolicy (Join-Path $root 'Cargo.toml') (Join-Path $root 'Cargo.lock')
        Assert-P2Test ($result.status-ceq'PASS'-and$result.manifest_count-eq5) 'checked-in experiment dependency graph was rejected'
    }
    Invoke-P2Test 'GPU identity compares exact P1B bytes with nvidia-smi whole MiB' {
        $reported=&$module { ConvertTo-P2NvidiaSmiMemoryMiB -Bytes 34190458880 }
        Assert-P2Test ($reported-eq32607) 'P1B exact memory bytes did not normalize to the sealed nvidia-smi value'
        $threw=$false;try{$null=&$module { ConvertTo-P2NvidiaSmiMemoryMiB -Bytes 0 }}catch{$threw=$true}
        Assert-P2Test $threw 'invalid P1B memory byte count was accepted'
    }
    Invoke-P2Test 'runtime provenance rejects Python DLLs and copied driver libraries' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-runtime-boundary-'+[Guid]::NewGuid().ToString('N'))
        try{[void][IO.Directory]::CreateDirectory($root)
            foreach($leaf in @('python311.dll','libpython312.dll','nvml.dll','nvcuda.dll')){Write-P2Utf8LfFile (Join-Path $root $leaf) 'fake' -CreateNew}
            foreach($leaf in @('python311.dll','libpython312.dll','nvml.dll','nvcuda.dll')){$p=Get-P2LoadedModuleProvenance -LoadedModules @((Join-Path $root $leaf)) -CudaToolkitRoot $root -WindowsRoot $env:SystemRoot -ModuleAudit $moduleAudit -DriverPackageAnchor '${WINDOWS}/System32/DriverStore/FileRepository/fake' -CandidateId candle
                Assert-P2Test (-not$p.all_allowed) "copied or Python module was accepted: $leaf"}
        }finally{if(Test-Path $root){Remove-Item $root -Recurse -Force}}
    }
    Invoke-P2Test 'redaction rejects host and user identities plus CR and BOM' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-redact-'+[Guid]::NewGuid().ToString('N'))
        try{[void][IO.Directory]::CreateDirectory($root);$identity=if($env:COMPUTERNAME){$env:COMPUTERNAME}else{$env:USERNAME};Write-P2Utf8LfFile (Join-Path $root 'x.txt') $identity -CreateNew
            $threw=$false;try{[void](& $module {param($p)Test-P2ReceiptRedaction $p} $root)}catch{$threw=$true};Assert-P2Test $threw 'identity leak accepted'}finally{if(Test-Path $root){Remove-Item $root -Recurse -Force}}
    }
    Invoke-P2Test 'redaction allows public HTTPS but rejects Windows drive paths' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-redact-path-'+[Guid]::NewGuid().ToString('N'));$path=Join-Path $root 'x.txt'
        try{
            [void][IO.Directory]::CreateDirectory($root);Write-P2Utf8LfFile $path 'CARGO_PKG_REPOSITORY=https://github.com/tree-sitter/tree-sitter-python' -CreateNew
            Assert-P2Test (&$module {param($p)Test-P2ReceiptRedaction $p} $root) 'public HTTPS URL was rejected as a drive path'
            foreach($drivePath in @('C:\Users\example\artifact.txt','C:/Users/example/artifact.txt')){
                Write-P2Utf8LfFile $path $drivePath;$threw=$false
                try{[void](&$module {param($p)Test-P2ReceiptRedaction $p} $root)}catch{$threw=$true}
                Assert-P2Test $threw "Windows drive path was accepted: $drivePath"
            }
        }finally{if(Test-Path $root){Remove-Item $root -Recurse -Force}}
    }
    Invoke-P2Test 'sealed failure run remains immutable and never publishes a pointer' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-fail-'+[Guid]::NewGuid().ToString('N'));$run=Join-Path $root 'runs\20260812T010203004Z-0123456789abcdef01234567'
        try{[void][IO.Directory]::CreateDirectory($run);Write-P2JsonFile (Join-Path $run 'evidence.json') ([ordered]@{status='FAIL'}) -CreateNew;[void](New-P2Seal $run)
            Assert-P2Test (Test-P2Seal $run) 'failure seal invalid';Assert-P2Test (-not(Test-Path (Join-Path $root 'evidence.json'))) 'failure advanced pointer'
        }finally{if(Test-Path $root){Remove-Item $root -Recurse -Force}}
    }
    Invoke-P2Test 'early failure writer emits the complete frozen layout and seal' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-early-failure-'+[Guid]::NewGuid().ToString('N'));$id='20260812T010203004Z-0123456789abcdef01234567'
        try{[void][IO.Directory]::CreateDirectory((Join-Path $root 'artifacts'));$watch=[Diagnostics.Stopwatch]::StartNew()
            $errorRecord=[ordered]@{code='EARLY_FAILURE';category=5;message='Early bounded failure.';remediation='Correct the fixture and retry.'}
            $evidence=&$module {param($r,$i,$w,$e)Write-P2FailureRun -RunRoot $r -RunId $i -StartedAt ([DateTime]::UtcNow) -Stopwatch $w `
                    -SourceIdentitySha256 ('0'*64) -P1BDependency $null -Gates (New-P2GateSet) -Commands @() -Errors @($e) `
                    -CleanupAttempted $true -TemporaryRootRemoved $true -ProcessTreesTerminated $true -Policy $null `
                    -DependencyInventory $null -FixtureManifest $null -HostState $null -CpuIsolation $null} $root $id $watch $errorRecord
            $required=@('source-identity.json','p1b-dependency.json','qualification-policy.json','dependency-inventory.json','fixture-manifest.json',
                'host-state.json','burn-cubecl.json','candle.json','comparison.json','decision.json','cpu-isolation.json','failure-summary.json')
            foreach($name in $required){Assert-P2Test (Test-Path (Join-Path $root "artifacts\$name") -PathType Leaf) "early failure artifact missing: $name"}
            Assert-P2Test (Test-Path (Join-Path $root 'commands') -PathType Container) 'early failure commands directory is missing'
            Assert-P2Test ($evidence.status-ceq'FAIL'-and@($evidence.candidates).Count-eq2) 'early failure evidence is incomplete'
            foreach($ref in @($evidence.candidates)){Assert-P2Test ($ref.sha256-cmatch'^[0-9a-f]{64}$'-and[int64]$ref.bytes-gt0) 'NOT_RUN candidate was not hash-bound'}
            Assert-P2Test (Test-P2Seal $root) 'early failure layout seal is invalid'
        }finally{if(Test-Path $root){Remove-Item $root -Recurse -Force}}
    }
    Invoke-P2Test 'failure writer preserves an attempted diagnostic fallback' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-fallback-failure-'+[Guid]::NewGuid().ToString('N'));$id='20260812T010203004Z-0123456789abcdef01234567'
        try{[void][IO.Directory]::CreateDirectory((Join-Path $root 'artifacts'));$watch=[Diagnostics.Stopwatch]::StartNew()
            Write-P2JsonFile (Join-Path $root 'artifacts\cudarc-fallback.json') (&$module {New-P2NotRunAggregate cudarc-fallback diagnostic_fallback}) -CreateNew
            $errorRecord=[ordered]@{code='FALLBACK_FAILURE';category=5;message='Fallback bounded failure.';remediation='Inspect the diagnostic receipt.'}
            $evidence=&$module {param($r,$i,$w,$e)Write-P2FailureRun -RunRoot $r -RunId $i -StartedAt ([DateTime]::UtcNow) -Stopwatch $w `
                    -SourceIdentitySha256 ('0'*64) -P1BDependency $null -Gates (New-P2GateSet) -Commands @() -Errors @($e) `
                    -CleanupAttempted $true -TemporaryRootRemoved $true -ProcessTreesTerminated $true -Policy $null `
                    -DependencyInventory $null -FixtureManifest $null -HostState $null -CpuIsolation $null} $root $id $watch $errorRecord
            Assert-P2Test ((@($evidence.candidates.candidate_id)-join',')-ceq'burn-cubecl,candle,cudarc-fallback') 'attempted fallback reference was dropped'
            Assert-P2Test (Test-P2Seal $root) 'fallback failure layout seal is invalid'
        }finally{if(Test-Path $root){Remove-Item $root -Recurse -Force}}
    }
    Invoke-P2Test 'entrypoint invalid output leaves stdout empty and emits terminal JSONL error' {
        $command=(Get-Command powershell.exe -ErrorAction Stop).Source
        $process=Invoke-P2Process -FilePath $command -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repositoryRoot 'scripts\qualify-backend.ps1'),'-OutputRoot','docs\receipts\P1B') -WorkingDirectory $repositoryRoot -TimeoutSeconds 30
        Assert-P2Test (-not $process.timed_out -and [int]$process.exit_code -eq 2) 'invalid root did not use usage/configuration exit 2'
        Assert-P2Test ([string]::IsNullOrEmpty([string]$process.stdout)) 'handled failure wrote stdout'
        $lines=@(([string]$process.stderr -split "`n")|Where-Object{$_})
        Assert-P2Test ($lines.Count -eq 2) 'stderr terminal record count wrong'
        foreach($line in $lines){$null=$line|ConvertFrom-Json}
    }
    Invoke-P2Test 'late unpublished PASS converts atomically to sealed FAIL evidence' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-late-'+[Guid]::NewGuid().ToString('N'))
        try{
            [void][IO.Directory]::CreateDirectory($root)
            Write-P2JsonFile -Path (Join-Path $root 'evidence.json') -Value ([ordered]@{run_id='20260812T010203004Z-0123456789abcdef01234567';status='PASS';errors=@();finished_at='2026-08-12T00:00:00.0000000Z';seal=[ordered]@{entries=1}}) -CreateNew
            [void](New-P2Seal $root)
            $errorRecord=[ordered]@{code='PUBLICATION_FAILED';category=1;message='publication failed';remediation='retry'}
            $converted=& $module {param($r,$e)Convert-P2UnpublishedPassToFailure -RunRoot $r -ErrorRecord $e} $root $errorRecord
            $evidence=Get-Content (Join-Path $root 'evidence.json') -Raw|ConvertFrom-Json
            Assert-P2Test ($converted-and$evidence.status-ceq'FAIL'-and@($evidence.errors).Count-eq1) 'late PASS did not become terminal FAIL'
            Assert-P2Test (Test-P2Seal $root) 'converted failure was not resealed'
        }finally{if(Test-Path $root){Remove-Item $root -Recurse -Force}}
    }
    Invoke-P2Test 'publication chains acceptances and atomically rolls back pointer failure' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-publish-'+[Guid]::NewGuid().ToString('N'))
        $fingerprint='f'*64;$source='a'*64
        $originals=&$module {[pscustomobject]@{pass=${function:Assert-P2PassRun};identity=${function:Get-P2RepositoryIdentity};acceptance=${function:Assert-P2Acceptance}}}
        try{
            [void][IO.Directory]::CreateDirectory((Join-Path $root 'runs'))
            foreach($id in @('20260812T010203004Z-0123456789abcdef01234567','20260812T010203005Z-1123456789abcdef01234567','20260812T010203006Z-2123456789abcdef01234567')){[void][IO.Directory]::CreateDirectory((Join-Path $root "runs\$id"))}
            & $module {
                Set-Item Function:script:Assert-P2PassRun {param($RunRoot,$RunId,$SourceIdentitySha256)[pscustomobject]@{evidence_sha256='1'*64;seal_sha256='2'*64;decision_sha256='3'*64}}
                Set-Item Function:script:Get-P2RepositoryIdentity {param($RepositoryRoot)[pscustomobject]@{fingerprint='f'*64}}
                Set-Item Function:script:Assert-P2Acceptance {param($Acceptance,$Sequence,$PreviousHash,$OutputRoot)}
            }
            $first=Publish-P2Acceptance -OutputRoot $root -RunId '20260812T010203004Z-0123456789abcdef01234567' -SourceIdentitySha256 $source -RepositoryRoot $repositoryRoot -SourceInputFingerprint $fingerprint
            $second=Publish-P2Acceptance -OutputRoot $root -RunId '20260812T010203005Z-1123456789abcdef01234567' -SourceIdentitySha256 $source -RepositoryRoot $repositoryRoot -SourceInputFingerprint $fingerprint
            Assert-P2Test ($first.sequence-eq1-and$second.sequence-eq2) 'acceptance sequence did not advance'
            $a1=Get-Content (Join-Path $root 'acceptances\00000001.json') -Raw|ConvertFrom-Json
            $a2=Get-Content (Join-Path $root 'acceptances\00000002.json') -Raw|ConvertFrom-Json
            Assert-P2Test ([string]$a2.previous_acceptance_sha256-ceq(Get-P2Sha256 (Join-Path $root 'acceptances\00000001.json'))) 'acceptance chain hash is wrong'
            $before=[IO.File]::ReadAllBytes((Join-Path $root 'evidence.json'))
            & $module {$script:TestAcceptanceCall=0;Set-Item Function:script:Assert-P2Acceptance {param($Acceptance,$Sequence,$PreviousHash,$OutputRoot)$script:TestAcceptanceCall++;if($script:TestAcceptanceCall-eq4){throw 'injected post-pointer validation failure'}}}
            $threw=$false;try{Publish-P2Acceptance -OutputRoot $root -RunId '20260812T010203006Z-2123456789abcdef01234567' -SourceIdentitySha256 $source -RepositoryRoot $repositoryRoot -SourceInputFingerprint $fingerprint}catch{$threw=$true}
            $after=[IO.File]::ReadAllBytes((Join-Path $root 'evidence.json'))
            Assert-P2Test $threw 'injected publication failure was ignored'
            Assert-P2Test ([Convert]::ToBase64String($before)-ceq[Convert]::ToBase64String($after)) 'previous pointer was not restored byte-for-byte'
            Assert-P2Test (-not(Test-Path (Join-Path $root 'acceptances\00000003.json'))) 'failed publication left an orphan acceptance'
            Assert-P2Test (@(Get-ChildItem $root -Filter '.*.tmp' -File -Recurse).Count-eq0) 'publication left transient files'
        }finally{
            & $module {param($o)Set-Item Function:script:Assert-P2PassRun $o.pass;Set-Item Function:script:Get-P2RepositoryIdentity $o.identity;Set-Item Function:script:Assert-P2Acceptance $o.acceptance} $originals
            if(Test-Path $root){Remove-Item $root -Recurse -Force}
        }
    }
    Invoke-P2Test 'concurrent publishers serialize into one gap-free acceptance chain' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-concurrent-publish-'+[Guid]::NewGuid().ToString('N'))
        $runIds=@('20260812T010203014Z-3123456789abcdef01234567','20260812T010203015Z-4123456789abcdef01234567')
        $jobs=@();try{
            [void][IO.Directory]::CreateDirectory((Join-Path $root 'runs'));foreach($id in $runIds){[void][IO.Directory]::CreateDirectory((Join-Path $root "runs\$id"))}
            $worker={param($ModulePath,$OutputRoot,$RunId,$Repo)
                $m=Import-Module -Name $ModulePath -Force -PassThru
                &$m {
                    Set-Item Function:script:Assert-P2PassRun {param($RunRoot,$RunId,$SourceIdentitySha256)[pscustomobject]@{evidence_sha256='1'*64;seal_sha256='2'*64;decision_sha256='3'*64}}
                    Set-Item Function:script:Get-P2RepositoryIdentity {param($RepositoryRoot)[pscustomobject]@{fingerprint='f'*64}}
                    Set-Item Function:script:Assert-P2Acceptance {param($Acceptance,$Sequence,$PreviousHash,$OutputRoot)}
                }
                Publish-P2Acceptance -OutputRoot $OutputRoot -RunId $RunId -SourceIdentitySha256 ('a'*64) -RepositoryRoot $Repo -SourceInputFingerprint ('f'*64)
            }
            foreach($id in $runIds){$jobs+=Start-Job -ScriptBlock $worker -ArgumentList $modulePath,$root,$id,$repositoryRoot}
            $done=@(Wait-Job -Job $jobs -Timeout 45);Assert-P2Test ($done.Count-eq2) 'concurrent publishers did not finish within the bound'
            foreach($job in $jobs){$failure=$null;$null=Receive-Job -Job $job -ErrorVariable failure -ErrorAction SilentlyContinue
                Assert-P2Test ($job.State-ceq'Completed'-and@($failure).Count-eq0) 'a concurrent publisher failed'}
            $acceptances=@(Get-ChildItem (Join-Path $root 'acceptances') -Filter '*.json'|Sort-Object Name)
            Assert-P2Test (($acceptances.Name-join',')-ceq'00000001.json,00000002.json') 'concurrent publication created a gap or collision'
            $second=Get-Content $acceptances[1].FullName -Raw|ConvertFrom-Json
            Assert-P2Test ([string]$second.previous_acceptance_sha256-ceq(Get-P2Sha256 $acceptances[0].FullName)) 'concurrent acceptance chain did not bind its predecessor'
            $pointer=Get-Content (Join-Path $root 'evidence.json') -Raw|ConvertFrom-Json
            Assert-P2Test ([string]$pointer.acceptance_path-ceq'acceptances/00000002.json') 'concurrent publication pointer did not select the terminal acceptance'
        }finally{foreach($job in @($jobs)){if($null-ne$job){Remove-Job -Job $job -Force -ErrorAction SilentlyContinue}};if(Test-Path $root){Remove-Item $root -Recurse -Force}}
    }
    Invoke-P2Test 'repository mutation during the publication mutex rejects pointer advance' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-publish-mutation-'+[Guid]::NewGuid().ToString('N'));$id='20260812T010203016Z-5123456789abcdef01234567'
        $fixture=Join-Path $repositoryRoot ('p2-concurrent-source-'+[Guid]::NewGuid().ToString('N')+'.rs')
        $readyName='Local\p2-ready-'+[Guid]::NewGuid().ToString('N');$continueName='Local\p2-continue-'+[Guid]::NewGuid().ToString('N')
        $ready=[Threading.EventWaitHandle]::new($false,[Threading.EventResetMode]::ManualReset,$readyName)
        $continue=[Threading.EventWaitHandle]::new($false,[Threading.EventResetMode]::ManualReset,$continueName);$job=$null
        try{
            [void][IO.Directory]::CreateDirectory((Join-Path $root "runs\$id"));$before=&$module {param($r)Get-P2RepositoryIdentity $r} $repositoryRoot
            $worker={param($ModulePath,$OutputRoot,$RunId,$Repo,$Fingerprint,$ReadyName,$ContinueName)
                $m=Import-Module -Name $ModulePath -Force -PassThru
                &$m {param($rn,$cn)
                    $script:P2TestReady=[Threading.EventWaitHandle]::OpenExisting($rn);$script:P2TestContinue=[Threading.EventWaitHandle]::OpenExisting($cn)
                    $script:P2TestOriginalIdentity=${function:Get-P2RepositoryIdentity}
                    Set-Item Function:script:Assert-P2PassRun {param($RunRoot,$RunId,$SourceIdentitySha256)[pscustomobject]@{evidence_sha256='1'*64;seal_sha256='2'*64;decision_sha256='3'*64}}
                    Set-Item Function:script:Get-P2RepositoryIdentity {param($RepositoryRoot)[void]$script:P2TestReady.Set();if(-not$script:P2TestContinue.WaitOne(30000)){throw 'test interleave timed out'};&$script:P2TestOriginalIdentity -RepositoryRoot $RepositoryRoot}
                } $ReadyName $ContinueName
                Publish-P2Acceptance -OutputRoot $OutputRoot -RunId $RunId -SourceIdentitySha256 ('a'*64) -RepositoryRoot $Repo -SourceInputFingerprint $Fingerprint
            }
            $job=Start-Job -ScriptBlock $worker -ArgumentList $modulePath,$root,$id,$repositoryRoot,$before.fingerprint,$readyName,$continueName
            Assert-P2Test ($ready.WaitOne(30000)) 'publisher never entered the mutex-protected source recheck'
            Write-P2Utf8LfFile $fixture 'concurrent mutation' -CreateNew;[void]$continue.Set();$null=Wait-Job $job -Timeout 40
            $failure=$null;$null=Receive-Job $job -ErrorVariable failure -ErrorAction SilentlyContinue
            Assert-P2Test (@($failure).Count-gt0-or$job.State-ceq'Failed') 'publication accepted a concurrent source mutation'
            Assert-P2Test (-not(Test-Path (Join-Path $root 'evidence.json'))) 'concurrent source mutation advanced the pointer'
            Assert-P2Test (@(Get-ChildItem (Join-Path $root 'acceptances') -Filter '*.json' -ErrorAction SilentlyContinue).Count-eq0) 'concurrent source mutation left an acceptance'
        }finally{[void]$continue.Set();if($null-ne$job){Remove-Job $job -Force -ErrorAction SilentlyContinue};$ready.Dispose();$continue.Dispose();if(Test-Path $fixture){Remove-Item $fixture -Force};if(Test-Path $root){Remove-Item $root -Recurse -Force}}
    }
    Invoke-P2Test 'real P2 receipt layout resolves exactly to repository root' {
        $run=Join-Path $repositoryRoot 'docs\receipts\P2\runs\20260812T010203004Z-0123456789abcdef01234567'
        $resolved=& $module {param($p)Get-P2RepositoryRootFromRunRoot -RunRoot $p} $run
        Assert-P2Test ([IO.Path]::GetFullPath($resolved)-ceq[IO.Path]::GetFullPath($repositoryRoot)) 'receipt run resolved to the wrong repository root'
        $wrong=Join-Path $repositoryRoot 'docs\receipts\P1B\runs\20260812T010203004Z-0123456789abcdef01234567';$threw=$false
        try{$null=& $module {param($p)Get-P2RepositoryRootFromRunRoot -RunRoot $p} $wrong}catch{$threw=$true}
        Assert-P2Test $threw 'non-P2 receipt layout was accepted'
    }
    Invoke-P2Test 'published pointer reread binds exact acceptance path and hash' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-pointer-'+[Guid]::NewGuid().ToString('N'))
        try{[void][IO.Directory]::CreateDirectory((Join-Path $root 'acceptances'))
            Write-P2Utf8LfFile (Join-Path $root 'acceptances\00000001.json') 'one' -CreateNew
            Write-P2Utf8LfFile (Join-Path $root 'acceptances\00000002.json') 'two' -CreateNew
            $hash=Get-P2Sha256 (Join-Path $root 'acceptances\00000001.json')
            $pointer=[pscustomobject][ordered]@{schema='python-slm-backend-phase-evidence-pointer-v1';phase_id='P2';acceptance_path='acceptances/00000002.json';acceptance_sha256=$hash;updated_at='2026-08-12T00:00:00Z'}
            $threw=$false;try{if($pointer.acceptance_path-cne'acceptances/00000001.json'){throw 'path mismatch'}}catch{$threw=$true}
            Assert-P2Test $threw 'pointer path substitution was accepted'
            Assert-P2Test ((Get-P2Sha256 (Join-Path $root 'acceptances\00000002.json'))-cne$pointer.acceptance_sha256) 'pointer hash fixture did not detect substitution'
        }finally{if(Test-Path $root){Remove-Item $root -Recurse -Force}}
    }
    Invoke-P2Test 'source identity detects repository input mutation and restoration' {
        $fixture=Join-Path $repositoryRoot ('p2-source-mutation-'+[Guid]::NewGuid().ToString('N')+'.rs')
        try{$before=&$module {param($r)Get-P2RepositoryIdentity $r} $repositoryRoot
            Write-P2Utf8LfFile $fixture 'after' -CreateNew;$after=&$module {param($r)Get-P2RepositoryIdentity $r} $repositoryRoot
            Assert-P2Test ($before.fingerprint-cne$after.fingerprint) 'repository input mutation was not detected'
            Remove-Item $fixture -Force;$restored=&$module {param($r)Get-P2RepositoryIdentity $r} $repositoryRoot
            Assert-P2Test ($before.fingerprint-ceq$restored.fingerprint) 'restored tracked input did not restore identity'
        }finally{if(Test-Path $fixture){Remove-Item $fixture -Force}}
    }
    Invoke-P2Test 'parent environment fingerprint detects injected drift' {
        $name='P2_TEST_ENV_'+[Guid]::NewGuid().ToString('N');$before=&$module {Get-P2EnvironmentFingerprint}
        try{[Environment]::SetEnvironmentVariable($name,'changed','Process');$after=&$module {Get-P2EnvironmentFingerprint}
            Assert-P2Test ($before-cne$after) 'parent environment drift was not detected'}finally{[Environment]::SetEnvironmentVariable($name,$null,'Process')}
        Assert-P2Test ($before-ceq(&$module {Get-P2EnvironmentFingerprint})) 'parent environment did not restore'
    }
    Invoke-P2Test 'selected P1B dependency artifact hash detects environment drift' {
        $root=Join-Path ([IO.Path]::GetTempPath()) ('p2-p1b-drift-'+[Guid]::NewGuid().ToString('N'));try{[void][IO.Directory]::CreateDirectory($root)
            $path=Join-Path $root 'environment.json';Write-P2Utf8LfFile $path '{"status":"PASS"}' -CreateNew;$expected=Get-P2Sha256 $path
            Write-P2Utf8LfFile $path '{"status":"FAIL"}';Assert-P2Test ((Get-P2Sha256 $path)-cne$expected) 'P1B environment drift retained its hash'
        }finally{if(Test-Path $root){Remove-Item $root -Recurse -Force}}
    }
}
finally {
    if ($null -ne $module -and (Get-Module -Name $module.Name -ErrorAction SilentlyContinue)) {
        Remove-Module -Name $module.Name -Force
    }
}

Write-Output "RESULT passed=$script:Passed failed=$script:Failed"
if ($script:Failures.Count -gt 0) {
    $script:Failures | ForEach-Object { [Console]::Error.WriteLine($_) }
    exit 1
}
exit 0

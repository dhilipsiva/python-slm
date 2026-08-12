Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:P2Utf8NoBom = [Text.UTF8Encoding]::new($false, $true)
$script:P2Lf = [char]10
$script:P2ReviewCommit = '8c822e51956543e3fa52aaf1f295a6b517e0fd40'
$script:P2Schema = [ordered]@{
    Policy = 'python-slm-backend-qualification-policy-v1'
    Candidate = 'python-slm-backend-candidate-result-v1'
    Evidence = 'python-slm-backend-phase-evidence-v1'
    Acceptance = 'python-slm-backend-phase-acceptance-v1'
    Pointer = 'python-slm-backend-phase-evidence-pointer-v1'
    Aggregate = 'python-slm-backend-candidate-aggregate-v1'
    Decision = 'python-slm-backend-selection-decision-v1'
}
$script:P2NativeLoaded = $false
$script:P2ExpectedModuleHashes = @{}
$script:P2TranscriptRoleRoots = @{}

function Initialize-P2NativeInterop {
    if ($script:P2NativeLoaded) { return }
    $requiredTypes=@('P2Job','P2NvmlSampleResult','P2NvmlMonitor','P2CudaHealth')
    $loadedTypes=@($requiredTypes|Where-Object{$null-ne($_-as[type])})
    if($loadedTypes.Count-eq$requiredTypes.Count){$script:P2NativeLoaded=$true;return}
    if($loadedTypes.Count-ne0){throw 'P2 native interop type set is only partially loaded'}
    $source = @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;

public sealed class P2Job : IDisposable {
    private IntPtr handle;
    [StructLayout(LayoutKind.Sequential)] private struct IO_COUNTERS {
        public ulong ReadOperationCount, WriteOperationCount, OtherOperationCount;
        public ulong ReadTransferCount, WriteTransferCount, OtherTransferCount;
    }
    [StructLayout(LayoutKind.Sequential)] private struct BASIC_LIMIT {
        public long PerProcessUserTimeLimit, PerJobUserTimeLimit;
        public uint LimitFlags; public UIntPtr MinimumWorkingSetSize, MaximumWorkingSetSize;
        public uint ActiveProcessLimit; public UIntPtr Affinity; public uint PriorityClass, SchedulingClass;
    }
    [StructLayout(LayoutKind.Sequential)] private struct EXTENDED_LIMIT {
        public BASIC_LIMIT BasicLimitInformation; public IO_COUNTERS IoInfo;
        public UIntPtr ProcessMemoryLimit, JobMemoryLimit, PeakProcessMemoryUsed, PeakJobMemoryUsed;
    }
    [StructLayout(LayoutKind.Sequential)] private struct BASIC_ACCOUNTING {
        public long TotalUserTime, TotalKernelTime, ThisPeriodTotalUserTime, ThisPeriodTotalKernelTime;
        public uint TotalPageFaultCount, TotalProcesses, ActiveProcesses, TotalTerminatedProcesses;
    }
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode)] private static extern IntPtr CreateJobObject(IntPtr attr, string name);
    [DllImport("kernel32.dll")] private static extern bool SetInformationJobObject(IntPtr job, int cls, IntPtr info, uint len);
    [DllImport("kernel32.dll")] private static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
    [DllImport("kernel32.dll")] private static extern bool TerminateJobObject(IntPtr job, uint code);
    [DllImport("kernel32.dll")] private static extern bool QueryInformationJobObject(IntPtr job, int cls, IntPtr info, uint len, out uint returned);
    [DllImport("kernel32.dll")] private static extern bool CloseHandle(IntPtr handle);
    public P2Job() {
        handle = CreateJobObject(IntPtr.Zero, null);
        if (handle == IntPtr.Zero) throw new System.ComponentModel.Win32Exception();
        var limits = new EXTENDED_LIMIT(); limits.BasicLimitInformation.LimitFlags = 0x2000;
        int size = Marshal.SizeOf(typeof(EXTENDED_LIMIT)); IntPtr ptr = Marshal.AllocHGlobal(size);
        try { Marshal.StructureToPtr(limits, ptr, false); if (!SetInformationJobObject(handle, 9, ptr, (uint)size)) throw new System.ComponentModel.Win32Exception(); }
        finally { Marshal.FreeHGlobal(ptr); }
    }
    public void Assign(Process process) { if (!AssignProcessToJobObject(handle, process.Handle)) throw new System.ComponentModel.Win32Exception(); }
    public uint ActiveProcessCount() {
        int size = Marshal.SizeOf(typeof(BASIC_ACCOUNTING)); IntPtr ptr = Marshal.AllocHGlobal(size);
        try { uint returned; if (!QueryInformationJobObject(handle, 1, ptr, (uint)size, out returned)) throw new System.ComponentModel.Win32Exception();
            return ((BASIC_ACCOUNTING)Marshal.PtrToStructure(ptr, typeof(BASIC_ACCOUNTING))).ActiveProcesses; }
        finally { Marshal.FreeHGlobal(ptr); }
    }
    public void Terminate(uint code) { if (handle != IntPtr.Zero && !TerminateJobObject(handle, code)) throw new System.ComponentModel.Win32Exception(); }
    public void Dispose() { if (handle != IntPtr.Zero) { CloseHandle(handle); handle = IntPtr.Zero; } }
}

public sealed class P2NvmlSampleResult {
    public ulong TotalBytes, BaselineBytes, PeakUsedBytes; public long MaxGapMs;
    public int Samples, BaselineForeignProcessCount, MaximumForeignProcessCount; public string Error;
}

public sealed class P2NvmlMonitor : IDisposable {
    [StructLayout(LayoutKind.Sequential)] private struct Memory { public ulong total, free, used; }
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern IntPtr LoadLibraryEx(string path, IntPtr file, uint flags);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern uint GetModuleFileName(IntPtr module, System.Text.StringBuilder path, int size);
    [DllImport("nvml.dll", EntryPoint="nvmlInit_v2")] private static extern int Init();
    [DllImport("nvml.dll", EntryPoint="nvmlShutdown")] private static extern int Shutdown();
    [DllImport("nvml.dll", EntryPoint="nvmlDeviceGetHandleByIndex_v2")] private static extern int GetDevice(uint index, out IntPtr device);
    [DllImport("nvml.dll", EntryPoint="nvmlDeviceGetMemoryInfo")] private static extern int GetMemory(IntPtr device, out Memory memory);
    [StructLayout(LayoutKind.Sequential)] private struct ProcessInfo { public uint pid; public ulong usedGpuMemory; public uint gpuInstanceId, computeInstanceId; }
    [DllImport("nvml.dll", EntryPoint="nvmlDeviceGetComputeRunningProcesses_v3")] private static extern int GetProcesses(IntPtr device, ref uint count, [Out] ProcessInfo[] infos);
    [DllImport("nvml.dll", EntryPoint="nvmlDeviceGetGraphicsRunningProcesses_v3")] private static extern int GetGraphicsProcesses(IntPtr device, ref uint count, [Out] ProcessInfo[] infos);
    private IntPtr device; private Thread thread; private volatile bool stop; private readonly List<ulong> baseline = new List<ulong>();
    private readonly object sync = new object(); private ulong total, peak; private long maxGap; private int samples, baselineForeign, maxForeign; private uint expectedPid; private string error; private Stopwatch clock;
    private static readonly object librarySync=new object();private static IntPtr qualifiedLibrary=IntPtr.Zero;private static string qualifiedLibraryPath;
    public static string EnsureQualifiedLibrary(){lock(librarySync){if(qualifiedLibrary!=IntPtr.Zero)return qualifiedLibraryPath;
        string expected=System.IO.Path.GetFullPath(System.IO.Path.Combine(Environment.SystemDirectory,"nvml.dll"));
        qualifiedLibrary=LoadLibraryEx(expected,IntPtr.Zero,0x00000800);if(qualifiedLibrary==IntPtr.Zero)throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error(),"NVML_SYSTEM32_LOAD_FAILED");
        var actual=new System.Text.StringBuilder(32768);if(GetModuleFileName(qualifiedLibrary,actual,actual.Capacity)==0)throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error(),"NVML_MODULE_PATH_FAILED");
        qualifiedLibraryPath=System.IO.Path.GetFullPath(actual.ToString());if(!String.Equals(expected,qualifiedLibraryPath,StringComparison.OrdinalIgnoreCase))throw new InvalidOperationException("NVML_NOT_SYSTEM32");return qualifiedLibraryPath;}}
    private delegate int ProcessQuery(IntPtr device, ref uint count, ProcessInfo[] infos);
    private static void CollectProcesses(IntPtr handle, ProcessQuery query, HashSet<uint> pids) { uint count=0; int rc=query(handle,ref count,null); if(rc==0)return; if(rc!=7)throw new InvalidOperationException("NVML_PROCESSES_"+rc);
        var infos=new ProcessInfo[count];rc=query(handle,ref count,infos);if(rc!=0)throw new InvalidOperationException("NVML_PROCESSES_"+rc);for(int i=0;i<count;i++)pids.Add(infos[i].pid); }
    private static int ProcessCount(IntPtr handle, uint excludePid) { var pids=new HashSet<uint>(); CollectProcesses(handle,GetProcesses,pids);CollectProcesses(handle,GetGraphicsProcesses,pids);if(excludePid!=0)pids.Remove(excludePid);return pids.Count; }
    public static int ActiveGpuProcessCount(uint ordinal) { EnsureQualifiedLibrary();int rc=Init();if(rc!=0)throw new InvalidOperationException("NVML_INIT_"+rc);try{IntPtr h;rc=GetDevice(ordinal,out h);if(rc!=0)throw new InvalidOperationException("NVML_DEVICE_"+rc);return ProcessCount(h,0);}finally{Shutdown();} }
    public void SetExpectedPid(uint pid){ expectedPid=pid; }
    public void Start(uint ordinal) {
        EnsureQualifiedLibrary();int rc = Init(); if (rc != 0) throw new InvalidOperationException("NVML_INIT_" + rc);
        rc = GetDevice(ordinal, out device); if (rc != 0) { Shutdown(); throw new InvalidOperationException("NVML_DEVICE_" + rc); }
        clock=Stopwatch.StartNew();long previous=clock.ElapsedMilliseconds;
        for (int i=0; i<20; i++) { Memory m; rc=GetMemory(device,out m); if(rc!=0) throw new InvalidOperationException("NVML_MEMORY_"+rc); total=m.total; baseline.Add(m.used);
            long now=clock.ElapsedMilliseconds;if(i>0&&now-previous>maxGap)maxGap=now-previous;previous=now;
            int foreign=ProcessCount(device,0);if(foreign>baselineForeign)baselineForeign=foreign;if(i<19) Thread.Sleep(50); }
        maxForeign=baselineForeign; baseline.Sort(); stop=false;
        thread = new Thread(() => { try { while(!stop) { Memory m; int status=GetMemory(device,out m); if(status!=0){ error="NVML_MEMORY_"+status; break; }
                    int foreign=expectedPid==0?0:ProcessCount(device,expectedPid);long now=clock.ElapsedMilliseconds; long gap=now-previous; previous=now; if(gap>maxGap)maxGap=gap; lock(sync){ if(m.used>peak)peak=m.used;if(foreign>maxForeign)maxForeign=foreign;samples++; } Thread.Sleep(20); } }
                catch(Exception ex){ error=ex.GetType().Name; } });
        thread.IsBackground=true; thread.Start();
    }
    public P2NvmlSampleResult Stop() {
        stop=true; if(thread!=null && !thread.Join(5000)) error="MONITOR_THREAD_TIMEOUT"; if(clock!=null)clock.Stop(); Shutdown();
        ulong median=baseline.Count==0?0:(baseline.Count%2==0?(baseline[baseline.Count/2-1]+baseline[baseline.Count/2])/2:baseline[baseline.Count/2]);
        return new P2NvmlSampleResult { TotalBytes=total, BaselineBytes=median, PeakUsedBytes=peak,
            MaxGapMs=maxGap, Samples=samples, BaselineForeignProcessCount=baselineForeign, MaximumForeignProcessCount=maxForeign, Error=error };
    }
    public void Dispose(){ if(thread!=null && thread.IsAlive){stop=true;thread.Join(1000);} try{Shutdown();}catch{} }
}

public static class P2CudaHealth {
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern IntPtr LoadLibraryEx(string path, IntPtr file, uint flags);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern uint GetModuleFileName(IntPtr module, System.Text.StringBuilder path, int size);
    [DllImport("nvcuda.dll", EntryPoint="cuInit")] private static extern int Init(uint flags);
    [DllImport("nvcuda.dll", EntryPoint="cuDeviceGetCount")] private static extern int Count(out int count);
    [DllImport("nvcuda.dll", EntryPoint="cuDeviceGet")] private static extern int Device(out int device, int ordinal);
    [DllImport("nvcuda.dll", EntryPoint="cuDevicePrimaryCtxRetain")] private static extern int Retain(out IntPtr ctx, int device);
    [DllImport("nvcuda.dll", EntryPoint="cuCtxSetCurrent")] private static extern int SetCurrent(IntPtr ctx);
    [DllImport("nvcuda.dll", EntryPoint="cuMemAlloc_v2")] private static extern int Alloc(out ulong ptr, ulong bytes);
    [DllImport("nvcuda.dll", EntryPoint="cuMemsetD32_v2")] private static extern int Memset(ulong ptr, uint value, ulong count);
    [DllImport("nvcuda.dll", EntryPoint="cuMemcpyDtoH_v2")] private static extern int CopyToHost(out uint value, ulong ptr, ulong bytes);
    [DllImport("nvcuda.dll", EntryPoint="cuCtxSynchronize")] private static extern int Sync();
    [DllImport("nvcuda.dll", EntryPoint="cuMemFree_v2")] private static extern int Free(ulong ptr);
    [DllImport("nvcuda.dll", EntryPoint="cuDevicePrimaryCtxRelease_v2")] private static extern int Release(int device);
    private static readonly object librarySync=new object();private static IntPtr qualifiedLibrary=IntPtr.Zero;
    private static void EnsureQualifiedDriverLibrary(){lock(librarySync){if(qualifiedLibrary!=IntPtr.Zero)return;string expected=System.IO.Path.GetFullPath(System.IO.Path.Combine(Environment.SystemDirectory,"nvcuda.dll"));
        qualifiedLibrary=LoadLibraryEx(expected,IntPtr.Zero,0x00000800);if(qualifiedLibrary==IntPtr.Zero)throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error(),"NVCUDA_SYSTEM32_LOAD_FAILED");
        var actual=new System.Text.StringBuilder(32768);if(GetModuleFileName(qualifiedLibrary,actual,actual.Capacity)==0)throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error(),"NVCUDA_MODULE_PATH_FAILED");
        if(!String.Equals(expected,System.IO.Path.GetFullPath(actual.ToString()),StringComparison.OrdinalIgnoreCase))throw new InvalidOperationException("NVCUDA_NOT_SYSTEM32");}}
    public static string Probe() { EnsureQualifiedDriverLibrary();int rc=Init(0); if(rc!=0)return "CU_INIT_"+rc; int count;rc=Count(out count);if(rc!=0||count!=1)return "CU_COUNT_"+rc+"_"+count;
        int dev;rc=Device(out dev,0);if(rc!=0)return "CU_DEVICE_"+rc;IntPtr ctx;rc=Retain(out ctx,dev);if(rc!=0)return "CU_CONTEXT_"+rc;
        ulong ptr=0;string result="PASS";try{rc=SetCurrent(ctx);if(rc!=0)result="CU_CURRENT_"+rc;
            if(result=="PASS"){rc=Alloc(out ptr,4);if(rc!=0)result="CU_ALLOC_"+rc;}
            if(result=="PASS"){rc=Memset(ptr,0x5032484c,1);if(rc!=0)result="CU_MEMSET_"+rc;}
            if(result=="PASS"){rc=Sync();if(rc!=0)result="CU_SYNC_"+rc;}
            if(result=="PASS"){uint value;rc=CopyToHost(out value,ptr,4);if(rc!=0)result="CU_COPY_"+rc;else if(value!=0x5032484c)result="CU_SENTINEL_MISMATCH";}}
        finally{if(ptr!=0){int freeRc=Free(ptr);if(freeRc!=0)result="CU_FREE_"+freeRc;}int clearRc=SetCurrent(IntPtr.Zero);if(clearRc!=0)result="CU_CLEAR_CURRENT_"+clearRc;
            int releaseRc=Release(dev);if(releaseRc!=0)result="CU_RELEASE_"+releaseRc;}return result; }
}
'@
    Add-Type -TypeDefinition $source -Language CSharp -ErrorAction Stop
    $script:P2NativeLoaded = $true
}

function Test-P2PathWithin {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][string]$Root)
    try {
        $fullPath = [IO.Path]::GetFullPath($Path).TrimEnd('\', '/')
        $fullRoot = [IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    }
    catch { return $false }
    return [string]::Equals($fullPath, $fullRoot, [StringComparison]::OrdinalIgnoreCase) -or
        $fullPath.StartsWith($fullRoot + [IO.Path]::DirectorySeparatorChar,
            [StringComparison]::OrdinalIgnoreCase)
}

function New-P2NvmlLibraryRecord {
    param([Parameter(Mandatory)][string]$Path,[Parameter(Mandatory)][string]$WindowsRoot)
    $expected=[IO.Path]::GetFullPath((Join-Path $WindowsRoot 'System32\nvml.dll'))
    $actual=[IO.Path]::GetFullPath($Path)
    if(-not[string]::Equals($actual,$expected,[StringComparison]::OrdinalIgnoreCase)-or-not(Test-Path -LiteralPath $actual -PathType Leaf)){
        throw 'NVML library is not the canonical System32 driver-boundary module'
    }
    return [ordered]@{path='${WINDOWS}/System32/nvml.dll';sha256=Get-P2Sha256 $actual;provenance='p2_live_system32_driver_boundary'}
}

function Resolve-P2OutputRoot {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$RepositoryRoot, [Parameter(Mandatory)][string]$OutputRoot)
    $repository = [IO.Path]::GetFullPath($RepositoryRoot).TrimEnd('\', '/')
    $candidate = if ([IO.Path]::IsPathRooted($OutputRoot)) {
        [IO.Path]::GetFullPath($OutputRoot)
    }
    else { [IO.Path]::GetFullPath((Join-Path $repository $OutputRoot)) }
    $allowed = [IO.Path]::GetFullPath((Join-Path $repository 'docs\receipts\P2'))
    if (-not [string]::Equals($candidate.TrimEnd('\', '/'), $allowed.TrimEnd('\', '/'),
            [StringComparison]::OrdinalIgnoreCase)) {
        throw 'P2 output root must resolve exactly to docs/receipts/P2'
    }
    if (-not (Test-P2PathWithin -Path $candidate -Root $repository) -or
        (Test-Path -LiteralPath $candidate -PathType Leaf)) {
        throw 'P2 output root is not a contained directory'
    }
    $cursor = $candidate
    while (Test-P2PathWithin -Path $cursor -Root $repository) {
        if (Test-Path -LiteralPath $cursor) {
            if (((Get-Item -Force -LiteralPath $cursor).Attributes -band
                    [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw 'P2 output root resolves through a reparse point'
            }
        }
        if ([string]::Equals($cursor.TrimEnd('\', '/'), $repository,
                [StringComparison]::OrdinalIgnoreCase)) { break }
        $cursor = Split-Path -Parent $cursor
    }
    return $candidate
}

function Assert-P2OwnedDirectory {
    param([Parameter(Mandatory)][string]$Path,[Parameter(Mandatory)][string]$Root)
    if(-not(Test-P2PathWithin -Path $Path -Root $Root)){throw 'P2 owned directory escaped its root'}
    $cursor=[IO.Path]::GetFullPath($Path)
    while(Test-P2PathWithin -Path $cursor -Root $Root){if(Test-Path -LiteralPath $cursor){$item=Get-Item -Force -LiteralPath $cursor
            if(-not$item.PSIsContainer-or(($item.Attributes-band[IO.FileAttributes]::ReparsePoint)-ne0)){throw 'P2 owned directory is not a plain directory'}}
        if([string]::Equals($cursor,[IO.Path]::GetFullPath($Root),[StringComparison]::OrdinalIgnoreCase)){break};$cursor=Split-Path -Parent $cursor}
}

function Write-P2Utf8LfFile {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Path,
        [AllowEmptyString()][string]$Text,
        [switch]$CreateNew
    )
    $value = if ($null -eq $Text) { '' } else { $Text }
    $value = $value.Replace("`r`n", "`n").Replace("`r", "`n")
    if ($value.Length -gt 0 -and -not $value.EndsWith("`n")) { $value += "`n" }
    $mode = if ($CreateNew) { [IO.FileMode]::CreateNew } else { [IO.FileMode]::Create }
    $stream = [IO.FileStream]::new($Path, $mode, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $bytes = $script:P2Utf8NoBom.GetBytes($value)
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
    }
    finally { $stream.Dispose() }
}

function Write-P2JsonFile {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)]$Value, [switch]$CreateNew)
    Write-P2Utf8LfFile -Path $Path -Text ($Value | ConvertTo-Json -Depth 64) -CreateNew:$CreateNew
}

function Get-P2Sha256 {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Path)
    return (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
}

function Get-P2TextSha256 {
    param([AllowEmptyString()][string]$Text)
    $hash = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($hash.ComputeHash(
                    $script:P2Utf8NoBom.GetBytes([string]$Text)))).Replace('-', '').ToLowerInvariant()
    }
    finally { $hash.Dispose() }
}

function New-P2RunId {
    return [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ') + '-' +
        ([Guid]::NewGuid().ToString('N').Substring(0, 24))
}

function Protect-P2Text {
    [CmdletBinding()]
    param(
        [AllowEmptyString()][string]$Text,
        [Parameter(Mandatory)][string]$RepositoryRoot,
        [string[]]$ExtraRoots = @(),
        [hashtable]$RoleRoots = @{}
    )
    $safe = [string]$Text
    $replacements = [Collections.Generic.List[object]]::new()
    $replacements.Add([pscustomobject]@{ Path = [IO.Path]::GetFullPath($RepositoryRoot); Token = '${REPO}' })
    foreach ($key in $RoleRoots.Keys) {
        if (-not [string]::IsNullOrWhiteSpace([string]$RoleRoots[$key])) {
            $replacements.Add([pscustomobject]@{
                    Path = [IO.Path]::GetFullPath([string]$RoleRoots[$key]); Token = '${' + $key + '}'
                })
        }
    }
    $tokens = @('USERPROFILE', 'WINDOWS', 'TEMP', 'TEMP')
    for ($i = 0; $i -lt $ExtraRoots.Count; $i++) {
        if (-not [string]::IsNullOrWhiteSpace($ExtraRoots[$i])) {
            try {
                $replacements.Add([pscustomobject]@{
                        Path = [IO.Path]::GetFullPath($ExtraRoots[$i]); Token = '${' + $tokens[[Math]::Min($i, 3)] + '}'
                    })
            }
            catch { }
        }
    }
    foreach ($entry in @($replacements | Sort-Object { $_.Path.Length } -Descending)) {
        $variants = @($entry.Path, $entry.Path.Replace('\', '/')) | Select-Object -Unique
        foreach ($variant in $variants) {
            $safe = [regex]::Replace($safe, [regex]::Escape($variant), $entry.Token,
                [Text.RegularExpressions.RegexOptions]::IgnoreCase)
        }
    }
    $safe = [regex]::Replace($safe, '(?i)(authorization|token|password|secret|api[_-]?key)\s*[:=]\s*[^\s,;]+', '$1=<redacted>')
    $safe = [regex]::Replace($safe, '(?i)(https?://)[^/@\s:]+:[^/@\s]+@', '$1<redacted>@')
    return $safe.Replace('\', '/')
}

function ConvertTo-P2CommandLine {
    param([Parameter(Mandatory)][string[]]$Arguments)
    $rendered = foreach ($argument in $Arguments) {
        if ($argument.Length -gt 0 -and $argument -notmatch '[\s"]') { $argument; continue }
        $builder = [Text.StringBuilder]::new(); [void]$builder.Append('"'); $slashes = 0
        foreach ($character in $argument.ToCharArray()) {
            if ($character -eq '\') { $slashes++; continue }
            if ($character -eq '"') {
                [void]$builder.Append(('\' * (($slashes * 2) + 1))); [void]$builder.Append('"'); $slashes = 0; continue
            }
            if ($slashes -gt 0) { [void]$builder.Append(('\' * $slashes)); $slashes = 0 }
            [void]$builder.Append($character)
        }
        if ($slashes -gt 0) { [void]$builder.Append(('\' * ($slashes * 2))) }
        [void]$builder.Append('"'); $builder.ToString()
    }
    return ($rendered -join ' ')
}

function Invoke-P2Process {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [string[]]$ArgumentList = @(),
        [Parameter(Mandatory)][string]$WorkingDirectory,
        [hashtable]$Environment = @{},
        [int]$TimeoutSeconds = 1800,
        [switch]$MonitorNvml
    )
    Initialize-P2NativeInterop
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $FilePath; $start.Arguments = ConvertTo-P2CommandLine -Arguments $ArgumentList
    $start.WorkingDirectory = $WorkingDirectory; $start.UseShellExecute = $false
    $start.CreateNoWindow = $true; $start.RedirectStandardOutput = $true; $start.RedirectStandardError = $true
    $canonical = [Collections.Generic.Dictionary[string,string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($entry in [Environment]::GetEnvironmentVariables().GetEnumerator()) {
        $name = [string]$entry.Key; $value = [string]$entry.Value
        if ($canonical.ContainsKey($name) -and $canonical[$name] -cne $value) {
            throw "parent environment contains conflicting case variants for $name"
        }
        $canonical[$name] = $value
    }
    foreach ($name in $Environment.Keys) {
        if ([string]::IsNullOrWhiteSpace([string]$name) -or [string]$name -match '[=\x00]') {
            throw 'invalid child environment variable name'
        }
        if ($null -eq $Environment[$name]) { [void]$canonical.Remove([string]$name) }
        else {
            $value = [string]$Environment[$name]; if ($value.IndexOf([char]0) -ge 0) { throw 'child environment contains NUL' }
            $canonical[[string]$name] = $value
        }
    }
    $childEnvironment = [Collections.Specialized.StringDictionary]::new()
    foreach ($entry in $canonical.GetEnumerator()) { $childEnvironment[$entry.Key] = $entry.Value }
    $environmentField = $start.GetType().GetField('environmentVariables', [Reflection.BindingFlags]'Instance,NonPublic')
    if ($null -eq $environmentField -or $environmentField.FieldType -ne [Collections.Specialized.StringDictionary]) {
        throw 'ProcessStartInfo child-environment backing field is unsupported'
    }
    $environmentField.SetValue($start, $childEnvironment)
    $process = [Diagnostics.Process]::new(); $process.StartInfo = $start
    $job = [P2Job]::new(); $monitor = $null; $monitorResult = $null
    $stopwatch = [Diagnostics.Stopwatch]::StartNew(); $timedOut = $false; $treeTerminated = $true
    $unexpectedDescendants = $false
    $modules = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    try {
        if ($MonitorNvml) { $monitor = [P2NvmlMonitor]::new(); $monitor.Start(0) }
        if (-not $process.Start()) { throw 'child process did not start' }
        $job.Assign($process); if ($null -ne $monitor) { $monitor.SetExpectedPid([uint32]$process.Id) }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync(); $stderrTask = $process.StandardError.ReadToEndAsync()
        $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
        while (-not $process.WaitForExit(50)) {
            if ([DateTime]::UtcNow -ge $deadline) {
                $timedOut = $true
                try { $job.Terminate(124) } catch { $treeTerminated = $false }
                if (-not $process.WaitForExit(10000)) { $treeTerminated = $false }
                break
            }
            try { foreach ($module in $process.Modules) { [void]$modules.Add($module.FileName) } } catch { }
        }
        if (-not $timedOut) {
            $process.WaitForExit()
            if ([uint32]$job.ActiveProcessCount() -gt 0) {
                $unexpectedDescendants = $true
                try { $job.Terminate(125) } catch { $treeTerminated = $false }
            }
        }
        try { foreach ($module in $process.Modules) { [void]$modules.Add($module.FileName) } } catch { }
        [void]$stdoutTask.Wait(10000); [void]$stderrTask.Wait(10000)
        return [pscustomobject][ordered]@{
            exit_code = if ($timedOut) { $null } else { [int]$process.ExitCode }
            duration_ms = [int64]$stopwatch.ElapsedMilliseconds
            stdout = if ($stdoutTask.IsCompleted) { [string]$stdoutTask.Result } else { '' }
            stderr = if ($stderrTask.IsCompleted) { [string]$stderrTask.Result } else { 'transcript capture did not complete' }
            timed_out = $timedOut; process_tree_terminated = $treeTerminated
            unexpected_descendants = $unexpectedDescendants
            loaded_modules = @($modules | Sort-Object)
            nvml = $null
        }
    }
    finally {
        if ($null -ne $monitor) {
            try { $monitorResult = $monitor.Stop() } finally { $monitor.Dispose() }
        }
        $stopwatch.Stop(); $job.Dispose(); $process.Dispose()
        if ($null -ne $monitorResult) {
            # Return objects are immutable PSCustomObjects in callers, so publish monitor
            # data through a script slot consumed immediately by Invoke-P2RecordedCommand.
            $script:P2LastNvml = $monitorResult
        }
        else { $script:P2LastNvml = $null }
    }
}

function Invoke-P2RecordedCommand {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Id,
        [Parameter(Mandatory)][string]$FilePath,
        [string[]]$ArgumentList = @(),
        [Parameter(Mandatory)][string]$WorkingDirectory,
        [Parameter(Mandatory)][string]$RunRoot,
        [Parameter(Mandatory)][string]$RepositoryRoot,
        [hashtable]$Environment = @{},
        [int[]]$ExpectedExitCodes = @(0),
        [int]$TimeoutSeconds = 1800,
        [switch]$MonitorNvml,
        [hashtable]$RoleRoots = @{}
    )
    $mergedRoleRoots=@{};foreach($entry in $script:P2TranscriptRoleRoots.GetEnumerator()){$mergedRoleRoots[$entry.Key]=$entry.Value};foreach($entry in $RoleRoots.GetEnumerator()){$mergedRoleRoots[$entry.Key]=$entry.Value}
    $RoleRoots=$mergedRoleRoots
    $result = Invoke-P2Process -FilePath $FilePath -ArgumentList $ArgumentList `
        -WorkingDirectory $WorkingDirectory -Environment $Environment -TimeoutSeconds $TimeoutSeconds `
        -MonitorNvml:$MonitorNvml
    if (-not [bool]$result.process_tree_terminated -or [bool]$result.unexpected_descendants) { $script:P2TreeViolation = $true }
    $roots = @($env:USERPROFILE, $env:SystemRoot, $env:TEMP, $env:TMP) + @($RoleRoots.Values)
    $stdout = Protect-P2Text -Text $result.stdout -RepositoryRoot $RepositoryRoot -ExtraRoots $roots -RoleRoots $RoleRoots
    $stderr = Protect-P2Text -Text $result.stderr -RepositoryRoot $RepositoryRoot -ExtraRoots $roots -RoleRoots $RoleRoots
    $commandsRoot = Join-Path $RunRoot 'commands'; [void][IO.Directory]::CreateDirectory($commandsRoot)
    $stdoutPath = Join-Path $commandsRoot "$Id.stdout.txt"; $stderrPath = Join-Path $commandsRoot "$Id.stderr.txt"
    Write-P2Utf8LfFile -Path $stdoutPath -Text $stdout -CreateNew
    Write-P2Utf8LfFile -Path $stderrPath -Text $stderr -CreateNew
    $allowed = -not $result.timed_out -and $ExpectedExitCodes -contains [int]$result.exit_code
    $status = if ($result.timed_out) { 'TIMEOUT' }
    elseif ([int]$result.exit_code -eq 0) { 'PASS' }
    elseif (($stdout + "`n" + $stderr) -match '(?i)(out of memory|cuda_error_out_of_memory|allocation failed)') { 'OOM' }
    elseif ([int]$result.exit_code -lt 0) { 'CRASH' }
    else { 'FAIL' }
    $safeArgv = @((Split-Path -Leaf $FilePath)) + @($ArgumentList) | ForEach-Object {
        Protect-P2Text -Text ([string]$_) -RepositoryRoot $RepositoryRoot -ExtraRoots $roots -RoleRoots $RoleRoots
    }
    $environmentDeltaNames=@($Environment.Keys|ForEach-Object{[string]$_}|Sort-Object -Unique)
    $environmentPolicy='^(?:CARGO(?:_|$)|RUST(?:C|DOC|FLAGS|UP|_).*$|CUDA(?:_|$)|CUDNN(?:_|$)|NVCC(?:_|$)|PYTHON(?:_|$)|PIP(?:_|$)|VIRTUAL_ENV$|CONDA_PREFIX$|PATH$|TEMP$|TMP$|USERPROFILE$|HOME$|LIB$|INCLUDE$|LIBPATH$|WindowsSdkDir$|WindowsSDKVersion$|VCToolsInstallDir$|VisualStudioVersion$)'
    $effectiveBuildEnvironment=[ordered]@{}
    foreach($name in @($Environment.Keys|Where-Object{[string]$_-match$environmentPolicy}|Sort-Object)){
        $value=$Environment[$name]
        $effectiveBuildEnvironment[[string]$name]=if($null-eq$value){'<CLEARED>'}
        elseif([string]$name-match'(?i)(token|secret|password|credential|api[_-]?key)'){'<PRESENT>'}
        else{Protect-P2Text -Text ([string]$value) -RepositoryRoot $RepositoryRoot -ExtraRoots $roots -RoleRoots $RoleRoots}
    }
    $safeCwd=Protect-P2Text -Text $WorkingDirectory -RepositoryRoot $RepositoryRoot -ExtraRoots $roots -RoleRoots $RoleRoots
    $configuration=[ordered]@{argv=@($safeArgv);cwd=$safeCwd;environment_delta_names=$environmentDeltaNames;effective_build_environment=$effectiveBuildEnvironment}
    $configurationPath=Join-Path $commandsRoot "$Id.configuration.json"
    Write-P2JsonFile -Path $configurationPath -Value $configuration -CreateNew
    $record = [pscustomobject][ordered]@{
        id = $Id; argv = @($safeArgv)
        cwd = $safeCwd;environment_delta_names=$environmentDeltaNames
        configuration_sha256=Get-P2Sha256 -Path $configurationPath
        expected_exit_codes = @($ExpectedExitCodes); expectation_met = $allowed
        exit_code = $result.exit_code; duration_ms = [int64]$result.duration_ms; status = $status
        stdout = New-P2FileReference -Path $stdoutPath -RunRoot $RunRoot
        stderr = New-P2FileReference -Path $stderrPath -RunRoot $RunRoot
    }
    return [pscustomobject][ordered]@{
        record = $record; raw_stdout = [string]$result.stdout; raw_stderr = [string]$result.stderr
        loaded_modules = @($result.loaded_modules); process_tree_terminated = [bool]$result.process_tree_terminated
        unexpected_descendants = [bool]$result.unexpected_descendants
        nvml = $script:P2LastNvml
    }
}

function Get-P2RepositoryIdentity {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$RepositoryRoot)
    $git = (Get-Command git.exe -ErrorAction Stop).Source
    $head = (& $git -c 'core.excludesFile=' -c 'core.quotePath=false' -C $RepositoryRoot rev-parse HEAD 2>$null).Trim()
    $tree = (& $git -c 'core.excludesFile=' -c 'core.quotePath=false' -C $RepositoryRoot rev-parse 'HEAD^{tree}' 2>$null).Trim()
    if ($LASTEXITCODE -ne 0 -or $head -cnotmatch '^[0-9a-f]{40}$' -or $tree -cnotmatch '^[0-9a-f]{40}$') {
        throw 'could not resolve P2 source commit and tree'
    }
    $status = @(& $git -c 'core.excludesFile=' -c 'core.quotePath=false' -C $RepositoryRoot status --porcelain=v1 --untracked-files=all -- . `
        ':(exclude)docs/receipts/P2' `
        ':(exclude)docs/receipts/P1A/runs/20260811T181143258Z-6b3bf0141a3b4964bacec231' 2>$null)
    $normalizedStatus = ($status | Sort-Object) -join "`n"
    # Request one path per line for deterministic hashing.
    $tracked=@(& $git -c 'core.excludesFile=' -c 'core.quotePath=false' -C $RepositoryRoot ls-files --full-name 2>$null|Where-Object{
            $_-notmatch '^docs/receipts/P2(?:/|$)'-and
            $_-cne'docs/receipts/P1A/runs/20260811T181143258Z-6b3bf0141a3b4964bacec231'
        }|Sort-Object)
    $inputManifest=[Collections.Generic.List[object]]::new()
    foreach($relative in $tracked){
        $path=[IO.Path]::GetFullPath((Join-Path $RepositoryRoot $relative))
        if(-not(Test-P2PathWithin -Path $path -Root $RepositoryRoot)-or-not(Test-Path -LiteralPath $path -PathType Leaf)){throw 'tracked P2 input is missing or escapes the repository'}
        $item=Get-Item -LiteralPath $path
        $inputManifest.Add([ordered]@{path=([string]$relative).Replace('\','/');sha256=Get-P2Sha256 $path;bytes=[int64]$item.Length})
    }
    $manifestHash=Get-P2TextSha256 -Text ($inputManifest|ConvertTo-Json -Depth 5 -Compress)
    $source = [ordered]@{
        schema = 'python-slm-p2-source-identity-v1'; git_head = $head; git_tree = $tree
        excluded_paths = @(
            'docs/receipts/P2',
            'docs/receipts/P1A/runs/20260811T181143258Z-6b3bf0141a3b4964bacec231'
        )
        worktree_status_sha256 = Get-P2TextSha256 -Text $normalizedStatus
        worktree_status_entries = @($status).Count
        input_manifest_sha256=$manifestHash;input_manifest=@($inputManifest)
    }
    return [pscustomobject][ordered]@{
        value = $source; fingerprint = Get-P2TextSha256 -Text ($source | ConvertTo-Json -Depth 8 -Compress)
    }
}

function New-P2GateSet {
    $set = [ordered]@{}
    foreach ($name in @('p1b_dependency', 'input_stability', 'dependency_policy',
            'fixture_determinism', 'cpu_isolation', 'sm120_launch', 'bf16_roundtrip',
            'correctness', 'synchronization', 'runtime_provenance', 'measurement_validity',
            'redaction', 'cleanup')) {
        $set[$name] = [ordered]@{ status = 'NOT_RUN'; detail = 'Gate has not run.' }
    }
    return $set
}

function Set-P2Gate {
    param([Parameter(Mandatory)]$Gates, [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][ValidateSet('PASS', 'FAIL', 'NOT_RUN')][string]$Status,
        [Parameter(Mandatory)][string]$Detail)
    $Gates[$Name] = [ordered]@{ status = $Status; detail = $Detail }
}

function Remove-P2OwnedTemporaryRoot {
    param([Parameter(Mandatory)][string]$Path)
    $full = [IO.Path]::GetFullPath($Path); $base = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\', '/')
    if (-not (Test-P2PathWithin -Path $full -Root $base) -or
        (Split-Path -Leaf $full) -cnotmatch '^python-slm-p2-[0-9]{8}T[0-9]{9}Z-[0-9a-f]{24}$') {
        throw 'refusing unsafe P2 temporary cleanup target'
    }
    if (Test-Path -LiteralPath $full) { Remove-Item -LiteralPath $full -Recurse -Force -ErrorAction Stop }
}

function Test-P2ReceiptRedaction {
    param([Parameter(Mandatory)][string]$RunRoot)
    $forbidden = '(?i)(?:[A-Za-z]:[\\/]|\\\\|(?:authorization|password|secret|api[_-]?key|token)\s*[:=]\s*(?!<redacted>)[^\s,;]+|https?://[^/@\s:]+:[^/@\s]+@)'
    $identityPatterns=@($env:USERNAME,$env:COMPUTERNAME)|Where-Object{-not[string]::IsNullOrWhiteSpace($_)}|ForEach-Object{[regex]::Escape([string]$_)}
    foreach ($file in Get-ChildItem -LiteralPath $RunRoot -Recurse -File | Where-Object {
            $_.Extension -in @('.json', '.txt')
        }) {
        $text = [IO.File]::ReadAllText($file.FullName, $script:P2Utf8NoBom)
        if ($text -match $forbidden -or @($identityPatterns|Where-Object{$text-match$_}).Count-gt0) { throw "retained P2 evidence contains an unredacted value in $($file.Name)" }
        $bytes = [IO.File]::ReadAllBytes($file.FullName)
        if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xef -and $bytes[1] -eq 0xbb -and $bytes[2] -eq 0xbf) {
            throw "retained P2 evidence contains a UTF-8 BOM in $($file.Name)"
        }
        if ($text.Contains("`r")) { throw "retained P2 evidence contains CR line endings in $($file.Name)" }
    }
    return $true
}

function Get-P2EnvironmentFingerprint {
    $records=@([Environment]::GetEnvironmentVariables().GetEnumerator()|Sort-Object Key|ForEach-Object{[ordered]@{name=[string]$_.Key;value=[string]$_.Value}})
    return Get-P2TextSha256 -Text ($records|ConvertTo-Json -Depth 4 -Compress)
}

function New-P2Policy {
    return [ordered]@{
        schema = $script:P2Schema.Policy; phase_id = 'P2'; policy_id = 'backend-qualification-v1'
        frozen_before_candidate_execution = $true
        candidates = @(
            [ordered]@{ candidate_id = 'burn-cubecl'; crate = 'burn'; version = '0.21.0'; role = 'framework' },
            [ordered]@{ candidate_id = 'candle'; crate = 'candle-core'; version = '0.11.0'; role = 'framework' },
            [ordered]@{ candidate_id = 'cudarc-fallback'; crate = 'cudarc'; version = '0.19.8'; role = 'diagnostic_fallback' }
        )
        fixture_generation = [ordered]@{
            seed = 'domain-separated-sha256'; expander = 'splitmix64'
            byte_mapping = 'signed_high_byte_divided_by_128_f32'; source_dtype = 'f32'; target_dtype = 'bf16'
            rounding = 'round-to-nearest-even'; byte_order = 'little-endian'; framework_rng_allowed = $false
        }
        workloads = [ordered]@{
            allocation_shape = @(16, 2048, 768); correctness = [ordered]@{ m = 17; k = 31; n = 29 }
            projection = [ordered]@{ m = 8192; k = 768; n = 768 }
            ffn_expansion = [ordered]@{ m = 8192; k = 768; n = 2432 }
        }
        correctness_graph = [ordered]@{
            matmul = 'Y=A*B'; loss = 'mean(cast_f32(Y)^2)'; backward = 'gradients_for_A_and_B'
            input_dtype = 'bf16'; output_dtype = 'bf16'; accumulation_dtype = 'fp32'; loss_dtype = 'fp32'
        }
        tolerances = [ordered]@{
            allocation_bitwise = $true
            forward_envelope = [ordered]@{ absolute_floor = 0.0078125; reference_multiplier = 0.015625 }
            forward_relative_l2_max = 0.015; loss_absolute_floor = 0.00001; loss_reference_multiplier = 0.01
            gradient_envelope = [ordered]@{ absolute_floor = 0.0078125; reference_multiplier = 0.03125 }
            gradient_relative_l2_max = 0.03; gradient_cosine_min = 0.999; nan_max = 0
            infinite_max = 0; envelope_violations_max = 0; relative_l2_denominator_floor = 1e-12
            reported_relative_error_denominator_floor = 0.000001; signed_zero_equal = $true
        }
        timing = [ordered]@{
            round_order = @('burn-cubecl', 'candle', 'candle', 'burn-cubecl'); fresh_process_each_round = $true
            round_order_scope = 'per_workload'; workload_order = @('projection','ffn-expansion')
            warmups_per_timed_operation = 50; minimum_samples = 200; minimum_window_seconds = 5
            maximum_samples = 10000; maximum_window_seconds = 60; synchronize_before_and_after = $true
            percentile_method = 'nearest-rank-integer-nanoseconds'; forward_flop_formula = '2*M*N*K/time'
            forward_backward_flop_formula = '6*M*N*K/time'; tokens_per_second_forbidden = $true
            eight_hour_projection_forbidden = $true; maximum_between_round_p50_drift_ratio = 0.1
        }
        memory = [ordered]@{
            monitor = 'native-nvml-zero-python'; sample_interval_ms = 20; baseline_samples = 20
            baseline_interval_ms = 50; baseline_statistic = 'median'; maximum_sample_gap_ms = 100
            foreign_gpu_work_allowed = $false
            cuda_memory_points = @('after_context', 'after_allocation', 'after_forward', 'after_backward')
        }
        selection = [ordered]@{
            hard_gates_first = $true; performance_advantage_ratio = 0.05
            ordered_tiebreakers = @('geometric_mean_forward_backward_p50',
                'geometric_mean_forward_backward_p95', 'observed_peak_vram',
                'locked_dependency_count', 'stable_candidate_id')
            stable_candidate_id_final_tie = $true
        }
        fallback = [ordered]@{
            candidate_id = 'cudarc-fallback'; run_only_if_frameworks_fail = $true
            diagnostic_only = $true; may_satisfy_p2 = $false
        }
    }
}

function Test-P2DependencyPolicy {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$ManifestPath, [Parameter(Mandatory)][string]$LockPath)
    $manifestRoot = Split-Path -Parent $ManifestPath
    $manifestFiles = @(Get-ChildItem -LiteralPath $manifestRoot -Recurse -File -Filter Cargo.toml | Sort-Object FullName)
    foreach ($manifestFile in $manifestFiles) {
        $manifestText = [IO.File]::ReadAllText($manifestFile.FullName, $script:P2Utf8NoBom)
        if ($manifestText -match '(?im)^\s*\[(?:patch\.|replace\])' -or $manifestText -match '(?i)\bgit\s*=' -or
            $manifestText -match '(?i)\bregistry\s*=' -or $manifestText -match '(?i)https?://[^\s"'']+@') {
            throw 'experiment manifest contains a prohibited dependency source'
        }
        $dependencySection = $false
        foreach ($line in ($manifestText -split '\r?\n')) {
            if ($line -match '^\s*\[(?<section>[^]]+)\]\s*(?:#.*)?$') {
                $dependencySection = $Matches.section -match '(?:^|\.)(?:dependencies|dev-dependencies|build-dependencies)$'
                continue
            }
            if (-not $dependencySection -or $line -match '^\s*(?:#|$)') { continue }
            if ($line -notmatch '^\s*(?<name>[A-Za-z0-9_.-]+)\s*=\s*(?<value>.+?)\s*(?:#.*)?$') {
                throw 'experiment dependency declaration is not statically auditable'
            }
            $dependencyName = [string]$Matches.name; $value = [string]$Matches.value
            if ($value -match '^"(?<version>[^"]+)"$') {
                if ($Matches.version -notmatch '^=[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$') { throw 'experiment direct registry dependency version is not exact' }
            }
            elseif ($value -match '^\{.*\}$') {
                $inline = $value.Substring(1, $value.Length - 2)
                $version = [regex]::Match($inline, '(?:^|,)\s*version\s*=\s*"(?<version>[^"]+)"')
                if ($version.Success -and $version.Groups['version'].Value -notmatch '^=[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$') {
                    throw 'experiment direct registry dependency version is not exact'
                }
                if (-not $version.Success -and $inline -notmatch '(?:^|,)\s*(?:path|workspace)\s*=') {
                    throw 'experiment direct dependency has neither an exact version nor a local/workspace binding'
                }
            }
            elseif ($value -ceq 'true' -and $dependencyName -cmatch '\.workspace$') {
                # Cargo's dotted-key shorthand `name.workspace = true` is
                # semantically identical to `name = { workspace = true }`.
                # The exact registry version remains frozen in the root
                # [workspace.dependencies] declaration audited above.
            }
            else { throw 'experiment dependency declaration is not a supported exact form' }
        }
    }
    $manifest = [IO.File]::ReadAllText($ManifestPath, $script:P2Utf8NoBom)
    foreach ($pin in @('burn.+version\s*=\s*"=0\.21\.0"',
            'candle-core.+version\s*=\s*"=0\.11\.0"', 'cudarc.+version\s*=\s*"=0\.19\.8"',
            'half.+version\s*=\s*"=2\.7\.1"')) {
        if ($manifest -notmatch $pin) { throw 'experiment direct dependency pin is not exact' }
    }
    if (-not (Test-Path -LiteralPath $LockPath -PathType Leaf)) { throw 'experiment Cargo.lock is missing' }
    $lock = [IO.File]::ReadAllText($LockPath, $script:P2Utf8NoBom)
    if ($lock -match '(?im)^source\s*=\s*"git\+' -or $lock -match '(?im)^source\s*=\s*"registry\+(?!https://github\.com/rust-lang/crates\.io-index)') {
        throw 'experiment lock uses a prohibited source'
    }
    $registryPackages = [regex]::Matches($lock, '(?ms)\[\[package\]\].*?(?=\r?\n\[\[package\]\]|\z)') |
        Where-Object { $_.Value -match '(?m)^source\s*=\s*"registry\+' }
    foreach ($package in $registryPackages) {
        if ($package.Value -notmatch '(?m)^checksum\s*=\s*"[0-9a-f]{64}"') {
            throw 'experiment lock contains a registry package without checksum'
        }
    }
    return [pscustomobject][ordered]@{
        status = 'PASS'; cargo_lock_sha256 = Get-P2Sha256 -Path $LockPath
        locked_dependency_count = @([regex]::Matches($lock, '(?m)^\[\[package\]\]$')).Count
        manifest_count = $manifestFiles.Count; all_direct_registry_versions_exact = $true
        hydration = [ordered]@{ bootstrap_acquisition = 'cargo fetch --target x86_64-pc-windows-msvc'; bootstrap_created_lock = $true
            prequalification_locked_cache_validation = 'cargo fetch --locked --target x86_64-pc-windows-msvc'
            qualification_commands_locked_offline = $true; qualification_network_allowed = $false }
        git_dependencies = 0; alternate_registries = 0; patches = 0
    }
}

function Test-P2CpuIsolationEvidence {
    [CmdletBinding()]
    param([Parameter(Mandatory)][object[]]$CommandResults, [Parameter(Mandatory)][string[]]$TargetFiles)
    $text = (($CommandResults | ForEach-Object { $_.raw_stdout + "`n" + $_.raw_stderr }) -join "`n")
    $forbidden = '(?i)(P2_CANARY_HIT|(?:^|[\s"''])[A-Za-z0-9_.\\/-]*(?:nvcc|python(?:3|w)?|pip(?:3)?)(?:\.exe)?(?:[\s"'']|$)|(?:^|[\s"''])[A-Za-z0-9_.\\/-]*(?:lib)?python(?:3(?:\d+)?)?\.(?:lib|dll)(?:[\s"'']|$)|(?:link\.exe|rustc\.exe).*(?:(?:cudnn|cublas|cudart|nvcuda|cuda)\.(?:lib|dll)|(?:lib)?python(?:3(?:\d+)?)?\.(?:lib|dll)))'
    $hits = [Collections.Generic.List[string]]::new()
    foreach ($match in [regex]::Matches($text, $forbidden)) { if (-not $hits.Contains($match.Value)) { $hits.Add($match.Value) } }
    foreach ($path in $TargetFiles) {
        if ($path -match '(?i)\.(cu|ptx|cubin|fatbin)$') { $hits.Add((Split-Path -Leaf $path)) }
    }
    return [pscustomobject][ordered]@{
        schema = 'python-slm-p2-cpu-isolation-v1'; status = if ($hits.Count -eq 0) { 'PASS' } else { 'FAIL' }
        forbidden_hits = @($hits | Sort-Object -Unique); cuda_or_python_discovered = $hits.Count -gt 0
    }
}

function Test-P2PythonTranscriptViolation {
    param([AllowEmptyString()][string]$Text)
    return [bool]([string]$Text-match'(?i)(P2_CANARY_HIT|(?:^|[\s"''])[A-Za-z0-9_.\\/:+-]*(?:python(?:w)?(?:3(?:\.?(?:\d+)?)?)?|py3?|pip(?:3(?:\.\d+)?)?)(?:\.exe)?(?:[\s"'']|$)|(?:lib)?python(?:3(?:\d+)?)?\.(?:lib|dll))')
}

function Get-P2DirectoryFingerprint {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { return 'ABSENT' }
    $root = [IO.Path]::GetFullPath($Path).TrimEnd('\','/')
    $records = @(Get-ChildItem -LiteralPath $root -Recurse -File -Force | Sort-Object FullName | ForEach-Object {
            [ordered]@{ path = $_.FullName.Substring($root.Length + 1).Replace('\','/'); bytes = [int64]$_.Length; sha256 = Get-P2Sha256 $_.FullName }
        })
    return Get-P2TextSha256 -Text ($records | ConvertTo-Json -Depth 6 -Compress)
}

function New-P2ExecutableCanaries {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Root,[Parameter(Mandatory)][string]$ClPath,
        [Parameter(Mandatory)][hashtable]$Environment,[Parameter(Mandatory)][string[]]$Names)
    [void][IO.Directory]::CreateDirectory($Root);$source=Join-Path $Root 'canary.c';$binary=Join-Path $Root 'p2-canary.exe'
    Write-P2Utf8LfFile -Path $source -Text '#include <stdio.h>
int main(void){fputs("P2_CANARY_HIT\n",stderr);return 97;}' -CreateNew
    $compile=Invoke-P2Process -FilePath $ClPath -ArgumentList @('/nologo','/W4','/WX','/O2','/MD',$source,"/Fe:$binary") `
        -WorkingDirectory $Root -Environment $Environment -TimeoutSeconds 120
    if($compile.exit_code-ne0-or-not(Test-Path -LiteralPath $binary -PathType Leaf)){throw 'failed to build executable isolation canary'}
    foreach($name in $Names){[IO.File]::Copy($binary,(Join-Path $Root "$name.exe"),$false)}
    Remove-Item -LiteralPath $binary -Force
}

function Invoke-P2ActivatedGraph {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$CandidateId, [Parameter(Mandatory)][string]$Package,
        [Parameter(Mandatory)][string]$Feature, [Parameter(Mandatory)][string]$Manifest,
        [Parameter(Mandatory)][string]$Cargo, [Parameter(Mandatory)][string]$CommandId,
        [Parameter(Mandatory)][string]$RunRoot, [Parameter(Mandatory)][string]$RepositoryRoot,
        [Parameter(Mandatory)][hashtable]$Environment, [Parameter(Mandatory)][string]$TemporaryRoot,
        [Parameter(Mandatory)][string]$CudaToolkitRoot)
    $command = Invoke-P2RecordedCommand -Id $CommandId -FilePath $Cargo -ArgumentList @(
        'tree','--locked','--offline','--manifest-path',$Manifest,'--package',$Package,'--no-default-features',
        '--features',$Feature,'--target','x86_64-pc-windows-msvc','-e','features','--prefix','none','--format','{p}|{f}') `
        -WorkingDirectory $RepositoryRoot -RunRoot $RunRoot -RepositoryRoot $RepositoryRoot -Environment $Environment `
        -ExpectedExitCodes @(0) -TimeoutSeconds 300 -RoleRoots @{TEMP=$TemporaryRoot;CUDA_TOOLKIT=$CudaToolkitRoot}
    if (-not $command.record.expectation_met) {$ex=[InvalidOperationException]::new("$CandidateId activated dependency graph failed");$ex.Data['P2CommandRecords']=@($command.record);throw$ex}
    $lines = @(([string]$command.raw_stdout -split '\r?\n') | Where-Object { $_ -match '\S' })
    if ($lines.Count -eq 0) {$ex=[InvalidOperationException]::new("$CandidateId activated dependency graph is empty");$ex.Data['P2CommandRecords']=@($command.record);throw$ex}
    $packageCommandId='C'+(([int]$CommandId.Substring(1))+1000).ToString('0000')
    $packageCommand=Invoke-P2RecordedCommand -Id $packageCommandId -FilePath $Cargo -ArgumentList @(
        'tree','--locked','--offline','--manifest-path',$Manifest,'--package',$Package,'--no-default-features',
        '--features',$Feature,'--target','x86_64-pc-windows-msvc','-e','normal,build','--prefix','none','--format','{p}') `
        -WorkingDirectory $RepositoryRoot -RunRoot $RunRoot -RepositoryRoot $RepositoryRoot -Environment $Environment `
        -ExpectedExitCodes @(0) -TimeoutSeconds 300 -RoleRoots @{TEMP=$TemporaryRoot;CUDA_TOOLKIT=$CudaToolkitRoot}
    if(-not$packageCommand.record.expectation_met){$ex=[InvalidOperationException]::new("$CandidateId activated package closure failed");$ex.Data['P2CommandRecords']=@($command.record,$packageCommand.record);throw$ex}
    $packages=@(([string]$packageCommand.raw_stdout-split'\r?\n')|ForEach-Object{$_.Trim().Replace(' (*)','')}|Where-Object{$_-match'\S'}|Sort-Object -Unique)
    $features = @($lines | ForEach-Object { if ($_ -match '\|(?<features>.*)$') { $Matches.features -split ',' } } |
        ForEach-Object { $_.Trim() } | Where-Object { $_ } | Sort-Object -Unique)
    $text = $lines -join "`n"
    if ($CandidateId -ceq 'burn-cubecl' -and (@('autodiff','autotune','cuda','fusion' | Where-Object { $_ -notin $features }).Count -gt 0 -or
        $text -notmatch '(?m)^cubek-matmul v0\.2\.0\|')) {
        $ex=[InvalidOperationException]::new('Burn activated graph lacks cuda/autodiff/fusion/autotune');$ex.Data['P2CommandRecords']=@($command.record,$packageCommand.record);throw$ex
    }
    if ($CandidateId -ceq 'candle' -and ($text -match '(?i)cudnn' -or 'cudnn' -in $features)) {
        $ex=[InvalidOperationException]::new('Candle activated graph unexpectedly enables cuDNN');$ex.Data['P2CommandRecords']=@($command.record,$packageCommand.record);throw$ex
    }
    return [pscustomobject][ordered]@{
        inventory = [ordered]@{ candidate_id=$CandidateId;feature=$Feature;package_count=$packages.Count
            packages_sha256=Get-P2TextSha256 -Text ($packages -join "`n");activated_features=$features
            graph_stdout=$command.record.stdout }
        command = $command; package_command=$packageCommand
    }
}

function Get-P2LoadedModuleProvenance {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][AllowEmptyCollection()][string[]]$LoadedModules,
        [Parameter(Mandatory)][string]$CudaToolkitRoot,
        [Parameter(Mandatory)][string]$WindowsRoot,
        [ValidateSet('burn-cubecl','candle','cudarc-fallback')][string]$CandidateId = 'burn-cubecl',
        [hashtable]$ExpectedModuleHashes=@{},
        [switch]$CpuMode
    )
    if($ExpectedModuleHashes.Count-eq0-and$null-ne$script:P2ExpectedModuleHashes){$ExpectedModuleHashes=$script:P2ExpectedModuleHashes}
    $cudaNames = '(?i)^(?:(?:nv(?:cuda|ml|rtc|jitlink)|cuda(?:rt)?|cu(?:blas|blaslt|dnn|rand|sparse|solver|fft)|nccl)[^\\/]*|(?:lib)?python(?:3(?:\d+)?)?)\.dll$'
    $records = [Collections.Generic.List[object]]::new(); $allAllowed = $true
    foreach ($path in @($LoadedModules | Sort-Object -Unique)) {
        $leaf = Split-Path -Leaf $path
        if ($leaf -notmatch $cudaNames) { continue }
        $full = [IO.Path]::GetFullPath($path)
        $isDriverBoundary=$leaf-imatch'^(?:nvcuda|nvml)\.dll$'
        $isPython=$leaf-imatch'^(?:lib)?python(?:3(?:\d+)?)?\.dll$'
        $allowed = -not$isDriverBoundary-and-not$isPython-and(Test-P2PathWithin -Path $full -Root $CudaToolkitRoot) -and $leaf -notmatch '(?i)^nccl'
        if($CandidateId-ceq'candle'-and$leaf-imatch'^cudnn'){$allowed=$false}
        if ($isDriverBoundary) {
            $allowed = Test-P2PathWithin -Path $full -Root (Join-Path $WindowsRoot 'System32')
        }
        if (-not $allowed -or -not (Test-Path -LiteralPath $full -PathType Leaf)) { $allAllowed = $false }
        $token = if (Test-P2PathWithin -Path $full -Root $CudaToolkitRoot) {
            '${CUDA_TOOLKIT}/' + $full.Substring([IO.Path]::GetFullPath($CudaToolkitRoot).TrimEnd('\').Length + 1).Replace('\', '/')
        }
        elseif (Test-P2PathWithin -Path $full -Root (Join-Path $WindowsRoot 'System32')) {
            '${WINDOWS}/System32/' + $full.Substring([IO.Path]::GetFullPath((Join-Path $WindowsRoot 'System32')).TrimEnd('\').Length + 1).Replace('\', '/')
        }
        else { '<rejected>' }
        if($token-ceq'<rejected>'){$allAllowed=$false;continue}
        $records.Add([pscustomobject][ordered]@{
                path = $token; sha256 = if (Test-Path -LiteralPath $full -PathType Leaf) { Get-P2Sha256 -Path $full } else { '0' * 64 }
            })
        if($ExpectedModuleHashes.ContainsKey($token)-and[string]$records[$records.Count-1].sha256-cne[string]$ExpectedModuleHashes[$token]){$allAllowed=$false}
        elseif($leaf-imatch'^(?:nvcuda|cudart|cublas|cublaslt)[^\\/]*\.dll$'-and-not$ExpectedModuleHashes.ContainsKey($token)){$allAllowed=$false}
    }
    $observedNames = @($records | ForEach-Object { Split-Path -Leaf ([string]$_.path) })
    $required = if($CpuMode){@()}else{@('nvcuda.dll')}
    if($CpuMode-and$records.Count-gt0){$allAllowed=$false}
    foreach ($name in $required) { if ($name -notin $observedNames) { $allAllowed = $false } }
    return [pscustomobject][ordered]@{
        loaded_modules = @($records); qualified_roots = @('${CUDA_TOOLKIT}', '${WINDOWS}/System32')
        all_allowed = $allAllowed
    }
}

function Assert-P2AbbaOrder {
    [CmdletBinding()]
    param([Parameter(Mandatory)][object[]]$Rounds)
    if ($Rounds.Count -ne 8) { throw 'ABBA protocol requires eight workload invocations' }
    $expected = @(
        @('burn-cubecl', 1, 'projection'), @('candle', 2, 'projection'),
        @('candle', 3, 'projection'), @('burn-cubecl', 4, 'projection'),
        @('burn-cubecl', 1, 'ffn-expansion'), @('candle', 2, 'ffn-expansion'),
        @('candle', 3, 'ffn-expansion'), @('burn-cubecl', 4, 'ffn-expansion')
    )
    for ($index = 0; $index -lt $expected.Count; $index++) {
        if ([string]$Rounds[$index].candidate_id -cne $expected[$index][0] -or
            [int]$Rounds[$index].round -ne $expected[$index][1] -or
            [string]$Rounds[$index].workload -cne $expected[$index][2]) {
            throw 'candidate benchmark invocations do not follow canonical ABBA order'
        }
    }
    return $true
}

function New-P2CandidateAggregate {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$CandidateId,
        [AllowNull()][object]$CpuSmoke,
        [AllowNull()][object]$Allocation,
        [AllowNull()][object]$Correctness,
        [Parameter(Mandatory)][object[]]$BenchmarkRounds,
        [object[]]$NvmlMeasurements = @(),
        [AllowNull()]$Summary,
        [Parameter(Mandatory)]$RuntimeProvenance,
        [object[]]$Failures = @()
    )
    $status = if ($null -ne $CpuSmoke -and $null -ne $Allocation -and $null -ne $Correctness -and
        [string]$CpuSmoke.status -ceq 'PASS' -and [string]$Allocation.status -ceq 'PASS' -and
        [string]$Correctness.status -ceq 'PASS' -and @($BenchmarkRounds | Where-Object { $_.status -cne 'PASS' }).Count -eq 0 -and
        $null -ne $Summary -and @($NvmlMeasurements).Count -eq 4 -and
        [bool]$RuntimeProvenance.all_allowed -and [int64]$Summary.max_nvml_gap_ms -le 100 -and
        [int]$Summary.foreign_process_count -eq 0 -and @($Failures).Count -eq 0) { 'PASS' } else { 'FAIL' }
    return [pscustomobject][ordered]@{
        schema = 'python-slm-backend-candidate-aggregate-v1'; candidate_id = $CandidateId; role = 'framework'
        status = $status; cpu_smoke = $CpuSmoke; allocation = $Allocation; correctness = $Correctness
        benchmark_rounds = @($BenchmarkRounds); nvml_measurements = @($NvmlMeasurements); summary = $Summary
        runtime_provenance = $RuntimeProvenance; failures = @($Failures)
    }
}

function New-P2InvocationReference {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Mode, [Parameter(Mandatory)][string]$Workload,
        [AllowNull()][Nullable[int]]$Round, [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$RunRoot, [Parameter(Mandatory)][string]$Status,
        [Parameter(Mandatory)][string]$CommandId)
    $file = New-P2FileReference -Path $Path -RunRoot $RunRoot
    return [pscustomobject][ordered]@{
        mode = $Mode; workload = $Workload; round = if ($null -eq $Round) { $null } else { [int]$Round }
        path = $file.path; sha256 = $file.sha256; bytes = $file.bytes; status = $Status; command_id = $CommandId
    }
}

function Invoke-P2CandidateInvocation {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$CandidateId,
        [Parameter(Mandatory)][string]$Mode,
        [Parameter(Mandatory)][string]$Workload,
        [AllowNull()][Nullable[int]]$Round,
        [Parameter(Mandatory)][string]$Executable,
        [Parameter(Mandatory)][string]$FixtureRoot,
        [Parameter(Mandatory)][string]$ResultRoot,
        [Parameter(Mandatory)][string]$CommandId,
        [Parameter(Mandatory)][string]$RunRoot,
        [Parameter(Mandatory)][string]$RepositoryRoot,
        [Parameter(Mandatory)][string]$WorkingDirectory,
        [Parameter(Mandatory)][hashtable]$Environment,
        [Parameter(Mandatory)][string]$CudaToolkitRoot,
        [Parameter(Mandatory)][string]$TemporaryRoot,
        [hashtable]$ExpectedModuleHashes=@{},
        [switch]$MonitorNvml
    )
    $suffix = if ($null -eq $Round) { "$Mode-$Workload" } else { "round-$Round-$Workload" }
    $candidateResultRoot = Join-Path $ResultRoot $CandidateId; [void][IO.Directory]::CreateDirectory($candidateResultRoot)
    $outputPath = Join-Path $candidateResultRoot ($suffix + '.json')
    $invocationRoot = Join-Path $TemporaryRoot ("invocations\$CandidateId\$suffix")
    [void][IO.Directory]::CreateDirectory($invocationRoot)
    Write-P2Utf8LfFile -Path (Join-Path $invocationRoot 'Cargo.toml') -Text @"
[package]
name = "p2-round-cache"
version = "0.0.0"
edition = "2024"
publish = false
"@ -CreateNew
    $invocationEnvironment = @{}; foreach ($key in $Environment.Keys) { $invocationEnvironment[$key] = $Environment[$key] }
    $invocationEnvironment['CUDA_CACHE_PATH'] = Join-Path $invocationRoot 'cuda-cache'
    $invocationEnvironment['USERPROFILE'] = $invocationRoot; $invocationEnvironment['HOME'] = $invocationRoot
    $invocationEnvironment['TEMP'] = $invocationRoot; $invocationEnvironment['TMP'] = $invocationRoot
    $transientOutput = Join-Path $invocationRoot 'raw-result.json'
    $arguments = @('--mode', $Mode, '--workload', $Workload, '--fixture-dir', $FixtureRoot, '--output', $transientOutput)
    $command = Invoke-P2RecordedCommand -Id $CommandId -FilePath $Executable -ArgumentList $arguments `
        -WorkingDirectory $invocationRoot -RunRoot $RunRoot -RepositoryRoot $RepositoryRoot `
        -Environment $invocationEnvironment -ExpectedExitCodes @(0, 5) -TimeoutSeconds 180 `
        -MonitorNvml:$MonitorNvml -RoleRoots @{ TEMP = $TemporaryRoot; CUDA_TOOLKIT = $CudaToolkitRoot }
    try {
        if (-not $command.record.expectation_met -or [string]$command.record.status -eq 'TIMEOUT' -or
            -not $command.process_tree_terminated -or $command.unexpected_descendants) {
            throw "$CandidateId process tree did not terminate completely"
        }
        if (-not (Test-Path -LiteralPath $transientOutput -PathType Leaf)) {
            throw "$CandidateId did not create its requested result"
        }
        $rawResultText = [IO.File]::ReadAllText($transientOutput, $script:P2Utf8NoBom)
        $rawResult = $rawResultText | ConvertFrom-Json
        $null = Assert-P2CandidateResult -Result $rawResult -CandidateId $CandidateId -Mode $Mode -Workload $Workload
        $safeResultText = Protect-P2Text -Text ($rawResult | ConvertTo-Json -Depth 64 -Compress) `
            -RepositoryRoot $RepositoryRoot -ExtraRoots @($env:USERPROFILE,$env:SystemRoot,$env:TEMP,$env:TMP,$TemporaryRoot) `
            -RoleRoots @{TEMP=$TemporaryRoot;CUDA_TOOLKIT=$CudaToolkitRoot}
        $result = $safeResultText | ConvertFrom-Json
        if (([string]$result.status -ceq 'PASS' -and [int]$command.record.exit_code -ne 0) -or
            ([string]$result.status -ceq 'FAIL' -and [int]$command.record.exit_code -ne 5)) {
            throw "$CandidateId result status does not match its process exit category"
        }
        Write-P2Utf8LfFile -Path $outputPath -Text $safeResultText -CreateNew
        if ($CandidateId -ceq 'burn-cubecl' -and $Mode -cne 'cpu-smoke' -and [string]$result.status -ceq 'PASS') {
            $cargoHome = if ($Environment.ContainsKey('CARGO_HOME')) { [string]$Environment['CARGO_HOME'] } else { Join-Path $env:USERPROFILE '.cargo' }
            [void](Assert-P2BurnAccumulationEvidence -Evidence ([string]$result.provenance.fp32_accumulation_evidence) `
                    -CargoHome $cargoHome)
        }
        $stdoutObject = [string]$command.raw_stdout | ConvertFrom-Json
        if (($stdoutObject | ConvertTo-Json -Depth 64 -Compress) -cne ($rawResult | ConvertTo-Json -Depth 64 -Compress)) {
            throw "$CandidateId stdout does not exactly match its result file"
        }
        $provenance = Get-P2LoadedModuleProvenance -LoadedModules @($command.loaded_modules) `
            -CudaToolkitRoot $CudaToolkitRoot -WindowsRoot $env:SystemRoot -CandidateId $CandidateId `
            -ExpectedModuleHashes $ExpectedModuleHashes -CpuMode:($Mode-ceq'cpu-smoke')
        return [pscustomobject][ordered]@{
            result = $result; command = $command.record; output_path = $outputPath
            reference = New-P2InvocationReference -Mode $Mode -Workload $Workload -Round $Round `
                -Path $outputPath -RunRoot $RunRoot -Status ([string]$result.status) -CommandId $CommandId
            nvml = $command.nvml; runtime_provenance = $provenance
        }
    }
    catch {
        $_.Exception.Data['P2CommandRecord'] = $command.record
        $_.Exception.Data['P2ProcessTreeTerminated'] = [bool]$command.process_tree_terminated
        if ($null -ne $command.nvml) { $_.Exception.Data['P2Nvml'] = $command.nvml }
        throw
    }
}

function New-P2NotRunAggregate {
    param([Parameter(Mandatory)][string]$CandidateId, [Parameter(Mandatory)][string]$Role)
    return [pscustomobject][ordered]@{
        schema = 'python-slm-backend-candidate-aggregate-v1'; candidate_id = $CandidateId; role = $Role
        status = 'NOT_RUN'; cpu_smoke = $null; allocation = $null; correctness = $null
        benchmark_rounds = @(); nvml_measurements = @(); summary = $null; runtime_provenance = $null; failures = @()
    }
}

function Get-P2CandidateFailure {
    param([Parameter(Mandatory)][string]$Message, [AllowNull()][string]$CommandId,
        [string]$Code = 'CANDIDATE_FAILED', [int]$Category = 5)
    return [pscustomobject][ordered]@{
        code = $Code; category = $Category; message = $Message; command_id = $CommandId
    }
}

function Get-P2CanonicalFixtureTable {
    return [ordered]@{
        allocation = [ordered]@{ a_elements=25165824;a_sha256='9b707d439be74adc6b224c46fdba78b82ee590b083fc28d7933be0cc99037bee';b_elements=0;b_sha256=$null;manifest_sha256='13a0b3927af9b208e3b68df4871cf08f550365f976fde8baaac34bd193ad410c' }
        correctness = [ordered]@{ a_elements=527;a_sha256='3a0438fe000928d4384c01166604631d32f2ff4f9e341ba58bd81912c3fecddc';b_elements=899;b_sha256='501883a6afa454243ac78dee6a6940301ba4eee81ea8b33fde86a997a351d182';manifest_sha256='9430bfc198461f718f7b1cce8de0129c9fb55003546eef57c160c83f15dc002f' }
        projection = [ordered]@{ a_elements=6291456;a_sha256='813ffd0eb83b9b71cf9b17361969f0c87da3b1647248f4bee6350cf6c9b42f2f';b_elements=589824;b_sha256='a28d78c9179c8ff579fe6fdfe6f10871ca855a692272eb6d28e079dc8581678c';manifest_sha256='80ca5beec5f84881f8d678bee1ce1e08b9c79efdfc0436570b554de5fb464ed8' }
        'ffn-expansion' = [ordered]@{ a_elements=6291456;a_sha256='09032e0d9115ef46bda8803823901958e28755b5f5c752ec3d0b5435ce67ec2d';b_elements=1867776;b_sha256='2024e09787f70025d046affc45827142f8efcd11c7344a5f11cae36a7e3f4f79';manifest_sha256='d4076314faf44c069eef82f650bee624a66f98b6166f9977c6d3e5b5d17ff5e8' }
    }
}

function Assert-P2FixtureManifestSet {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$FixtureRoot)
    $expected = Get-P2CanonicalFixtureTable; $records = [Collections.Generic.List[object]]::new()
    foreach ($workload in @('allocation','correctness','projection','ffn-expansion')) {
        $path = Join-Path $FixtureRoot "$workload\manifest.json"
        if (-not (Test-Path -LiteralPath $path -PathType Leaf) -or (Get-P2Sha256 $path) -cne $expected[$workload].manifest_sha256) {
            throw "canonical $workload fixture manifest hash mismatch"
        }
        $manifest = [IO.File]::ReadAllText($path,$script:P2Utf8NoBom)|ConvertFrom-Json
        if ([string]$manifest.schema -cne 'python-slm-backend-fixture-v1' -or [string]$manifest.workload -cne $workload -or
            [string]$manifest.generator -cne 'sha256-domain-seed+splitmix64-high-byte-v1' -or
            [string]$manifest.conversion -cne 'signed-high-byte/128.0f32-to-bf16-rne' -or
            [uint64]$manifest.a.elements -ne [uint64]$expected[$workload].a_elements -or
            [string]$manifest.a.sha256 -cne [string]$expected[$workload].a_sha256) { throw "canonical $workload fixture manifest semantics mismatch" }
        if ($expected[$workload].b_elements -eq 0) {
            if ($null -ne $manifest.b) { throw 'allocation fixture unexpectedly contains operand B' }
        } elseif ($null -eq $manifest.b -or [uint64]$manifest.b.elements -ne [uint64]$expected[$workload].b_elements -or
            [string]$manifest.b.sha256 -cne [string]$expected[$workload].b_sha256) { throw "canonical $workload operand B mismatch" }
        foreach ($operand in @('a','b')) {
            $item=$manifest.$operand;if($null -eq $item){continue};$data=Join-Path (Split-Path -Parent $path) ([string]$item.relative_path)
            if ((Get-P2Sha256 $data) -cne [string]$item.sha256 -or [int64](Get-Item $data).Length -ne (2*[int64]$item.elements)) {
                throw "canonical $workload operand bytes mismatch"
            }
        }
        $records.Add([ordered]@{workload=$workload;manifest_sha256=$expected[$workload].manifest_sha256
            a_sha256=$expected[$workload].a_sha256;b_sha256=$expected[$workload].b_sha256
            a_elements=$expected[$workload].a_elements;b_elements=$expected[$workload].b_elements})
    }
    return @($records)
}

function Merge-P2RuntimeProvenance {
    param([Parameter(Mandatory)][object[]]$Records)
    $modules = [Collections.Generic.Dictionary[string, object]]::new([StringComparer]::Ordinal)
    $roots = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $allowed = $true
    foreach ($record in $Records) {
        $allowed = $allowed -and [bool]$record.all_allowed
        foreach ($root in @($record.qualified_roots)) { [void]$roots.Add([string]$root) }
        foreach ($module in @($record.loaded_modules)) { $modules[[string]$module.path] = $module }
    }
    return [pscustomobject][ordered]@{
        loaded_modules = @($modules.Values | Sort-Object path); qualified_roots = @($roots | Sort-Object)
        all_allowed = $allowed
    }
}

function ConvertTo-P2NvmlMeasurement {
    [CmdletBinding()]
    param([Parameter(Mandatory)]$Invocation, [Parameter(Mandatory)][int]$Round,
        [Parameter(Mandatory)][string]$Workload)
    $value = $Invocation.nvml
    if ($null -eq $value) { throw 'benchmark invocation has no native NVML measurement' }
    $baseline = [uint64]$value.BaselineBytes; $peak = [uint64]$value.PeakUsedBytes
    if ($peak -lt $baseline) { throw 'NVML peak is below its baseline' }
    return [pscustomobject][ordered]@{
        command_id = [string]$Invocation.command.id; round = $Round; workload = $Workload
        baseline_samples = 20; baseline_interval_ms = 50; sample_interval_ms = 20
        total_bytes = [uint64]$value.TotalBytes; baseline_bytes = $baseline; peak_bytes = $peak
        delta_bytes = [uint64]($peak - $baseline); sample_count = [int]$value.Samples
        max_gap_ms = [double]$value.MaxGapMs
        baseline_foreign_process_count = [int]$value.BaselineForeignProcessCount
        maximum_foreign_process_count = [int]$value.MaximumForeignProcessCount
        error = if ([string]::IsNullOrWhiteSpace([string]$value.Error)) { $null } else { [string]$value.Error }
    }
}

function Test-P2NvmlMeasurementPolicy {
    param([Parameter(Mandatory)]$Measurement,[Parameter(Mandatory)]$Reference)
    try{
        return [string]$Measurement.command_id-ceq[string]$Reference.command_id-and
            [int]$Measurement.round-eq[int]$Reference.round-and[string]$Measurement.workload-ceq[string]$Reference.workload-and
            [int]$Measurement.baseline_samples-eq20-and[int]$Measurement.baseline_interval_ms-eq50-and
            [int]$Measurement.sample_interval_ms-eq20-and[uint64]$Measurement.total_bytes-gt0-and
            [uint64]$Measurement.peak_bytes-ge[uint64]$Measurement.baseline_bytes-and
            [uint64]$Measurement.delta_bytes-eq([uint64]$Measurement.peak_bytes-[uint64]$Measurement.baseline_bytes)-and
            [int]$Measurement.sample_count-ge1-and[double]$Measurement.max_gap_ms-le100-and
            [int]$Measurement.baseline_foreign_process_count-eq0-and[int]$Measurement.maximum_foreign_process_count-eq0-and$null-eq$Measurement.error
    }catch{return $false}
}

function Get-P2RunNvmlMeasurement {
    param([Parameter(Mandatory)]$Invocation, [Parameter(Mandatory)][int]$Round,
        [Parameter(Mandatory)][string]$Workload, [Parameter(Mandatory)][string]$CandidateId)
    try {
        $measurement = ConvertTo-P2NvmlMeasurement -Invocation $Invocation -Round $Round -Workload $Workload
        if (-not (Test-P2NvmlMeasurementPolicy -Measurement $measurement -Reference $measurement)) {
            throw 'native NVML evidence violates the frozen sampling, gap, or foreign-work policy'
        }
        return $measurement
    }
    catch {
        Stop-P2RunForInvalidMeasurement -CandidateId $CandidateId -Cause $_.Exception
    }
}

function Stop-P2RunForInvalidMeasurement {
    param([Parameter(Mandatory)][string]$CandidateId,
        [Parameter(Mandatory)][System.Exception]$Cause)
    $exception = [InvalidOperationException]::new(
        "P2 measurement protocol invalid for ${CandidateId}: $($Cause.Message)", $Cause)
    $exception.Data['P2Code'] = 'MEASUREMENT_INVALID'
    $exception.Data['P2Category'] = 5
    throw $exception
}

function Get-P2BuildEnvironment {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$TemporaryRoot)
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere -PathType Leaf)) { throw 'vswhere.exe is missing' }
    $raw = & $vswhere -version '[17.0,18.0)' -products '*' `
        -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -format json -utf8
    if ($LASTEXITCODE -ne 0) { throw 'VS2022 discovery failed' }
    $instances = @($raw | ConvertFrom-Json | Where-Object {
            $_.isComplete -and $_.isLaunchable -and -not $_.isRebootRequired
        } | Sort-Object @{ Expression = { [version]$_.installationVersion }; Descending = $true }, instanceId)
    if ($instances.Count -eq 0) { throw 'no qualified VS2022 instance is available' }
    $vsRoot = [IO.Path]::GetFullPath([string]$instances[0].installationPath)
    $devCmd = Join-Path $vsRoot 'Common7\Tools\VsDevCmd.bat'
    if (-not (Test-Path -LiteralPath $devCmd -PathType Leaf)) { throw 'qualified VS2022 developer command is missing' }
    $capture = Join-Path $TemporaryRoot 'capture-vs-env.cmd'
    Write-P2Utf8LfFile -Path $capture -Text @"
@echo off
call "$devCmd" -arch=amd64 -host_arch=amd64 -startdir=none >nul
if errorlevel 1 exit /b %errorlevel%
set
"@ -CreateNew
    $process = Invoke-P2Process -FilePath $env:ComSpec -ArgumentList @('/d', '/c', $capture) `
        -WorkingDirectory $TemporaryRoot -TimeoutSeconds 120
    if ($process.exit_code -ne 0) { throw 'qualified VS2022 environment capture failed' }
    $environment = @{}
    foreach ($line in ($process.stdout -split '\r?\n')) {
        $equals = $line.IndexOf('='); if ($equals -lt 1) { continue }
        $name = $line.Substring(0, $equals); $value = $line.Substring($equals + 1)
        $existing = @($environment.Keys | Where-Object { [string]::Equals($_, $name, [StringComparison]::OrdinalIgnoreCase) })
        if ($existing.Count -gt 0 -and [string]$environment[$existing[0]] -cne $value) {
            throw "VS developer environment contains conflicting $name aliases"
        }
        if ($existing.Count -eq 0) { $environment[$name] = $value }
    }
    return [pscustomobject][ordered]@{ environment = $environment; vs_install = $vsRoot }
}

function New-P2IsolatedEnvironment {
    [CmdletBinding()]
    param([Parameter(Mandatory)][hashtable]$DeveloperEnvironment,
        [Parameter(Mandatory)][string]$TemporaryRoot,
        [Parameter(Mandatory)][string]$VsRoot,
        [Parameter(Mandatory)][string]$CudaToolkitRoot)
    $isolated = @{}
    foreach ($entry in [Environment]::GetEnvironmentVariables().GetEnumerator()) { $isolated[[string]$entry.Key] = $null }
    $allow = '^(?i:Path|PATHEXT|SystemRoot|WINDIR|ComSpec|PROCESSOR_ARCHITECTURE|NUMBER_OF_PROCESSORS|INCLUDE|LIB|LIBPATH|VSINSTALLDIR|VCINSTALLDIR|VCToolsInstallDir|VCToolsVersion|VisualStudioVersion|VSCMD_.*|WindowsSdkDir|WindowsSDKVersion|WindowsSdkBinPath|WindowsSdkVerBinPath|UCRTVersion|UniversalCRTSdkDir|ExtensionSdkDir|FrameworkDir.*|FrameworkVersion.*|NETFXSDKDir|DevEnvDir)$'
    foreach ($name in $DeveloperEnvironment.Keys) {
        if ([string]$name -match $allow) { $isolated[[string]$name] = [string]$DeveloperEnvironment[$name] }
    }
    $cargoHome = if ($env:CARGO_HOME) { [IO.Path]::GetFullPath($env:CARGO_HOME) } else { Join-Path $env:USERPROFILE '.cargo' }
    $rustupHome = if ($env:RUSTUP_HOME) { [IO.Path]::GetFullPath($env:RUSTUP_HOME) } else { Join-Path $env:USERPROFILE '.rustup' }
    $isolated['CARGO_HOME'] = $cargoHome; $isolated['RUSTUP_HOME'] = $rustupHome
    if ($env:RUSTUP_TOOLCHAIN) { $isolated['RUSTUP_TOOLCHAIN'] = [string]$env:RUSTUP_TOOLCHAIN }
    $isolated['USERPROFILE'] = $TemporaryRoot; $isolated['HOME'] = $TemporaryRoot
    $isolated['TEMP'] = $TemporaryRoot; $isolated['TMP'] = $TemporaryRoot
    $allowedRoots = @($VsRoot, [string]$DeveloperEnvironment['WindowsSdkDir'], $env:SystemRoot,
        (Split-Path -Parent (Get-Command cargo.exe).Source), $cargoHome, $rustupHome, $CudaToolkitRoot)
    $parts = [Collections.Generic.List[string]]::new();$pathSet=[Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($part in ([string]$DeveloperEnvironment['Path']).Split(';')) {
        if ([string]::IsNullOrWhiteSpace($part)) { continue }
        try {
            $full = [IO.Path]::GetFullPath($part).TrimEnd('\', '/')
            if (@($allowedRoots | Where-Object { $_ -and (Test-P2PathWithin $full $_) }).Count -gt 0 -and $pathSet.Add($full)) {
                $parts.Add($full)
            }
        }
        catch { }
    }
    foreach ($required in @((Split-Path -Parent (Get-Command cargo.exe).Source),
            (Join-Path $env:SystemRoot 'System32'), (Join-Path $CudaToolkitRoot 'bin'),
            (Join-Path $CudaToolkitRoot 'bin\x64'))) {
        if (Test-Path $required) { $full = [IO.Path]::GetFullPath($required); if ($pathSet.Add($full)) { $parts.Add($full) } }
    }
    $isolated['Path'] = $parts -join ';'; $isolated['CUDA_PATH'] = $CudaToolkitRoot
    $isolated['CARGO_NET_OFFLINE'] = 'true'; $isolated['CARGO_INCREMENTAL'] = '0'; $isolated['CARGO_TERM_COLOR'] = 'never'
    foreach ($name in @('RUSTC_WRAPPER','RUSTC_WORKSPACE_WRAPPER','RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUSTDOCFLAGS','CARGO_ENCODED_RUSTDOCFLAGS',
            'PYTHONHOME','PYTHONPATH','PYTHONNOUSERSITE','VIRTUAL_ENV','CONDA_PREFIX','PIP_CONFIG_FILE')) { $isolated[$name] = $null }
    return $isolated
}

function Assert-P2CargoConfigurationSafe {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$RepositoryRoot,[Parameter(Mandatory)][string]$CargoHome)
    foreach($name in @('RUSTC_WRAPPER','RUSTC_WORKSPACE_WRAPPER','RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUSTDOCFLAGS','CARGO_ENCODED_RUSTDOCFLAGS')){
        $value=[Environment]::GetEnvironmentVariable($name);if(-not[string]::IsNullOrWhiteSpace($value)){throw "build-affecting environment variable is set: $name"}
    }
    $paths=[Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach($root in @($RepositoryRoot,$CargoHome)){
        $cursor=[IO.Path]::GetFullPath($root)
        while($cursor){foreach($relative in @('.cargo\config.toml','.cargo\config','config.toml','config')){$candidate=Join-Path $cursor $relative;if(Test-Path -LiteralPath $candidate -PathType Leaf){[void]$paths.Add($candidate)}}
            $parent=Split-Path -Parent $cursor;if([string]::IsNullOrWhiteSpace($parent)-or$parent-ceq$cursor){break};$cursor=$parent}
    }
    foreach($path in $paths){$text=[IO.File]::ReadAllText($path,$script:P2Utf8NoBom)
        if($text-match'(?im)^\s*(?:rustc|rustc-wrapper|rustc-workspace-wrapper|runner|linker|rustflags|target-dir|build-dir)\s*='-or
            $text-match'(?im)^\s*\[(?:target|source|registries|registry|patch)\b'){throw "build-affecting Cargo configuration is prohibited"}}
    return $true
}

function ConvertTo-P2NvidiaSmiMemoryMiB {
    [CmdletBinding()]
    param([Parameter(Mandatory)][int64]$Bytes)
    if ($Bytes -le 0) { throw 'P1B GPU memory byte count is invalid' }
    # nvidia-smi exposes memory.total as a whole MiB value, while the CUDA
    # runtime receipt retains the exact byte count. Round only after converting
    # the exact value to MiB so the two representations compare coherently.
    return [int64][Math]::Round(
        ([decimal]$Bytes / [decimal]1MB), 0, [MidpointRounding]::AwayFromZero)
}

function Assert-P2LiveP1BEnvironment {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$RepositoryRoot, [Parameter(Mandatory)]$Dependency,
        [Parameter(Mandatory)][string]$CudaToolkitRoot)
    $path = Join-Path $RepositoryRoot ([string]$Dependency.environment_path).Replace('/', '\')
    $qualified = [IO.File]::ReadAllText($path, $script:P2Utf8NoBom) | ConvertFrom-Json
    if ([string]$qualified.status -cne 'PASS' -or [string]$qualified.cuda_toolkit.version -cne '13.1.0' -or
        [string]$qualified.gpu.name -cne 'NVIDIA GeForce RTX 5090' -or
        [int]$qualified.gpu.compute_capability_major -ne 12 -or [int]$qualified.gpu.compute_capability_minor -ne 0) {
        throw 'P1B qualified CUDA/GPU environment is not the required target'
    }
    foreach($toolName in @('rustc','cargo')){
        $tool=(Get-Command ($toolName+'.exe') -ErrorAction Stop).Source;$expected=$qualified.rust.$toolName
        if((Get-P2Sha256 $tool)-cne[string]$expected.sha256){throw "live $toolName hash drifted from P1B"}
        $version=& $tool --version
        if($LASTEXITCODE-ne0-or[string]$version-notmatch("^"+[regex]::Escape($toolName)+" "+[regex]::Escape([string]$expected.version)+"(?: |$)")){throw "live $toolName version drifted from P1B"}
    }
    foreach ($tool in @($qualified.cuda_toolkit.tools)) {
        $candidate = Join-Path $CudaToolkitRoot ([string]$tool.path).Replace('${CUDA_TOOLKIT}/', '').Replace('/', '\')
        if (-not (Test-Path -LiteralPath $candidate -PathType Leaf) -or
            (Get-P2Sha256 -Path $candidate) -cne [string]$tool.sha256) { throw "live CUDA tool drifted: $($tool.name)" }
    }
    foreach ($component in @($qualified.cuda_toolkit.headers) + @($qualified.cuda_toolkit.libraries) +
        @($qualified.cuda_toolkit.runtime_dlls)) {
        $candidate = Join-Path $CudaToolkitRoot ([string]$component.path).Replace('${CUDA_TOOLKIT}/', '').Replace('/', '\')
        if (-not (Test-Path -LiteralPath $candidate -PathType Leaf) -or
            (Get-P2Sha256 -Path $candidate) -cne [string]$component.sha256 -or
            [int64](Get-Item $candidate).Length -ne [int64]$component.bytes) {
            throw "live required CUDA component drifted: $($component.name)"
        }
    }
    $driverPath = Join-Path $env:SystemRoot 'System32\nvcuda.dll'
    if ((Get-P2Sha256 -Path $driverPath) -cne [string]$qualified.driver.library.sha256) { throw 'live CUDA driver DLL drifted from P1B' }
    $smi = Join-Path $env:SystemRoot 'System32\nvidia-smi.exe'
    if ((Get-P2Sha256 -Path $smi) -cne [string]$qualified.driver.nvidia_smi.sha256) {
        throw 'live nvidia-smi drifted from P1B'
    }
    $identity = @(& $smi --query-gpu=name,compute_cap,memory.total,driver_version --format=csv,noheader,nounits)
    $fields=if($identity.Count-eq1){@($identity[0]-split','|ForEach-Object{$_.Trim()})}else{@()}
    $expectedMiB=ConvertTo-P2NvidiaSmiMemoryMiB -Bytes ([int64]$qualified.gpu.memory_total_bytes)
    if ($LASTEXITCODE -ne 0 -or $fields.Count-ne4-or$fields[0]-cne'NVIDIA GeForce RTX 5090'-or$fields[1]-cne'12.0'-or
        [int64]$fields[2]-ne$expectedMiB-or$fields[3]-cne[string]$qualified.driver.driver_version) {
        throw 'live GPU identity no longer matches P1B'
    }
    return [pscustomobject][ordered]@{
        status = 'PASS'; gpu = 'NVIDIA GeForce RTX 5090'; compute_capability = '12.0'
        driver_version = [string]$qualified.driver.driver_version; cuda_toolkit_version = [string]$qualified.cuda_toolkit.version
        manifest = $qualified
    }
}

function Assert-P2BuildEnvironmentMatch {
    param([Parameter(Mandatory)][hashtable]$Environment, [Parameter(Mandatory)]$QualifiedManifest)
    $pathParts = @(([string]$Environment['Path']).Split(';'))
    foreach ($tool in @($QualifiedManifest.visual_studio.tools) + @($QualifiedManifest.windows_sdk.tools)) {
        $resolved = @($pathParts | ForEach-Object { Join-Path $_ ([string]$tool.name + '.exe') } |
            Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -Unique)
        if ($resolved.Count -ne 1 -or (Get-P2Sha256 $resolved[0]) -cne [string]$tool.sha256) {
            throw "live MSVC/SDK tool does not match P1B: $($tool.name)"
        }
    }
    if (([string]$Environment['WindowsSDKVersion']).TrimEnd('\','/') -cne [string]$QualifiedManifest.windows_sdk.version -or
        [string]$Environment['VisualStudioVersion'] -notlike '17.*') {
        throw 'live VS2022/Windows SDK version does not match P1B'
    }
    return $true
}

function New-P2CandidateArtifactReference {
    param([Parameter(Mandatory)][string]$CandidateId, [Parameter(Mandatory)][string]$Role,
        [Parameter(Mandatory)]$Aggregate, [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$RunRoot)
    Write-P2JsonFile -Path $Path -Value $Aggregate -CreateNew
    $ref = New-P2FileReference -Path $Path -RunRoot $RunRoot
    return [pscustomobject][ordered]@{
        candidate_id = $CandidateId; role = $Role; status = [string]$Aggregate.status
        path = $ref.path; sha256 = $ref.sha256; bytes = $ref.bytes
    }
}

function Assert-P2InvocationReference {
    param([Parameter(Mandatory)]$Reference, [Parameter(Mandatory)][string]$RunRoot,
        [Parameter(Mandatory)][string]$CandidateId)
    Assert-P2ClosedObject -Value $Reference -Fields @(
        'mode', 'workload', 'round', 'path', 'sha256', 'bytes', 'status', 'command_id') `
        -Name "$CandidateId invocation reference"
    $basic = [pscustomobject][ordered]@{
        path = [string]$Reference.path; sha256 = [string]$Reference.sha256; bytes = [int64]$Reference.bytes
    }
    $path = Assert-P2FileReference -Reference $basic -RunRoot $RunRoot -Name "$CandidateId invocation"
    $result = [IO.File]::ReadAllText($path, $script:P2Utf8NoBom) | ConvertFrom-Json
    $null = Assert-P2CandidateResult -Result $result -CandidateId $CandidateId `
        -Mode ([string]$Reference.mode) -Workload ([string]$Reference.workload)
    if ([string]$result.status -cne [string]$Reference.status) { throw 'aggregate invocation status does not match raw result' }
    return $result
}

function Assert-P2AggregateRuntimeProvenance {
    param([AllowNull()]$Value,[Parameter(Mandatory)][string]$CandidateId)
    if($null-eq$Value){return}
    Assert-P2ClosedObject $Value @('loaded_modules','qualified_roots','all_allowed') 'aggregate runtime provenance'
    if($Value.all_allowed-isnot[bool]){throw 'aggregate runtime provenance all_allowed is not boolean'}
    $paths=[Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach($module in @($Value.loaded_modules)){
        Assert-P2ClosedObject $module @('path','sha256') 'aggregate loaded module';Assert-P2Sha256 $module.sha256 'aggregate loaded module hash'
        if([string]$module.path-cnotmatch'^\$\{(?:CUDA_TOOLKIT\}/(?:bin(?:/x64)?|lib/x64)|WINDOWS\}/System32(?:/DriverStore/FileRepository/[A-Za-z0-9_.+/-]+)?)/[A-Za-z0-9_.+-]+$'-or-not$paths.Add([string]$module.path)){
            throw 'aggregate loaded module path is unsafe or duplicated'}
        if([string]$module.path-cmatch'(?i)/(?:lib)?python[^/]*\.dll$'-or($CandidateId-ceq'candle'-and[string]$module.path-cmatch'(?i)/cudnn[^/]*\.dll$')-or[string]$module.path-cmatch'(?i)/nccl[^/]*\.dll$'){
            throw 'aggregate runtime provenance contains a prohibited library'}
    }
    $roots=@($Value.qualified_roots);if(@($roots|Sort-Object -Unique).Count-ne$roots.Count-or@($roots|Where-Object{$_-notin@('${CUDA_TOOLKIT}','${WINDOWS}/System32','${WINDOWS}/System32/DriverStore/FileRepository')}).Count-ne0){
        throw 'aggregate runtime provenance roots are invalid'}
}

function Assert-P2AggregateSummary {
    param([AllowNull()]$Value)
    if($null-eq$Value){return}
    Assert-P2ClosedObject $Value @('geomean_fwbw_p50_ns','geomean_fwbw_p95_ns','observed_peak_bytes','locked_dependency_count','max_nvml_gap_ms','foreign_process_count') 'aggregate summary'
    [void](Assert-P2FiniteNumber $Value.geomean_fwbw_p50_ns 'aggregate p50 geometric mean' 0.0000001)
    [void](Assert-P2FiniteNumber $Value.geomean_fwbw_p95_ns 'aggregate p95 geometric mean' 0.0000001)
    [void](Assert-P2IntegerNumber $Value.observed_peak_bytes 'aggregate observed peak' 0)
    [void](Assert-P2IntegerNumber $Value.locked_dependency_count 'aggregate dependency count' 1)
    [void](Assert-P2FiniteNumber $Value.max_nvml_gap_ms 'aggregate maximum NVML gap' 0)
    [void](Assert-P2IntegerNumber $Value.foreign_process_count 'aggregate foreign process count' 0)
}

function Assert-P2CandidateAggregate {
    [CmdletBinding()]
    param([Parameter(Mandatory)]$Aggregate, [Parameter(Mandatory)][string]$RunRoot)
    Assert-P2ClosedObject -Value $Aggregate -Fields @(
        'schema', 'candidate_id', 'role', 'status', 'cpu_smoke', 'allocation', 'correctness',
        'benchmark_rounds', 'nvml_measurements', 'summary', 'runtime_provenance', 'failures') -Name 'candidate aggregate'
    $candidateId = [string]$Aggregate.candidate_id
    if ([string]$Aggregate.schema -cne $script:P2Schema.Aggregate -or
        $candidateId -notin @('burn-cubecl', 'candle', 'cudarc-fallback') -or
        [string]$Aggregate.status -notin @('PASS', 'FAIL', 'NOT_RUN')) { throw 'candidate aggregate identity is invalid' }
    $expectedRole=if($candidateId-ceq'cudarc-fallback'){'diagnostic_fallback'}else{'framework'}
    if([string]$Aggregate.role-cne$expectedRole){throw 'candidate aggregate role is invalid'}
    foreach($failure in @($Aggregate.failures)){
        Assert-P2ClosedObject $failure @('code','category','message','command_id') 'candidate failure'
        if([string]$failure.code-cnotmatch'^[A-Z][A-Z0-9_]*$'-or[int]$failure.category-lt1-or[int]$failure.category-gt5-or
            [string]::IsNullOrWhiteSpace([string]$failure.message)-or($null-ne$failure.command_id-and[string]$failure.command_id-cnotmatch'^C[0-9]{2,4}$')){
            throw 'candidate failure record is malformed'
        }
    }
    if ([string]$Aggregate.status -ceq 'NOT_RUN') {
        if($null-ne$Aggregate.cpu_smoke-or$null-ne$Aggregate.allocation-or$null-ne$Aggregate.correctness-or
            @($Aggregate.benchmark_rounds).Count-ne0-or@($Aggregate.nvml_measurements).Count-ne0-or$null-ne$Aggregate.summary-or
            $null-ne$Aggregate.runtime_provenance-or@($Aggregate.failures).Count-ne0){throw 'NOT_RUN aggregate contains attempted evidence'}
        return $Aggregate
    }
    Assert-P2AggregateRuntimeProvenance -Value $Aggregate.runtime_provenance -CandidateId $candidateId
    Assert-P2AggregateSummary -Value $Aggregate.summary
    $measurements=@($Aggregate.nvml_measurements);if($measurements.Count-gt4){throw 'aggregate has too many NVML measurements'}
    foreach($measurement in $measurements){
        Assert-P2ClosedObject $measurement @('command_id','round','workload','baseline_samples','baseline_interval_ms','sample_interval_ms','total_bytes','baseline_bytes','peak_bytes','delta_bytes','sample_count','max_gap_ms','baseline_foreign_process_count','maximum_foreign_process_count','error') 'aggregate NVML measurement'
        if(-not(Test-P2NvmlMeasurementPolicy $measurement $measurement)){throw 'aggregate contains an invalid run-global NVML measurement'}
    }
    $results = [Collections.Generic.List[object]]::new()
    foreach ($reference in @($Aggregate.cpu_smoke, $Aggregate.allocation, $Aggregate.correctness) + @($Aggregate.benchmark_rounds)) {
        if ($null -ne $reference) { $results.Add((Assert-P2InvocationReference -Reference $reference -RunRoot $RunRoot -CandidateId $candidateId)) }
    }
    if ([string]$Aggregate.status -ceq 'PASS') {
        if ($candidateId -ceq 'cudarc-fallback') {
            if ($results.Count -ne 2 -or $null -ne $Aggregate.cpu_smoke -or @($Aggregate.benchmark_rounds).Count -ne 0 -or
                @($Aggregate.nvml_measurements).Count -ne 0 -or $null -ne $Aggregate.summary -or
                -not [bool]$Aggregate.runtime_provenance.all_allowed -or @($Aggregate.failures).Count -ne 0) {
                throw 'passing diagnostic fallback aggregate violates its bounded role'
            }
            return $Aggregate
        }
        $modules=@($Aggregate.runtime_provenance.loaded_modules);$paths=@($modules.path)
        if($modules.Count-lt1-or@($paths|Sort-Object -Unique).Count-ne$paths.Count-or
            '${WINDOWS}/System32/nvcuda.dll'-notin$paths-or'${WINDOWS}/System32'-notin@($Aggregate.runtime_provenance.qualified_roots)){
            throw 'passing runtime provenance lacks the qualified CUDA driver boundary'
        }
        foreach($module in $modules){Assert-P2ClosedFields $module @('path','sha256') 'loaded CUDA module';Assert-P2Sha256 $module.sha256 'loaded CUDA module hash'}
        if ($results.Count -ne 7 -or
            @($results | Where-Object { $_.status -cne 'PASS' }).Count -ne 0 -or
            $null -eq $Aggregate.summary -or -not [bool]$Aggregate.runtime_provenance.all_allowed -or
            [double]$Aggregate.summary.max_nvml_gap_ms -gt 100 -or
            [int]$Aggregate.summary.foreign_process_count -ne 0 -or @($Aggregate.failures).Count -ne 0) {
            throw 'passing candidate aggregate violates qualification gates'
        }
        $rounds = @($Aggregate.benchmark_rounds | ForEach-Object {
                [pscustomobject]@{ candidate_id = $candidateId; round = $_.round; workload = $_.workload }
            })
        $expectedRounds = if ($candidateId -ceq 'burn-cubecl') { @(1, 1, 4, 4) } else { @(2, 2, 3, 3) }
        if ((@($rounds.round) -join ',') -cne ($expectedRounds -join ',') -or
            (@($rounds.workload) -join ',') -cne 'projection,ffn-expansion,projection,ffn-expansion') {
            throw 'passing aggregate benchmark round layout is invalid'
        }
        $measurements = @($Aggregate.nvml_measurements)
        if ($measurements.Count -ne 4) { throw 'passing framework aggregate requires four NVML measurements' }
        for ($index = 0; $index -lt 4; $index++) {
            $measurement = $measurements[$index]; $reference = @($Aggregate.benchmark_rounds)[$index]
            Assert-P2ClosedObject -Value $measurement -Fields @('command_id','round','workload','baseline_samples',
                'baseline_interval_ms','sample_interval_ms','total_bytes','baseline_bytes','peak_bytes','delta_bytes',
                'sample_count','max_gap_ms','baseline_foreign_process_count','maximum_foreign_process_count','error') `
                -Name 'NVML measurement'
            if (-not(Test-P2NvmlMeasurementPolicy -Measurement $measurement -Reference $reference)) {
                throw 'NVML measurement violates the frozen sampling and contention policy'
            }
        }
    }
    elseif (@($Aggregate.failures).Count -eq 0) { throw 'failed aggregate contains no failure' }
    elseif($candidateId-ceq'cudarc-fallback'-and(@($Aggregate.benchmark_rounds).Count-ne0-or@($Aggregate.nvml_measurements).Count-ne0-or$null-ne$Aggregate.summary-or$null-ne$Aggregate.cpu_smoke)){
        throw 'failed diagnostic fallback exceeds its bounded role'
    }
    return $Aggregate
}

function Invoke-P2FrameworkPreflight {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$CandidateId,
        [Parameter(Mandatory)][string]$CpuExecutable,
        [Parameter(Mandatory)][string]$CudaExecutable,
        [Parameter(Mandatory)][string]$FixtureRoot,
        [Parameter(Mandatory)][string]$ResultRoot,
        [Parameter(Mandatory)][string]$RunRoot,
        [Parameter(Mandatory)][string]$RepositoryRoot,
        [Parameter(Mandatory)][string]$WorkingDirectory,
        [Parameter(Mandatory)][hashtable]$Environment,
        [Parameter(Mandatory)][hashtable]$CpuEnvironment,
        [Parameter(Mandatory)][string]$CudaToolkitRoot,
        [Parameter(Mandatory)][string]$TemporaryRoot,
        [Parameter(Mandatory)][ref]$CommandNumber
    )
    $commands = [Collections.Generic.List[object]]::new(); $provenance = [Collections.Generic.List[object]]::new()
    $failures = [Collections.Generic.List[object]]::new();$recoveries=[Collections.Generic.List[object]]::new()
    $nextId = { $id = 'C' + $CommandNumber.Value.ToString('00'); $CommandNumber.Value++; return $id }
    $invoke = {
        param($Mode, $Workload, $Round, $Executable, $Monitor)
        $id = & $nextId
        try {
            $childEnvironment=if($Mode-ceq'cpu-smoke'){$CpuEnvironment}else{$Environment}
            $value = Invoke-P2CandidateInvocation -CandidateId $CandidateId -Mode $Mode -Workload $Workload `
                -Round $Round -Executable $Executable -FixtureRoot $FixtureRoot -ResultRoot $ResultRoot `
                -CommandId $id -RunRoot $RunRoot -RepositoryRoot $RepositoryRoot -WorkingDirectory $WorkingDirectory `
                -Environment $childEnvironment -CudaToolkitRoot $CudaToolkitRoot -TemporaryRoot $TemporaryRoot `
                -MonitorNvml:$Monitor
            $commands.Add($value.command); $provenance.Add($value.runtime_provenance)
            if ([string]$value.result.status -cne 'PASS') {
                $failures.Add((Get-P2CandidateFailure -Message "$CandidateId $Mode/$Workload returned FAIL" `
                        -CommandId $id -Code 'CANDIDATE_RESULT_FAILED'))
                if($Mode-cne'cpu-smoke'){$health=[P2CudaHealth]::Probe();if($health-cne'PASS'){throw "CUDA health probe failed after candidate FAIL: $health"}
                    $failures.Add((Get-P2CandidateFailure -Message 'Fresh CUDA health probe passed after candidate failure.' -CommandId $id -Code 'CUDA_HEALTH_RECOVERY_PASSED'))
                    $recoveries.Add([ordered]@{candidate_id=$CandidateId;failed_command_id=$id;trigger='candidate_result_fail';result='PASS'})}
            }
            return $value
        }
        catch {
            if ($_.Exception.Data.Contains('P2CommandRecord')) {
                $commands.Add($_.Exception.Data['P2CommandRecord'])
            }
            $failures.Add((Get-P2CandidateFailure -Message ([string]$_.Exception.Message) -CommandId $id))
            $health = [P2CudaHealth]::Probe()
            if ($health -cne 'PASS') {
                $failures.Add((Get-P2CandidateFailure -Message "CUDA health probe failed after candidate failure: $health" `
                        -CommandId $id -Code 'CUDA_HEALTH_FAILED' -Category 4))
                throw "CUDA health probe failed after candidate failure: $health"
            }
            $failures.Add((Get-P2CandidateFailure -Message 'Fresh CUDA health probe passed after candidate process failure.' -CommandId $id -Code 'CUDA_HEALTH_RECOVERY_PASSED'))
            $recoveries.Add([ordered]@{candidate_id=$CandidateId;failed_command_id=$id;trigger='candidate_process_failure';result='PASS'})
            return $null
        }
    }
    $cpu = & $invoke 'cpu-smoke' 'correctness' $null $CpuExecutable $false
    $allocation = if ($null -ne $cpu -and $cpu.result.status-ceq'PASS') { & $invoke 'correctness' 'allocation' $null $CudaExecutable $false } else { $null }
    $correctness = if ($null -ne $allocation -and $allocation.result.status-ceq'PASS') { & $invoke 'correctness' 'correctness' $null $CudaExecutable $false } else { $null }
    return [pscustomobject][ordered]@{
        cpu = $cpu; allocation = $allocation; correctness = $correctness
        commands = @($commands); provenance = @($provenance); failures = @($failures);recoveries=@($recoveries)
        eligible = $null -ne $correctness -and [string]$correctness.result.status -ceq 'PASS' -and $failures.Count -eq 0
    }
}

function New-P2SchemaBundleIdentity {
    param([Parameter(Mandatory)][string]$RepositoryRoot)
    $root = Join-Path $RepositoryRoot 'docs\schemas\P2'
    $files = @(Get-ChildItem -LiteralPath $root -File -Filter '*.schema.json' | Sort-Object Name)
    if ($files.Count -ne 7) { throw 'P2 schema bundle must contain exactly seven schemas' }
    $records = @($files | ForEach-Object {
            [ordered]@{ path = 'docs/schemas/P2/' + $_.Name; sha256 = Get-P2Sha256 -Path $_.FullName; bytes = [int64]$_.Length }
        })
    return [pscustomobject][ordered]@{
        files = $records; bundle_sha256 = Get-P2TextSha256 -Text ($records | ConvertTo-Json -Depth 8 -Compress)
    }
}

function New-P2Decision {
    param([Parameter(Mandatory)][string]$RunId, [Parameter(Mandatory)]$Selection,
        [Parameter(Mandatory)][object[]]$Aggregates, [Parameter(Mandatory)]$Fallback)
    $passing = @($Aggregates | Where-Object { $_.candidate_id -in @('burn-cubecl', 'candle') -and $_.status -ceq 'PASS' } |
        Sort-Object candidate_id | ForEach-Object { [string]$_.candidate_id })
    $rejected = @($Aggregates | Where-Object { $_.candidate_id -in @('burn-cubecl', 'candle') -and $_.status -ne 'PASS' } |
        Sort-Object candidate_id | ForEach-Object {
            [ordered]@{ candidate_id = [string]$_.candidate_id; reason_codes = @($_.failures.code | Sort-Object -Unique) }
        })
    return [ordered]@{
        schema = $script:P2Schema.Decision; phase_id = 'P2'; policy_id = 'backend-qualification-v1'
        status = [string]$Selection.status; owner_review_status = 'AWAITING_REVIEW'
        selected_candidate_id = $Selection.selected_candidate_id
        selection_criterion = if ([string]$Selection.status -ceq 'PASS') { [string]$Selection.criterion } else { $null }
        passing_frameworks = $passing; rejected_candidates = $rejected
        diagnostic_fallback = $Fallback; qualification_run_id = $RunId
        limitations = @('primitive_bf16_training_viability_only', 'transformer_layer_parity_deferred_to_P10',
            'full_optimizer_step_deferred_to_P12', 'no_vram_fit_claim', 'no_tokens_per_second_claim',
            'no_eight_hour_feasibility_claim')
    }
}

function Assert-P2Decision {
    param([Parameter(Mandatory)]$Decision, [Parameter(Mandatory)][string]$RunId)
    Assert-P2ClosedObject -Value $Decision -Fields @(
        'schema', 'phase_id', 'policy_id', 'status', 'owner_review_status', 'selected_candidate_id',
        'selection_criterion', 'passing_frameworks', 'rejected_candidates', 'diagnostic_fallback',
        'qualification_run_id', 'limitations') -Name 'P2 selection decision'
    if ([string]$Decision.schema -cne $script:P2Schema.Decision -or [string]$Decision.phase_id -cne 'P2' -or
        [string]$Decision.policy_id -cne 'backend-qualification-v1' -or
        [string]$Decision.owner_review_status -cne 'AWAITING_REVIEW' -or [string]$Decision.qualification_run_id -cne $RunId -or
        [string]$Decision.status -notin @('PASS', 'FAIL')) { throw 'P2 decision identity is invalid' }
    if ([string]$Decision.status -ceq 'PASS' -and
        ([string]$Decision.selected_candidate_id -notin @('burn-cubecl', 'candle') -or
            @($Decision.passing_frameworks).Count -lt 1)) { throw 'passing P2 decision selects no qualified framework' }
    if ([string]$Decision.status -ceq 'FAIL' -and $null -ne $Decision.selected_candidate_id) {
        throw 'failed P2 decision cannot select a framework'
    }
    return $Decision
}

function Write-P2FailureRun {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$RunRoot,
        [Parameter(Mandatory)][string]$RunId,
        [Parameter(Mandatory)][DateTime]$StartedAt,
        [Parameter(Mandatory)][Diagnostics.Stopwatch]$Stopwatch,
        [Parameter(Mandatory)][string]$SourceIdentitySha256,
        [AllowNull()]$P1BDependency,
        [Parameter(Mandatory)]$Gates,
        [Parameter(Mandatory)][AllowEmptyCollection()][object[]]$Commands,
        [Parameter(Mandatory)][object[]]$Errors,
        [bool]$CleanupAttempted,
        [bool]$TemporaryRootRemoved,
        [bool]$ProcessTreesTerminated,
        [AllowNull()]$Policy,
        [AllowNull()]$DependencyInventory,
        [AllowNull()]$FixtureManifest,
        [AllowNull()]$HostState,
        [AllowNull()]$CpuIsolation,
        [object[]]$CandidateReferences = @(),
        [AllowNull()]$Comparison,
        [AllowNull()]$Decision
    )
    $commandsRoot=Join-Path $RunRoot 'commands';[void][IO.Directory]::CreateDirectory($commandsRoot);Assert-P2OwnedDirectory $commandsRoot $RunRoot
    $artifacts = Join-Path $RunRoot 'artifacts'; [void][IO.Directory]::CreateDirectory($artifacts);Assert-P2OwnedDirectory $artifacts $RunRoot
    $placeholder = { param($name, $value)
        $path = Join-Path $artifacts $name
        if (-not (Test-Path -LiteralPath $path)) { Write-P2JsonFile -Path $path -Value $value -CreateNew }
        New-P2FileReference -Path $path -RunRoot $RunRoot
    }
    $sourceIdentityPath=Join-Path $artifacts 'source-identity.json'
    if(-not(Test-Path -LiteralPath $sourceIdentityPath -PathType Leaf)){
        Write-P2JsonFile -Path $sourceIdentityPath -Value ([ordered]@{
                schema='python-slm-p2-source-identity-v1';status='NOT_RUN';reason='Source identity was unavailable before terminal failure.'
            }) -CreateNew
    }
    $SourceIdentitySha256=Get-P2Sha256 -Path $sourceIdentityPath
    $notRunP1B=[ordered]@{
        status='NOT_RUN';pointer_path=$null;pointer_sha256=$null;acceptance_path=$null;acceptance_sha256=$null;acceptance_sequence=$null
        run_path=$null;run_evidence_sha256=$null;seal_path=$null;seal_sha256=$null;environment_path=$null;environment_sha256=$null
        source_identity_sha256=$null;verifier_sha256=$null;schema_bundle_sha256=$null;review_closure_commit=$null
    }
    $effectiveP1B=if($null-eq$P1BDependency){$notRunP1B}else{$P1BDependency}
    $p1bArtifactPath=Join-Path $artifacts 'p1b-dependency.json'
    if(-not(Test-Path -LiteralPath $p1bArtifactPath -PathType Leaf)){Write-P2JsonFile -Path $p1bArtifactPath -Value $effectiveP1B -CreateNew}
    else{$retainedP1B=[IO.File]::ReadAllText($p1bArtifactPath,$script:P2Utf8NoBom)|ConvertFrom-Json
        if(($retainedP1B|ConvertTo-Json -Depth 32 -Compress)-cne($effectiveP1B|ConvertTo-Json -Depth 32 -Compress)){throw 'retained P1B dependency disagrees with terminal failure state'}}
    if ($null -eq $Policy) { $Policy = New-P2Policy }
    $policyRef = & $placeholder 'qualification-policy.json' $Policy
    $dependencyRef = & $placeholder 'dependency-inventory.json' $(if ($null -eq $DependencyInventory) {
            [ordered]@{ schema = 'python-slm-p2-dependency-inventory-v1'; status = 'NOT_RUN' }
        } else { $DependencyInventory })
    $fixtureRef = & $placeholder 'fixture-manifest.json' $(if ($null -eq $FixtureManifest) {
            [ordered]@{ schema = 'python-slm-p2-fixture-manifest-v1'; status = 'NOT_RUN' }
        } else { $FixtureManifest })
    $hostRef = & $placeholder 'host-state.json' $(if ($null -eq $HostState) {
            [ordered]@{ schema = 'python-slm-p2-host-state-v1'; status = 'NOT_RUN' }
        } else { $HostState })
    $cpuRef = & $placeholder 'cpu-isolation.json' $(if ($null -eq $CpuIsolation) {
            [ordered]@{ schema = 'python-slm-p2-cpu-isolation-v1'; status = 'NOT_RUN'; forbidden_hits = @(); cuda_or_python_discovered = $false }
        } else { $CpuIsolation })
    $frameworkAggregates=[Collections.Generic.List[object]]::new();$frameworkRefs=[Collections.Generic.List[object]]::new()
    foreach($candidateId in @('burn-cubecl','candle')){
        $candidatePath=Join-Path $artifacts "$candidateId.json"
        if(-not(Test-Path -LiteralPath $candidatePath -PathType Leaf)){
            Write-P2JsonFile -Path $candidatePath -Value (New-P2NotRunAggregate -CandidateId $candidateId -Role framework) -CreateNew
        }
        $aggregate=[IO.File]::ReadAllText($candidatePath,$script:P2Utf8NoBom)|ConvertFrom-Json
        $null=Assert-P2CandidateAggregate -Aggregate $aggregate -RunRoot $RunRoot;$frameworkAggregates.Add($aggregate)
        $ref=New-P2FileReference -Path $candidatePath -RunRoot $RunRoot
        $frameworkRefs.Add([ordered]@{candidate_id=$candidateId;role='framework';status=[string]$aggregate.status;path=$ref.path;sha256=$ref.sha256;bytes=$ref.bytes})
    }
    $fallbackPath=Join-Path $artifacts 'cudarc-fallback.json'
    if(Test-Path -LiteralPath $fallbackPath -PathType Leaf){
        $fallbackAggregate=[IO.File]::ReadAllText($fallbackPath,$script:P2Utf8NoBom)|ConvertFrom-Json;$null=Assert-P2CandidateAggregate $fallbackAggregate $RunRoot
        $fallbackBasic=New-P2FileReference -Path $fallbackPath -RunRoot $RunRoot
        $frameworkRefs.Add([ordered]@{candidate_id='cudarc-fallback';role='diagnostic_fallback';status=[string]$fallbackAggregate.status;path=$fallbackBasic.path;sha256=$fallbackBasic.sha256;bytes=$fallbackBasic.bytes})
    }
    $comparisonPath=Join-Path $artifacts 'comparison.json'
    if(-not(Test-Path -LiteralPath $comparisonPath -PathType Leaf)){
        $selectionCandidates=@($frameworkAggregates|ForEach-Object{[pscustomobject]@{candidate_id=$_.candidate_id;status=$_.status;comparison=$_.summary}})
        $selection=Select-P2Candidate -Candidates $selectionCandidates
        Write-P2JsonFile -Path $comparisonPath -Value ([ordered]@{schema='python-slm-p2-backend-comparison-v1';policy_id='backend-qualification-v1';candidates=$selectionCandidates;selection=$selection}) -CreateNew
    }
    $comparisonRef=New-P2FileReference -Path $comparisonPath -RunRoot $RunRoot
    $decisionPath=Join-Path $artifacts 'decision.json'
    if(-not(Test-Path -LiteralPath $decisionPath -PathType Leaf)){
        $rejected=@($frameworkAggregates|ForEach-Object{[ordered]@{candidate_id=$_.candidate_id;reason_codes=@('PHASE_TERMINATED_BEFORE_CANDIDATE')}})
        $decisionValue=[ordered]@{schema=$script:P2Schema.Decision;phase_id='P2';policy_id='backend-qualification-v1';status='FAIL';owner_review_status='AWAITING_REVIEW'
            selected_candidate_id=$null;selection_criterion=$null;passing_frameworks=@();rejected_candidates=$rejected
            diagnostic_fallback=[ordered]@{status='NOT_RUN';reason='The phase terminated before diagnostic fallback eligibility was established.'}
            qualification_run_id=$RunId;limitations=@('primitive_bf16_training_viability_only','transformer_layer_parity_deferred_to_P10',
                'full_optimizer_step_deferred_to_P12','no_vram_fit_claim','no_tokens_per_second_claim','no_eight_hour_feasibility_claim')}
        $null=Assert-P2Decision -Decision $decisionValue -RunId $RunId;Write-P2JsonFile -Path $decisionPath -Value $decisionValue -CreateNew
    }
    $decisionValue=[IO.File]::ReadAllText($decisionPath,$script:P2Utf8NoBom)|ConvertFrom-Json;$decisionBasic=New-P2FileReference -Path $decisionPath -RunRoot $RunRoot
    $decisionRef=[ordered]@{path=$decisionBasic.path;sha256=$decisionBasic.sha256;bytes=$decisionBasic.bytes;status=[string]$decisionValue.status
        selected_candidate_id=$decisionValue.selected_candidate_id;owner_review_status='AWAITING_REVIEW'}
    $failureSummary = [ordered]@{ schema = 'python-slm-p2-failure-summary-v1'; phase_id = 'P2'; run_id = $RunId; errors = @($Errors) }
    Write-P2JsonFile -Path (Join-Path $artifacts 'failure-summary.json') -Value $failureSummary -CreateNew
    $sealCount = @(Get-ChildItem -LiteralPath $RunRoot -Recurse -File).Count + 1
    $evidence = [ordered]@{
        schema = $script:P2Schema.Evidence; phase_id = 'P2'; run_id = $RunId; status = 'FAIL'
        owner_review_status = 'AWAITING_REVIEW'; started_at = $StartedAt.ToUniversalTime().ToString('o')
        finished_at = [DateTime]::UtcNow.ToString('o'); duration_ms = [int64]$Stopwatch.ElapsedMilliseconds
        invocation = [ordered]@{ argv = @('powershell', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
                'scripts/qualify-backend.ps1', '-OutputRoot', 'docs/receipts/P2'); cwd = '${REPO}' }
        source_identity_sha256 = $SourceIdentitySha256
        p1b_dependency = $effectiveP1B
        policy = $policyRef; dependency_inventory = $dependencyRef; fixture_manifest = $fixtureRef; host_state = $hostRef
        candidates = @($frameworkRefs); comparison = $comparisonRef; decision = $decisionRef; cpu_isolation = $cpuRef
        commands = @($Commands); gates = $Gates; errors = @($Errors)
        cleanup = [ordered]@{ attempted = $CleanupAttempted; temporary_root_removed = $TemporaryRootRemoved; process_trees_terminated = $ProcessTreesTerminated }
        seal = [ordered]@{ path = 'SHA256SUMS'; entries = $sealCount; coverage_rule = 'all_run_files_except_seal' }
    }
    Write-P2JsonFile -Path (Join-Path $RunRoot 'evidence.json') -Value $evidence -CreateNew
    $actualFiles = @(Get-ChildItem -LiteralPath $RunRoot -Recurse -File).Count
    $evidence.seal.entries = $actualFiles
    Write-P2JsonFile -Path (Join-Path $RunRoot 'evidence.json') -Value $evidence
    [void](Test-P2ReceiptRedaction -RunRoot $RunRoot)
    [void](New-P2Seal -RunRoot $RunRoot)
    [void](Assert-P2FailureRunLayout -RunRoot $RunRoot -RunId $RunId)
    return $evidence
}

function Assert-P2FailureRunLayout {
    param([Parameter(Mandatory)][string]$RunRoot,[Parameter(Mandatory)][string]$RunId)
    if(-not(Test-P2Seal -RunRoot $RunRoot)){throw 'terminal P2 failure seal is invalid'}
    $evidence=[IO.File]::ReadAllText((Join-Path $RunRoot 'evidence.json'),$script:P2Utf8NoBom)|ConvertFrom-Json
    if([string]$evidence.status-cne'FAIL'-or[string]$evidence.run_id-cne$RunId-or@($evidence.errors).Count-lt1){throw 'terminal P2 failure evidence is invalid'}
    $required=@('source-identity.json','p1b-dependency.json','qualification-policy.json','dependency-inventory.json','fixture-manifest.json',
        'host-state.json','burn-cubecl.json','candle.json','comparison.json','decision.json','cpu-isolation.json','failure-summary.json')
    foreach($name in $required){if(-not(Test-Path -LiteralPath (Join-Path $RunRoot "artifacts\$name") -PathType Leaf)){throw "terminal P2 failure artifact is missing: $name"}}
    $sourcePath=Join-Path $RunRoot 'artifacts\source-identity.json';if((Get-P2Sha256 $sourcePath)-cne[string]$evidence.source_identity_sha256){throw 'terminal source identity is not hash-bound'}
    $p1b=[IO.File]::ReadAllText((Join-Path $RunRoot 'artifacts\p1b-dependency.json'),$script:P2Utf8NoBom)|ConvertFrom-Json
    if(($p1b|ConvertTo-Json -Depth 32 -Compress)-cne($evidence.p1b_dependency|ConvertTo-Json -Depth 32 -Compress)){throw 'terminal P1B dependency is not evidence-bound'}
    $candidateIds=@($evidence.candidates.candidate_id);if($candidateIds.Count-notin@(2,3)-or($candidateIds[0..1]-join',')-cne'burn-cubecl,candle'-or
        ($candidateIds.Count-eq3-and[string]$candidateIds[2]-cne'cudarc-fallback')){throw 'terminal candidate references are incomplete or unordered'}
    foreach($ref in @($evidence.candidates)){Assert-P2ClosedObject $ref @('candidate_id','role','status','path','sha256','bytes') 'terminal candidate reference'
        $path=Assert-P2FileReference ([pscustomobject]@{path=$ref.path;sha256=$ref.sha256;bytes=$ref.bytes}) $RunRoot 'terminal candidate artifact'
        $aggregate=[IO.File]::ReadAllText($path,$script:P2Utf8NoBom)|ConvertFrom-Json;$null=Assert-P2CandidateAggregate $aggregate $RunRoot
        if([string]$aggregate.candidate_id-cne[string]$ref.candidate_id-or[string]$aggregate.status-cne[string]$ref.status){throw 'terminal candidate reference disagrees with aggregate'}}
    $null=Assert-P2FileReference $evidence.comparison $RunRoot 'terminal comparison'
    $null=Assert-P2DecisionReference $evidence.decision $RunRoot 'terminal decision'
    return $true
}

function Convert-P2UnpublishedPassToFailure {
    param([Parameter(Mandatory)][string]$RunRoot,[Parameter(Mandatory)]$ErrorRecord)
    $evidencePath=Join-Path $RunRoot 'evidence.json';$sealPath=Join-Path $RunRoot 'SHA256SUMS'
    if(-not(Test-Path -LiteralPath $evidencePath -PathType Leaf)){return $false}
    $evidence=[IO.File]::ReadAllText($evidencePath,$script:P2Utf8NoBom)|ConvertFrom-Json
    if([string]$evidence.status-cne'PASS'){return $false}
    $backupRoot=Join-Path ([IO.Path]::GetTempPath()) ('python-slm-p2-convert-'+[Guid]::NewGuid().ToString('N'))
    [void][IO.Directory]::CreateDirectory($backupRoot)
    try{
        Copy-Item -LiteralPath $evidencePath -Destination (Join-Path $backupRoot 'evidence.json') -ErrorAction Stop
        if(Test-Path -LiteralPath $sealPath){Copy-Item -LiteralPath $sealPath -Destination (Join-Path $backupRoot 'SHA256SUMS') -ErrorAction Stop}
        $summaryPath=Join-Path $RunRoot 'artifacts\failure-summary.json';$summaryExisted=Test-Path -LiteralPath $summaryPath
        if($summaryExisted){Copy-Item -LiteralPath $summaryPath -Destination (Join-Path $backupRoot 'failure-summary.json') -ErrorAction Stop}
        try{
            if(Test-Path -LiteralPath $sealPath){Remove-Item -LiteralPath $sealPath -Force -ErrorAction Stop}
            [void][IO.Directory]::CreateDirectory((Join-Path $RunRoot 'artifacts'))
            $summary=[ordered]@{schema='python-slm-p2-failure-summary-v1';phase_id='P2';run_id=[string]$evidence.run_id;errors=@($ErrorRecord)}
            if($summaryExisted){Write-P2JsonFile -Path $summaryPath -Value $summary}else{Write-P2JsonFile -Path $summaryPath -Value $summary -CreateNew}
            $evidence.status='FAIL';$evidence.errors=@($ErrorRecord);$evidence.finished_at=[DateTime]::UtcNow.ToString('o')
            Write-P2JsonFile -Path $evidencePath -Value $evidence
            $evidence.seal.entries=@(Get-ChildItem -LiteralPath $RunRoot -Recurse -File).Count
            Write-P2JsonFile -Path $evidencePath -Value $evidence
            [void](Test-P2ReceiptRedaction -RunRoot $RunRoot);[void](New-P2Seal -RunRoot $RunRoot)
            return $true
        }catch{
            Copy-Item -LiteralPath (Join-Path $backupRoot 'evidence.json') -Destination $evidencePath -Force
            if(Test-Path -LiteralPath $sealPath){Remove-Item -LiteralPath $sealPath -Force}
            if(Test-Path -LiteralPath (Join-Path $backupRoot 'SHA256SUMS')){Copy-Item -LiteralPath (Join-Path $backupRoot 'SHA256SUMS') -Destination $sealPath -Force}
            if($summaryExisted){Copy-Item -LiteralPath (Join-Path $backupRoot 'failure-summary.json') -Destination $summaryPath -Force}
            elseif(Test-Path -LiteralPath $summaryPath){Remove-Item -LiteralPath $summaryPath -Force}
            if(-not(Test-P2Seal -RunRoot $RunRoot)){throw 'late publication failure conversion failed and original PASS seal could not be restored'}
            throw
        }
    }finally{if(Test-Path -LiteralPath $backupRoot){Remove-Item -LiteralPath $backupRoot -Recurse -Force -ErrorAction SilentlyContinue}}
}

function Get-P2CandidateBuildCommands {
    param([Parameter(Mandatory)][string]$ExperimentRoot,
        [Parameter(Mandatory)][string]$CpuTarget, [Parameter(Mandatory)][string]$CudaTarget)
    $manifest = Join-Path $ExperimentRoot 'Cargo.toml'
    $burnCpu=Join-Path $CpuTarget 'burn-cubecl';$burnCuda=Join-Path $CudaTarget 'burn-cubecl'
    $candleCpu=Join-Path $CpuTarget 'candle';$candleCuda=Join-Path $CudaTarget 'candle'
    return @(
        [ordered]@{ candidate_id = 'burn-cubecl'; binary = 'p2-burn-cubecl'; package = 'p2-burn-cubecl'
            cpu_target=$burnCpu;cuda_target=$burnCuda
            cpu_args = @('build', '-vv', '--locked', '--offline', '--release', '--manifest-path', $manifest,
                '--package', 'p2-burn-cubecl', '--no-default-features', '--features', 'cpu', '--target-dir', $burnCpu)
            cuda_args = @('build', '-vv', '--locked', '--offline', '--release', '--manifest-path', $manifest,
                '--package', 'p2-burn-cubecl', '--no-default-features', '--features', 'cuda', '--target',
                'x86_64-pc-windows-msvc', '--target-dir', $burnCuda) },
        [ordered]@{ candidate_id = 'candle'; binary = 'p2-candle'; package = 'p2-candle'
            cpu_target=$candleCpu;cuda_target=$candleCuda
            cpu_args = @('build', '-vv', '--locked', '--offline', '--release', '--manifest-path', $manifest,
                '--package', 'p2-candle', '--no-default-features', '--features', 'cpu', '--target-dir', $candleCpu)
            cuda_args = @('build', '-vv', '--locked', '--offline', '--release', '--manifest-path', $manifest,
                '--package', 'p2-candle', '--no-default-features', '--features', 'cuda', '--target',
                'x86_64-pc-windows-msvc', '--target-dir', $candleCuda) }
    )
}

function New-P2ArtifactReference {
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][string]$RunRoot)
    return New-P2FileReference -Path $Path -RunRoot $RunRoot
}

function Invoke-P2Qualification {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$RepositoryRoot, [Parameter(Mandatory)][string]$OutputRoot)
    $repository = [IO.Path]::GetFullPath($RepositoryRoot)
    try {
        $output = Resolve-P2OutputRoot -RepositoryRoot $repository -OutputRoot $OutputRoot
    }
    catch {
        $exception = [ArgumentException]::new('P2 OutputRoot must resolve exactly to docs/receipts/P2.', $_.Exception)
        $exception.Data['P2Code'] = 'OUTPUT_ROOT_INVALID'
        $exception.Data['P2Category'] = 2
        throw $exception
    }
    $runsRoot=Join-Path $output 'runs';[void][IO.Directory]::CreateDirectory($runsRoot);Assert-P2OwnedDirectory $runsRoot $output
    $runId = New-P2RunId; $runRoot = Join-Path (Join-Path $output 'runs') $runId
    [void][IO.Directory]::CreateDirectory($runRoot);Assert-P2OwnedDirectory $runRoot $output
    [void][IO.Directory]::CreateDirectory((Join-Path $runRoot 'commands'));Assert-P2OwnedDirectory (Join-Path $runRoot 'commands') $runRoot
    [void][IO.Directory]::CreateDirectory((Join-Path $runRoot 'artifacts'));Assert-P2OwnedDirectory (Join-Path $runRoot 'artifacts') $runRoot
    $started = [DateTime]::UtcNow; $watch = [Diagnostics.Stopwatch]::StartNew()
    $commands = [Collections.Generic.List[object]]::new(); $errors = [Collections.Generic.List[object]]::new()
    $gates = New-P2GateSet; $p1b = $null; $policy = New-P2Policy; $dependencyInventory = $null
    $fixtureManifest = $null; $hostState = $null; $cpuIsolation = $null; $comparisonRef = $null; $decisionRef = $null
    $candidateRefs = [Collections.Generic.List[object]]::new();$healthRecoveries=[Collections.Generic.List[object]]::new()
    $monitorChecks=[Collections.Generic.List[object]]::new(); $temporaryRoot = $null
    $cleanupAttempted = $false; $temporaryRemoved = $false; $treesTerminated = $true;$script:P2TreeViolation=$false
    $source = $null; $sourceHash = '0' * 64; $sourceInputFingerprint = $null
    $sourcePath = Join-Path $runRoot 'artifacts\source-identity.json'
    $parentEnvironmentBefore=Get-P2EnvironmentFingerprint
    try {
        Initialize-P2NativeInterop
        $source = Get-P2RepositoryIdentity -RepositoryRoot $repository
        $sourceInputFingerprint = $source.fingerprint
        Write-P2JsonFile -Path $sourcePath -Value $source.value -CreateNew
        $sourceHash = Get-P2Sha256 -Path $sourcePath
        if ([int]$source.value.worktree_status_entries -ne 0) {
            $exception = [InvalidOperationException]::new('P2 requires the latest committed clean harness inputs')
            $exception.Data['P2Code'] = 'HARNESS_NOT_COMMITTED'; $exception.Data['P2Category'] = 2; throw $exception
        }
        $policyPath = Join-Path $runRoot 'artifacts\qualification-policy.json'
        Write-P2JsonFile -Path $policyPath -Value $policy -CreateNew
        $schemaBundle = New-P2SchemaBundleIdentity -RepositoryRoot $repository
        $p1b = Get-P2SelectedP1BDependency -RepositoryRoot $repository
        Set-P2Gate $gates p1b_dependency PASS 'Selected P1B chain, nested P1A/P0 chain, and review closure commit validated.'
        $p1bPath = Join-Path $runRoot 'artifacts\p1b-dependency.json'
        Write-P2JsonFile -Path $p1bPath -Value $p1b -CreateNew
        $temporaryBase = [IO.Path]::GetTempPath(); $temporaryRoot = Join-Path $temporaryBase ("python-slm-p2-$runId")
        if (Test-Path -LiteralPath $temporaryRoot) { throw 'owned P2 temporary root unexpectedly exists' }
        [void][IO.Directory]::CreateDirectory($temporaryRoot)
        if (((Get-Item -Force -LiteralPath $temporaryRoot).Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw 'owned P2 temporary root is a reparse point'
        }
        $experiment = Join-Path $repository 'experiments\p2-backends'; $manifest = Join-Path $experiment 'Cargo.toml'
        $lock = Join-Path $experiment 'Cargo.lock'; $dependencyInventory = Test-P2DependencyPolicy -ManifestPath $manifest -LockPath $lock
        $dependencyInventory = [ordered]@{
            schema = 'python-slm-p2-dependency-inventory-v1'; status = 'PASS'
            cargo_lock_sha256 = $dependencyInventory.cargo_lock_sha256
            locked_dependency_count = $dependencyInventory.locked_dependency_count
            hydration = $dependencyInventory.hydration
            git_dependencies = 0; alternate_registries = 0; patches = 0
            schema_bundle_sha256 = $schemaBundle.bundle_sha256; schema_files = $schemaBundle.files
        }
        $dependencyPath = Join-Path $runRoot 'artifacts\dependency-inventory.json'
        Write-P2JsonFile -Path $dependencyPath -Value $dependencyInventory -CreateNew
        Set-P2Gate $gates dependency_policy PASS 'Exact pins, lock checksums, registry boundary, and seven-schema bundle validated.'
        $cudaRoot = [IO.Path]::GetFullPath($env:CUDA_PATH)
        $qualifiedHost = Assert-P2LiveP1BEnvironment -RepositoryRoot $repository -Dependency $p1b -CudaToolkitRoot $cudaRoot
        $script:P2ExpectedModuleHashes=@{}
        $script:P2ExpectedModuleHashes[[string]$qualifiedHost.manifest.driver.library.path]=[string]$qualifiedHost.manifest.driver.library.sha256
        foreach($dll in @($qualifiedHost.manifest.cuda_toolkit.runtime_dlls)){$script:P2ExpectedModuleHashes[[string]$dll.path]=[string]$dll.sha256}
        $nvmlLibrary=New-P2NvmlLibraryRecord -Path ([P2NvmlMonitor]::EnsureQualifiedLibrary()) -WindowsRoot $env:SystemRoot
        $contention = [P2NvmlMonitor]::ActiveGpuProcessCount(0)
        $hostState = [ordered]@{
            schema = 'python-slm-p2-host-state-v1'; status = 'PASS'; gpu = $qualifiedHost.gpu
            compute_capability = $qualifiedHost.compute_capability; driver_version = $qualifiedHost.driver_version
            cuda_toolkit_version = $qualifiedHost.cuda_toolkit_version; foreign_gpu_process_count = $contention
            nvml_library=$nvmlLibrary;cuda_health_recoveries=@();benchmark_monitor_checks=@()
        }
        $hostPath = Join-Path $runRoot 'artifacts\host-state.json'; Write-P2JsonFile -Path $hostPath -Value $hostState -CreateNew
        if ($contention -ne 0) {
            $exception = [InvalidOperationException]::new("GPU contention detected: $contention foreign GPU processes")
            $exception.Data['P2Code'] = 'GPU_CONTENTION'; $exception.Data['P2Category'] = 4; throw $exception
        }
        $health = [P2CudaHealth]::Probe(); if ($health -cne 'PASS') { throw "CUDA health probe failed: $health" }
        $build = Get-P2BuildEnvironment -TemporaryRoot $temporaryRoot
        $script:P2TranscriptRoleRoots=@{VS_INSTALL=$build.vs_install;VC_TOOLS=[string]$build.environment['VCToolsInstallDir'];WINDOWS_KITS=[string]$build.environment['WindowsSdkDir'];CARGO_HOME=[string]$build.environment['CARGO_HOME'];RUSTUP_HOME=[string]$build.environment['RUSTUP_HOME']}
        [void](Assert-P2BuildEnvironmentMatch -Environment $build.environment -QualifiedManifest $qualifiedHost.manifest)
        $environment = New-P2IsolatedEnvironment -DeveloperEnvironment $build.environment `
            -TemporaryRoot $temporaryRoot -VsRoot $build.vs_install -CudaToolkitRoot $cudaRoot
        [void](Assert-P2CargoConfigurationSafe -RepositoryRoot $repository -CargoHome ([string]$environment['CARGO_HOME']))
        $cl = @(([string]$build.environment['Path']).Split(';') | ForEach-Object { Join-Path $_ 'cl.exe' } |
            Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -Unique)
        if($cl.Count-ne1){throw 'qualified cl.exe resolution is missing or ambiguous'}
        $pythonCanaryRoot = Join-Path $temporaryRoot 'python-canaries'; [void][IO.Directory]::CreateDirectory($pythonCanaryRoot)
        New-P2ExecutableCanaries -Root $pythonCanaryRoot -ClPath $cl[0] -Environment $build.environment `
            -Names @('python','python3','python3.13','python313','pythonw','pythonw3','pythonw313','py','py3','pip','pip3','pip3.13')
        $environment['Path'] = $pythonCanaryRoot + ';' + [string]$environment['Path']
        $cpuCanaryRoot = Join-Path $temporaryRoot 'cpu-canaries'; [void][IO.Directory]::CreateDirectory($cpuCanaryRoot)
        New-P2ExecutableCanaries -Root $cpuCanaryRoot -ClPath $cl[0] -Environment $build.environment `
            -Names @('nvcc','ptxas','fatbinary','nvlink')
        $cpuEnvironment = @{}; foreach ($key in $environment.Keys) { $cpuEnvironment[$key] = $environment[$key] }
        $cpuPath=@(([string]$environment['Path']).Split(';')|Where-Object{-not(Test-P2PathWithin -Path $_ -Root $cudaRoot)})
        $cpuEnvironment['Path'] = $cpuCanaryRoot + ';' + ($cpuPath -join ';')
        foreach ($name in @('CUDA_PATH','CUDA_HOME','CUDA_ROOT','CUDA_TOOLKIT_ROOT_DIR','CUDNN_PATH','CUDNN_ROOT','NVCC','NVCC_PREPEND_FLAGS','NVCC_APPEND_FLAGS')) {
            $cpuEnvironment[$name] = $null
        }
        $cpuTarget = Join-Path $temporaryRoot 'cpu-target'; $cudaTarget = Join-Path $temporaryRoot 'cuda-target'
        $fixtureRoot = Join-Path $temporaryRoot 'fixtures'; [void][IO.Directory]::CreateDirectory($fixtureRoot)
        $cargo = (Get-Command cargo.exe -ErrorAction Stop).Source; $commandNumber = 1
        $existingTarget = Join-Path $repository 'target'; $existingTargetBefore = Get-P2DirectoryFingerprint $existingTarget
        $rootTarget = Join-Path $temporaryRoot 'root-target'
        $rootEnvironment = @{}; foreach ($key in $cpuEnvironment.Keys) { $rootEnvironment[$key] = $cpuEnvironment[$key] }
        $rootEnvironment['CARGO_TARGET_DIR'] = $rootTarget
        foreach ($gateArgs in @(
                @('fmt','--all','--','--check'),
                @('clippy','--locked','--all-targets','--features','cpu-reference','--','-D','warnings'),
                @('test','--locked','--features','cpu-reference'))) {
            $id='C'+$commandNumber.ToString('00');$commandNumber++
            $gateCommand=Invoke-P2RecordedCommand -Id $id -FilePath $cargo -ArgumentList $gateArgs `
                -WorkingDirectory $repository -RunRoot $runRoot -RepositoryRoot $repository -Environment $rootEnvironment `
                -ExpectedExitCodes @(0) -TimeoutSeconds 1800 -RoleRoots @{TEMP=$temporaryRoot;CUDA_TOOLKIT=$cudaRoot}
            $commands.Add($gateCommand.record);if(-not $gateCommand.record.expectation_met){throw 'root CPU quality gate failed'}
        }
        $buildSpecs = Get-P2CandidateBuildCommands -ExperimentRoot $experiment -CpuTarget $cpuTarget -CudaTarget $cudaTarget
        $buildState = @{}; $cpuAuditCommands = [Collections.Generic.List[object]]::new()
        $activatedGraphs = [ordered]@{}
        foreach ($spec in $buildSpecs) {
            $state = [pscustomobject]@{ cpu_built=$false;cuda_built=$false;failures=[Collections.Generic.List[object]]::new() }
            foreach ($kind in @('cpu', 'cuda')) {
                $id = 'C' + $commandNumber.ToString('00'); $commandNumber++
                $args = if ($kind -ceq 'cpu') { $spec.cpu_args } else { $spec.cuda_args }
                $childEnvironment = if ($kind -ceq 'cpu') { $cpuEnvironment } else { $environment }
                $recorded = Invoke-P2RecordedCommand -Id $id -FilePath $cargo -ArgumentList $args `
                    -WorkingDirectory $repository -RunRoot $runRoot -RepositoryRoot $repository -Environment $childEnvironment `
                    -ExpectedExitCodes @(0,101) -TimeoutSeconds 1800 -RoleRoots @{ TEMP = $temporaryRoot; CUDA_TOOLKIT = $cudaRoot }
                $commands.Add($recorded.record); if ($kind -ceq 'cpu') { $cpuAuditCommands.Add($recorded) }
                if ($recorded.record.status -ceq 'PASS') { $state.($kind + '_built') = $true }
                else {
                    $state.failures.Add((Get-P2CandidateFailure -Message "$($spec.candidate_id) $kind build failed" `
                            -CommandId $id -Code (($kind.ToUpperInvariant()) + '_BUILD_FAILED')))
                    if([string]$recorded.record.status-in@('TIMEOUT','OOM','CRASH')){
                        $postBuildHealth=[P2CudaHealth]::Probe()
                        if($postBuildHealth-cne'PASS'){throw "CUDA unhealthy after exceptional candidate build outcome: $postBuildHealth"}
                        $state.failures.Add((Get-P2CandidateFailure -Message 'Fresh CUDA health probe passed after exceptional candidate build outcome.' `
                            -CommandId $id -Code 'CUDA_HEALTH_RECOVERY_PASSED'))
                        $healthRecoveries.Add([ordered]@{candidate_id=$spec.candidate_id;failed_command_id=$id;trigger='candidate_build_process_failure';result='PASS'})
                    }
                }
            }
            if ($state.cuda_built) {
                $id='C'+$commandNumber.ToString('00');$commandNumber++
                try {
                    $graph=Invoke-P2ActivatedGraph -CandidateId $spec.candidate_id -Package $spec.package -Feature cuda `
                        -Manifest $manifest -Cargo $cargo -CommandId $id -RunRoot $runRoot -RepositoryRoot $repository `
                        -Environment $environment -TemporaryRoot $temporaryRoot -CudaToolkitRoot $cudaRoot
                    $commands.Add($graph.command.record);$commands.Add($graph.package_command.record);$activatedGraphs[$spec.candidate_id]=$graph.inventory
                } catch {
                    if ($_.Exception.Data.Contains('P2CommandRecords')){foreach($record in @($_.Exception.Data['P2CommandRecords'])){$commands.Add($record)
                            if([string]$record.status-cne'PASS'){$state.failures.Add((Get-P2CandidateFailure -Message 'Activated graph subcommand failed.' -CommandId ([string]$record.id) -Code 'ACTIVATED_GRAPH_FAILED'))}}}
                    elseif ($_.Exception.Data.Contains('P2CommandRecord')){$commands.Add($_.Exception.Data['P2CommandRecord'])}
                    $state.cuda_built=$false;$state.failures.Add((Get-P2CandidateFailure -Message ([string]$_.Exception.Message) -CommandId $id -Code 'ACTIVATED_GRAPH_FAILED'))
                }
            }
            if ($state.cpu_built) {
                $id='C'+$commandNumber.ToString('00');$commandNumber++
                $cpuGraph=Invoke-P2RecordedCommand -Id $id -FilePath $cargo -ArgumentList @('tree','--locked','--offline',
                    '--manifest-path',$manifest,'--package',$spec.package,'--no-default-features','--features','cpu',
                    '--target','x86_64-pc-windows-msvc','-e','features','--prefix','none','--format','{p}|{f}') `
                    -WorkingDirectory $repository -RunRoot $runRoot -RepositoryRoot $repository -Environment $cpuEnvironment `
                    -ExpectedExitCodes @(0) -TimeoutSeconds 300 -RoleRoots @{TEMP=$temporaryRoot;CUDA_TOOLKIT=$cudaRoot}
                $commands.Add($cpuGraph.record);$cpuAuditCommands.Add($cpuGraph)
                if (-not $cpuGraph.record.expectation_met -or ([string]$cpuGraph.raw_stdout -match '(?i)(cuda|cudnn|cublas|cudarc|cubecl-cuda)')) {
                    $state.cpu_built=$false;$state.failures.Add((Get-P2CandidateFailure -Message 'CPU activated graph contains CUDA or failed' -CommandId $id -Code 'CPU_GRAPH_ISOLATION_FAILED'))
                }
            }
            $buildState[$spec.candidate_id]=$state
        }
        $dependencyInventory['candidate_graphs'] = $activatedGraphs
        Write-P2JsonFile -Path $dependencyPath -Value $dependencyInventory
        $fixtureId = 'C' + $commandNumber.ToString('00'); $commandNumber++
        $fixtureTarget=Join-Path $temporaryRoot 'fixture-target'
        # Common is not transitively built as a binary by candidate packages.
        $commonBuild = Invoke-P2RecordedCommand -Id $fixtureId -FilePath $cargo -ArgumentList @(
            'run', '--locked', '--offline', '--release', '--manifest-path', $manifest, '--package',
            'p2-backend-common', '--bin', 'p2-fixtures', '--target-dir', $fixtureTarget, '--', '--output', $fixtureRoot) `
            -WorkingDirectory $repository -RunRoot $runRoot -RepositoryRoot $repository -Environment $cpuEnvironment `
            -ExpectedExitCodes @(0) -TimeoutSeconds 1800 -RoleRoots @{ TEMP = $temporaryRoot; CUDA_TOOLKIT = $cudaRoot }
        $commands.Add($commonBuild.record); if (-not $commonBuild.record.expectation_met) { throw 'fixture generation failed' }
        $manifests = @(Assert-P2FixtureManifestSet -FixtureRoot $fixtureRoot)
        $fixtureManifest = [ordered]@{ schema = 'python-slm-p2-fixture-manifest-v1'; status = 'PASS'; manifests = $manifests }
        $fixtureArtifact = Join-Path $runRoot 'artifacts\fixture-manifest.json'; Write-P2JsonFile -Path $fixtureArtifact -Value $fixtureManifest -CreateNew
        Set-P2Gate $gates fixture_determinism PASS 'Four domain-separated BF16 fixture manifests were generated and hashed.'
        $cpuAuditCommands.Add($commonBuild)
        $dumpbin = @(([string]$build.environment['Path']).Split(';') | ForEach-Object { Join-Path $_ 'dumpbin.exe' } |
            Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -Unique)
        if ($dumpbin.Count -ne 1) { throw 'qualified dumpbin.exe resolution is missing or ambiguous' }
        foreach ($spec in $buildSpecs) {
            if (-not $buildState[$spec.candidate_id].cpu_built) { continue }
            $cpuExe = Join-Path $spec.cpu_target ("release\$($spec.binary).exe")
            $id='C'+$commandNumber.ToString('00');$commandNumber++
            $pe=Invoke-P2RecordedCommand -Id $id -FilePath $dumpbin[0] -ArgumentList @('/HEADERS','/DEPENDENTS',$cpuExe) `
                -WorkingDirectory $repository -RunRoot $runRoot -RepositoryRoot $repository -Environment $cpuEnvironment `
                -ExpectedExitCodes @(0) -TimeoutSeconds 120 -RoleRoots @{TEMP=$temporaryRoot;CUDA_TOOLKIT=$cudaRoot}
            $commands.Add($pe.record);$cpuAuditCommands.Add($pe)
            if (-not $pe.record.expectation_met -or [string]$pe.raw_stdout -notmatch '(?i)machine \(x64\)' -or
                [string]$pe.raw_stdout -match '(?im)^\s*(?:(?:nvcuda|cudart|cublas(?:lt)?|cudnn|nvrtc|nvjitlink|nccl)[^\s]*|(?:lib)?python(?:3(?:\d+)?)?)\.dll\s*$') {
                $buildState[$spec.candidate_id].cpu_built=$false
                $buildState[$spec.candidate_id].failures.Add((Get-P2CandidateFailure -Message 'CPU PE audit failed' -CommandId $id -Code 'CPU_PE_AUDIT_FAILED'))
            }
            if($buildState[$spec.candidate_id].cuda_built){
                $cudaExe=Join-Path $spec.cuda_target ("x86_64-pc-windows-msvc\release\$($spec.binary).exe")
                $id='C'+$commandNumber.ToString('00');$commandNumber++
                $cudaPe=Invoke-P2RecordedCommand -Id $id -FilePath $dumpbin[0] -ArgumentList @('/HEADERS','/DEPENDENTS',$cudaExe) `
                    -WorkingDirectory $repository -RunRoot $runRoot -RepositoryRoot $repository -Environment $environment `
                    -ExpectedExitCodes @(0) -TimeoutSeconds 120 -RoleRoots @{TEMP=$temporaryRoot;CUDA_TOOLKIT=$cudaRoot}
                $commands.Add($cudaPe.record)
                if(-not$cudaPe.record.expectation_met-or[string]$cudaPe.raw_stdout-notmatch'(?i)machine \(x64\)'-or
                    [string]$cudaPe.raw_stdout-match'(?im)^\s*(?:(?:lib)?python(?:3(?:\d+)?)?|cudnn[^\s]*|nccl[^\s]*)\.dll\s*$'){
                    $buildState[$spec.candidate_id].cuda_built=$false
                    $buildState[$spec.candidate_id].failures.Add((Get-P2CandidateFailure -Message 'CUDA PE audit failed' -CommandId $id -Code 'CUDA_PE_AUDIT_FAILED'))
                }
            }
        }
        $targetFiles = @((Get-ChildItem -LiteralPath $cpuTarget -Recurse -File | ForEach-Object FullName)+
            (Get-ChildItem -LiteralPath $fixtureTarget -Recurse -File | ForEach-Object FullName))
        $cpuIsolation = Test-P2CpuIsolationEvidence -CommandResults @($cpuAuditCommands) -TargetFiles $targetFiles
        $cpuIsolationPath = Join-Path $runRoot 'artifacts\cpu-isolation.json'; Write-P2JsonFile -Path $cpuIsolationPath -Value $cpuIsolation -CreateNew
        if ([string]$cpuIsolation.status -cne 'PASS') { throw 'CPU experiment build discovered CUDA or Python' }
        Set-P2Gate $gates cpu_isolation PASS 'CPU targets contained no CUDA/Python invocation, artifact, or link evidence.'
        $resultsRoot = Join-Path $runRoot 'candidate-results'; [void][IO.Directory]::CreateDirectory($resultsRoot)
        $preflight = @{}
        foreach ($spec in $buildSpecs) {
            $state=$buildState[$spec.candidate_id]
            if (-not $state.cpu_built -or -not $state.cuda_built) {
                $preflight[$spec.candidate_id]=[pscustomobject]@{cpu=$null;allocation=$null;correctness=$null;commands=@();provenance=@();failures=@($state.failures);recoveries=@();eligible=$false}
                continue
            }
            $cpuExe = Join-Path $spec.cpu_target ("release\$($spec.binary).exe")
            $cudaExe = Join-Path $spec.cuda_target ("x86_64-pc-windows-msvc\release\$($spec.binary).exe")
            $preflight[$spec.candidate_id] = Invoke-P2FrameworkPreflight -CandidateId $spec.candidate_id `
                -CpuExecutable $cpuExe -CudaExecutable $cudaExe -FixtureRoot $fixtureRoot -ResultRoot $resultsRoot `
                -RunRoot $runRoot -RepositoryRoot $repository -WorkingDirectory $repository -Environment $environment `
                -CpuEnvironment $cpuEnvironment `
                -CudaToolkitRoot $cudaRoot -TemporaryRoot $temporaryRoot -CommandNumber ([ref]$commandNumber)
            $preflight[$spec.candidate_id].failures=@($state.failures)+@($preflight[$spec.candidate_id].failures)
            foreach($recovery in @($preflight[$spec.candidate_id].recoveries)){$healthRecoveries.Add($recovery)}
            foreach ($record in @($preflight[$spec.candidate_id].commands)) { $commands.Add($record) }
        }
        $roundValues = [Collections.Generic.List[object]]::new()
        foreach ($workload in @('projection', 'ffn-expansion')) {
            foreach ($roundSpec in @(@('burn-cubecl', 1), @('candle', 2), @('candle', 3), @('burn-cubecl', 4))) {
                $candidateId = $roundSpec[0]; if (-not $preflight[$candidateId].eligible) { continue }
                $spec = @($buildSpecs | Where-Object candidate_id -eq $candidateId)[0]
                $cudaExe = Join-Path $spec.cuda_target ("x86_64-pc-windows-msvc\release\$($spec.binary).exe")
                $id = 'C' + $commandNumber.ToString('00'); $commandNumber++
                try {
                    $value = Invoke-P2CandidateInvocation -CandidateId $candidateId -Mode benchmark -Workload $workload `
                        -Round ([Nullable[int]]$roundSpec[1]) -Executable $cudaExe -FixtureRoot $fixtureRoot `
                        -ResultRoot $resultsRoot -CommandId $id -RunRoot $runRoot -RepositoryRoot $repository `
                        -WorkingDirectory $repository -Environment $environment -CudaToolkitRoot $cudaRoot `
                        -TemporaryRoot $temporaryRoot -MonitorNvml
                    $commands.Add($value.command)
                    # Monitor policy is run-global.  Validate it even when the
                    # framework reports FAIL so a peer cannot mask contention or
                    # an invalid sampling window.
                    $measurement=Get-P2RunNvmlMeasurement -Invocation $value -Round ([int]$roundSpec[1]) `
                        -Workload $workload -CandidateId $candidateId
                    $value|Add-Member -NotePropertyName measurement -NotePropertyValue $measurement
                    $monitorChecks.Add([ordered]@{candidate_id=$candidateId;measurement=$measurement})
                    $roundValues.Add([pscustomobject]@{
                            candidate_id = $candidateId; round = [int]$roundSpec[1]; workload = $workload; value = $value })
                    if ([string]$value.result.status -cne 'PASS') {
                        $preflight[$candidateId].failures += @(Get-P2CandidateFailure `
                                -Message "$candidateId benchmark/$workload returned FAIL" -CommandId $id -Code 'BENCHMARK_RESULT_FAILED')
                        $postHealth = [P2CudaHealth]::Probe()
                        if ($postHealth -cne 'PASS') { throw "CUDA unhealthy after benchmark FAIL: $postHealth" }
                        $preflight[$candidateId].failures += @(Get-P2CandidateFailure -Message 'Fresh CUDA health probe passed after benchmark result failure.' `
                            -CommandId $id -Code 'CUDA_HEALTH_RECOVERY_PASSED')
                        $healthRecoveries.Add([ordered]@{candidate_id=$candidateId;failed_command_id=$id;trigger='benchmark_result_fail';result='PASS'})
                    }
                }
                catch {
                    if ($_.Exception.Data.Contains('P2Code') -and
                        [string]$_.Exception.Data['P2Code'] -ceq 'MEASUREMENT_INVALID') { throw }
                    if ($_.Exception.Data.Contains('P2CommandRecord')) { $commands.Add($_.Exception.Data['P2CommandRecord']) }
                    if ($_.Exception.Data.Contains('P2Nvml')) {
                        $failedInvocation=[pscustomobject]@{
                            command=[pscustomobject]@{id=$id};nvml=$_.Exception.Data['P2Nvml']
                        }
                        $failedMeasurement=Get-P2RunNvmlMeasurement -Invocation $failedInvocation -Round ([int]$roundSpec[1]) `
                            -Workload $workload -CandidateId $candidateId
                        $monitorChecks.Add([ordered]@{candidate_id=$candidateId;measurement=$failedMeasurement})
                    }
                    $preflight[$candidateId].failures += @(Get-P2CandidateFailure -Message ([string]$_.Exception.Message) `
                            -CommandId $id -Code 'BENCHMARK_FAILED')
                    $postHealth = [P2CudaHealth]::Probe(); if ($postHealth -cne 'PASS') { throw "CUDA unhealthy after benchmark failure: $postHealth" }
                    $preflight[$candidateId].failures += @(Get-P2CandidateFailure -Message 'Fresh CUDA health probe passed after benchmark process failure.' `
                        -CommandId $id -Code 'CUDA_HEALTH_RECOVERY_PASSED')
                    $healthRecoveries.Add([ordered]@{candidate_id=$candidateId;failed_command_id=$id;trigger='benchmark_process_failure';result='PASS'})
                }
            }
        }
        $protocolRows = @($roundValues | ForEach-Object { [pscustomobject]@{ candidate_id = $_.candidate_id; round = $_.round; workload = $_.workload } })
        if ($protocolRows.Count -eq 8) { [void](Assert-P2AbbaOrder -Rounds $protocolRows) }
        $aggregates = [Collections.Generic.List[object]]::new()
        foreach ($spec in $buildSpecs) {
            $candidateId = $spec.candidate_id; $state = $preflight[$candidateId]
            $candidateRows=@($roundValues|Where-Object candidate_id -eq $candidateId|Sort-Object -Property @(
                    @{Expression={[int]$_.round}},@{Expression={if($_.workload-ceq'projection'){0}else{1}}}))
            $candidateRounds = @($candidateRows | ForEach-Object { $_.value })
            $allProv = @($state.provenance) + @($candidateRounds.runtime_provenance); $runtime = Merge-P2RuntimeProvenance -Records $allProv
            if (-not [bool]$runtime.all_allowed) {
                $state.failures += @(Get-P2CandidateFailure -Message "$candidateId loaded an unqualified CUDA runtime boundary" `
                        -CommandId $null -Code 'RUNTIME_PROVENANCE_FAILED' -Category 3)
            }
            $summary = $null; $nvmlMeasurements = [Collections.Generic.List[object]]::new()
            foreach($roundValue in $candidateRows){$nvmlMeasurements.Add($roundValue.value.measurement)}
            if ($candidateRounds.Count -eq 4 -and @($candidateRounds.result | Where-Object status -ne PASS).Count -eq 0) {
                try {
                    $dependencyCount=[int]$activatedGraphs[$candidateId].package_count
                    $observed=[int64](($nvmlMeasurements | Measure-Object delta_bytes -Maximum).Maximum)
                    $cmp = Get-P2CandidateComparison -CandidateId $candidateId -BenchmarkResults @($candidateRounds.result) `
                        -LockedDependencyCount $dependencyCount -ObservedPeakBytes $observed
                    $summary = [ordered]@{
                        geomean_fwbw_p50_ns = $cmp.comparison.geomean_fwbw_p50_ns; geomean_fwbw_p95_ns = $cmp.comparison.geomean_fwbw_p95_ns
                        observed_peak_bytes = $cmp.comparison.observed_peak_bytes; locked_dependency_count = $cmp.comparison.locked_dependency_count
                        max_nvml_gap_ms = [double](($nvmlMeasurements | Measure-Object max_gap_ms -Maximum).Maximum)
                        foreign_process_count = [int](($nvmlMeasurements | Measure-Object maximum_foreign_process_count -Maximum).Maximum)
                    }
                } catch {
                    # Foreign GPU work, a sampling gap, malformed monitor evidence, or
                    # between-round drift invalidates the shared measurement run.  It is
                    # not a framework-local failure that the peer can mask.
                    Stop-P2RunForInvalidMeasurement -CandidateId $candidateId -Cause $_.Exception
                }
            }
            $cpuRef=$(if($null -ne $state.cpu){$state.cpu.reference}else{$null})
            $allocationRef=$(if($null -ne $state.allocation){$state.allocation.reference}else{$null})
            $correctnessRef=$(if($null -ne $state.correctness){$state.correctness.reference}else{$null})
            $aggregate = New-P2CandidateAggregate -CandidateId $candidateId -CpuSmoke $cpuRef `
                -Allocation $allocationRef -Correctness $correctnessRef -BenchmarkRounds @($candidateRounds.reference) `
                -NvmlMeasurements @($nvmlMeasurements) -Summary $summary -RuntimeProvenance $runtime -Failures @($state.failures)
            $null = Assert-P2CandidateAggregate -Aggregate $aggregate -RunRoot $runRoot
            $aggregates.Add($aggregate)
            $candidateRefs.Add((New-P2CandidateArtifactReference -CandidateId $candidateId -Role framework -Aggregate $aggregate `
                    -Path (Join-Path $runRoot "artifacts\$candidateId.json") -RunRoot $runRoot))
        }
        $frameworksPassed=@($aggregates|Where-Object { $_.role -ceq 'framework' -and $_.status -ceq 'PASS' }).Count
        if($frameworksPassed -eq 0){
            $fallbackFailures=[Collections.Generic.List[object]]::new();$fallbackResults=[Collections.Generic.List[object]]::new();$fallbackProv=[Collections.Generic.List[object]]::new()
            $fallbackTarget=Join-Path $temporaryRoot 'fallback-target';$id='C'+$commandNumber.ToString('00');$commandNumber++
            $fallbackBuild=Invoke-P2RecordedCommand -Id $id -FilePath $cargo -ArgumentList @('build','--locked','--offline','--release',
                '--manifest-path',$manifest,'--package','p2-cudarc-fallback','--no-default-features','--features','cuda',
                '--target','x86_64-pc-windows-msvc','--target-dir',$fallbackTarget) -WorkingDirectory $repository `
                -RunRoot $runRoot -RepositoryRoot $repository -Environment $environment -ExpectedExitCodes @(0,101) `
                -TimeoutSeconds 1800 -RoleRoots @{TEMP=$temporaryRoot;CUDA_TOOLKIT=$cudaRoot}
            $commands.Add($fallbackBuild.record)
            if($fallbackBuild.record.status -ne 'PASS'){$fallbackFailures.Add((Get-P2CandidateFailure -Message 'cudarc diagnostic build failed' -CommandId $id -Code 'CUDA_BUILD_FAILED'))}
            else{
                $id='C'+$commandNumber.ToString('00');$commandNumber++
                try{$graph=Invoke-P2ActivatedGraph -CandidateId cudarc-fallback -Package p2-cudarc-fallback -Feature cuda -Manifest $manifest `
                    -Cargo $cargo -CommandId $id -RunRoot $runRoot -RepositoryRoot $repository -Environment $environment `
                    -TemporaryRoot $temporaryRoot -CudaToolkitRoot $cudaRoot;$commands.Add($graph.command.record);$commands.Add($graph.package_command.record);$activatedGraphs['cudarc-fallback']=$graph.inventory}
                catch{if($_.Exception.Data.Contains('P2CommandRecords')){foreach($record in @($_.Exception.Data['P2CommandRecords'])){$commands.Add($record)
                            if([string]$record.status-cne'PASS'){$fallbackFailures.Add((Get-P2CandidateFailure -Message 'Activated fallback graph subcommand failed.' -CommandId ([string]$record.id) -Code 'ACTIVATED_GRAPH_FAILED'))}}}
                    elseif($_.Exception.Data.Contains('P2CommandRecord')){$commands.Add($_.Exception.Data['P2CommandRecord'])}
                    $fallbackFailures.Add((Get-P2CandidateFailure -Message ([string]$_.Exception.Message) -CommandId $id -Code 'ACTIVATED_GRAPH_FAILED'))}
                $fallbackExe=Join-Path $fallbackTarget 'x86_64-pc-windows-msvc\release\p2-cudarc-fallback.exe'
                foreach($workload in @('allocation','correctness')){
                    $id='C'+$commandNumber.ToString('00');$commandNumber++
                    try{$value=Invoke-P2CandidateInvocation -CandidateId cudarc-fallback -Mode correctness -Workload $workload -Round $null `
                        -Executable $fallbackExe -FixtureRoot $fixtureRoot -ResultRoot $resultsRoot -CommandId $id -RunRoot $runRoot `
                        -RepositoryRoot $repository -WorkingDirectory $repository -Environment $environment -CudaToolkitRoot $cudaRoot `
                        -TemporaryRoot $temporaryRoot;$commands.Add($value.command);$fallbackResults.Add($value);$fallbackProv.Add($value.runtime_provenance)
                        if($value.result.status -ne 'PASS'){
                            $fallbackFailures.Add((Get-P2CandidateFailure -Message "cudarc diagnostic $workload failed" -CommandId $id))
                            $health=[P2CudaHealth]::Probe();if($health-cne'PASS'){throw "CUDA unhealthy after cudarc diagnostic FAIL: $health"}
                            $fallbackFailures.Add((Get-P2CandidateFailure -Message 'Fresh CUDA health probe passed after fallback result failure.' -CommandId $id -Code 'CUDA_HEALTH_RECOVERY_PASSED'))
                            $healthRecoveries.Add([ordered]@{candidate_id='cudarc-fallback';failed_command_id=$id;trigger='fallback_result_fail';result='PASS'})
                        }}
                    catch{
                        if($_.Exception.Data.Contains('P2CommandRecord')){$commands.Add($_.Exception.Data['P2CommandRecord'])}
                        $fallbackFailures.Add((Get-P2CandidateFailure -Message ([string]$_.Exception.Message) -CommandId $id))
                        $health=[P2CudaHealth]::Probe();if($health-cne'PASS'){throw "CUDA unhealthy after cudarc diagnostic failure: $health"}
                        $fallbackFailures.Add((Get-P2CandidateFailure -Message 'Fresh CUDA health probe passed after fallback process failure.' -CommandId $id -Code 'CUDA_HEALTH_RECOVERY_PASSED'))
                        $healthRecoveries.Add([ordered]@{candidate_id='cudarc-fallback';failed_command_id=$id;trigger='fallback_process_failure';result='PASS'})
                    }
                }
            }
            $fallbackRuntime=Merge-P2RuntimeProvenance -Records @($fallbackProv)
            $fallback=[pscustomobject][ordered]@{schema=$script:P2Schema.Aggregate;candidate_id='cudarc-fallback';role='diagnostic_fallback'
                status=$(if($fallbackFailures.Count -eq 0 -and $fallbackResults.Count -eq 2){'PASS'}else{'FAIL'});cpu_smoke=$null
                allocation=$(if($fallbackResults.Count -gt 0){$fallbackResults[0].reference}else{$null})
                correctness=$(if($fallbackResults.Count -gt 1){$fallbackResults[1].reference}else{$null})
                benchmark_rounds=@();nvml_measurements=@();summary=$null;runtime_provenance=$fallbackRuntime;failures=@($fallbackFailures)}
            $null=Assert-P2CandidateAggregate -Aggregate $fallback -RunRoot $runRoot
            $fallbackRef=New-P2CandidateArtifactReference -CandidateId cudarc-fallback -Role diagnostic_fallback -Aggregate $fallback `
                -Path (Join-Path $runRoot 'artifacts\cudarc-fallback.json') -RunRoot $runRoot
            $fallbackDecision=[ordered]@{status=$fallback.status;reason='Both automatic-differentiation frameworks failed; bounded cudarc diagnostics cannot satisfy P2.'}
        }else{
            $fallback=New-P2NotRunAggregate -CandidateId cudarc-fallback -Role diagnostic_fallback
            $fallbackRef=[pscustomobject][ordered]@{candidate_id='cudarc-fallback';role='diagnostic_fallback';status='NOT_RUN';path='artifacts/cudarc-fallback.json';sha256=$null;bytes=$null}
            $fallbackDecision=[ordered]@{status='NOT_RUN';reason='At least one automatic-differentiation framework passed, so the diagnostic fallback was not eligible to run.'}
        }
        if($frameworksPassed -eq 0){$candidateRefs.Add($fallbackRef)}; $aggregates.Add($fallback)
        $selectionCandidates = @($aggregates | Where-Object role -eq framework | ForEach-Object {
                [pscustomobject]@{ candidate_id = $_.candidate_id; status = $_.status; comparison = $_.summary }
            })
        $selection = Select-P2Candidate -Candidates $selectionCandidates
        $comparison = [ordered]@{ schema = 'python-slm-p2-backend-comparison-v1'; policy_id = 'backend-qualification-v1'
            candidates = $selectionCandidates; selection = $selection }
        $comparisonPath = Join-Path $runRoot 'artifacts\comparison.json'; Write-P2JsonFile -Path $comparisonPath -Value $comparison -CreateNew
        $comparisonRef = New-P2FileReference -Path $comparisonPath -RunRoot $runRoot
        $decision = New-P2Decision -RunId $runId -Selection $selection -Aggregates @($aggregates) `
            -Fallback $fallbackDecision
        $null = Assert-P2Decision -Decision $decision -RunId $runId
        $decisionPath = Join-Path $runRoot 'artifacts\decision.json'; Write-P2JsonFile -Path $decisionPath -Value $decision -CreateNew
        $decisionBasic = New-P2FileReference -Path $decisionPath -RunRoot $runRoot
        $decisionRef = [ordered]@{ path = $decisionBasic.path; sha256 = $decisionBasic.sha256; bytes = $decisionBasic.bytes
            status = $decision.status; selected_candidate_id = $decision.selected_candidate_id; owner_review_status = 'AWAITING_REVIEW' }
        if ([string]$selection.status -cne 'PASS') { throw 'no framework passed P2 qualification' }
        foreach($commandRecord in @($commands)){
            foreach($reference in @($commandRecord.stdout,$commandRecord.stderr)){
                $transcript=Assert-P2FileReference $reference $runRoot 'global canary audit transcript'
                $transcriptText=[IO.File]::ReadAllText($transcript,$script:P2Utf8NoBom)
                if(Test-P2PythonTranscriptViolation $transcriptText){
                    throw 'Python invocation or linkage appeared in a qualification transcript'
                }
            }
        }
        $treesTerminated=-not[bool]$script:P2TreeViolation;if(-not$treesTerminated){throw 'one or more qualification process trees did not terminate cleanly'}
        foreach ($gate in @('bf16_roundtrip', 'correctness', 'synchronization', 'runtime_provenance', 'measurement_validity')) {
            Set-P2Gate $gates $gate PASS 'At least one framework passed the frozen candidate and measurement gates.'
        }
        $endIdentity = Get-P2RepositoryIdentity -RepositoryRoot $repository
        if ($endIdentity.fingerprint -cne $sourceInputFingerprint) { throw 'repository inputs changed during P2 qualification' }
        if((Get-P2EnvironmentFingerprint)-cne$parentEnvironmentBefore){throw 'parent environment changed during P2 qualification'}
        $hostState.cuda_health_recoveries=@($healthRecoveries);$hostState.benchmark_monitor_checks=@($monitorChecks)
        Write-P2JsonFile -Path $hostPath -Value $hostState
        Set-P2Gate $gates input_stability PASS 'Source identity remained stable excluding only P2 output and the known incomplete P1A run.'
        if ((Get-P2DirectoryFingerprint (Join-Path $repository 'target')) -cne $existingTargetBefore) { throw 'existing repository target changed during P2' }
        Set-P2Gate $gates sm120_launch PASS 'A passing framework executed its correctness and timed BF16 CUDA kernels on the P1B-qualified SM120 device.'
        if (-not (Test-P2ReceiptRedaction -RunRoot $runRoot)) { throw 'retained P2 evidence failed redaction' }
        Set-P2Gate $gates redaction PASS 'Retained evidence passed path, credential, UTF-8, and LF validation.'
        $cleanupAttempted = $true; Remove-P2OwnedTemporaryRoot -Path $temporaryRoot; $temporaryRemoved = -not (Test-Path $temporaryRoot)
        Set-P2Gate $gates cleanup $(if ($temporaryRemoved) { 'PASS' } else { 'FAIL' }) 'Owned temporary root cleanup completed.'
        if (-not $temporaryRemoved) { throw 'owned P2 temporary root cleanup failed' }
        $evidence = [ordered]@{
            schema = $script:P2Schema.Evidence; phase_id = 'P2'; run_id = $runId; status = 'PASS'; owner_review_status = 'AWAITING_REVIEW'
            started_at = $started.ToString('o'); finished_at = [DateTime]::UtcNow.ToString('o'); duration_ms = [int64]$watch.ElapsedMilliseconds
            invocation = [ordered]@{ argv = @('powershell','-NoProfile','-ExecutionPolicy','Bypass','-File','scripts/qualify-backend.ps1','-OutputRoot','docs/receipts/P2'); cwd='${REPO}' }
            source_identity_sha256 = $sourceHash; p1b_dependency = $p1b
            policy = New-P2FileReference $policyPath $runRoot; dependency_inventory = New-P2FileReference $dependencyPath $runRoot
            fixture_manifest = New-P2FileReference $fixtureArtifact $runRoot; host_state = New-P2FileReference $hostPath $runRoot
            candidates = @($candidateRefs); comparison = $comparisonRef; decision = $decisionRef
            cpu_isolation = New-P2FileReference $cpuIsolationPath $runRoot; commands = @($commands); gates = $gates; errors = @()
            cleanup = [ordered]@{ attempted=$true;temporary_root_removed=$true;process_trees_terminated=$treesTerminated }
            seal = [ordered]@{path='SHA256SUMS';entries=(@(Get-ChildItem $runRoot -Recurse -File).Count + 1);coverage_rule='all_run_files_except_seal'}
        }
        Write-P2JsonFile -Path (Join-Path $runRoot 'evidence.json') -Value $evidence -CreateNew
        $evidence.seal.entries = @(Get-ChildItem $runRoot -Recurse -File).Count
        Write-P2JsonFile -Path (Join-Path $runRoot 'evidence.json') -Value $evidence
        $sealIdentity=Get-P2RepositoryIdentity -RepositoryRoot $repository
        if($sealIdentity.fingerprint-cne$sourceInputFingerprint){throw 'repository inputs changed before P2 run sealing'}
        if (-not (Test-P2ReceiptRedaction -RunRoot $runRoot)) { throw 'terminal P2 evidence failed redaction' }
        [void](New-P2Seal -RunRoot $runRoot)
        $publication = Publish-P2Acceptance -OutputRoot $output -RunId $runId -SourceIdentitySha256 $sourceHash `
            -RepositoryRoot $repository -SourceInputFingerprint $sourceInputFingerprint
        return [pscustomobject][ordered]@{ status='PASS';phase_id='P2';run_id=$runId;evidence_path="runs/$runId/evidence.json"
            selected_candidate_id=$selection.selected_candidate_id;acceptance_path=$publication.acceptance_path }
    }
    catch {
        $treesTerminated=-not[bool]$script:P2TreeViolation
        $safe = Protect-P2Text -Text ([string]$_.Exception.Message) -RepositoryRoot $repository `
            -ExtraRoots @($env:USERPROFILE,$env:SystemRoot,$env:TEMP,$env:TMP,$temporaryRoot)
        $code = if ($_.Exception.Data.Contains('P2Code')) { [string]$_.Exception.Data['P2Code'] } else { 'P2_QUALIFICATION_FAILED' }
        $category = if ($_.Exception.Data.Contains('P2Category')) { [int]$_.Exception.Data['P2Category'] } else { 5 }
        $errors.Add([ordered]@{ code=$code;category=$category;message=$safe;remediation='Resolve the recorded failure and rerun P2 from stable inputs.' })
        $cleanupAttempted = $true
        if ($null -ne $temporaryRoot -and (Test-Path -LiteralPath $temporaryRoot)) {
            try { Remove-P2OwnedTemporaryRoot -Path $temporaryRoot } catch { $errors.Add([ordered]@{
                        code='TEMP_CLEANUP_FAILED';category=4;message='Owned temporary cleanup failed.';remediation='Remove only the recorded P2 temporary directory.' }) }
        }
        $temporaryRemoved = $null -eq $temporaryRoot -or -not (Test-Path -LiteralPath $temporaryRoot)
        Set-P2Gate $gates cleanup $(if ($temporaryRemoved) {'PASS'}else{'FAIL'}) 'Failure-path temporary cleanup was attempted.'
        $terminalError=[ordered]@{code=$code;category=$category;message=$safe;remediation='Resolve the recorded failure and rerun P2 from stable inputs.'}
        if($null-ne$hostState){$hostState.cuda_health_recoveries=@($healthRecoveries);$hostState.benchmark_monitor_checks=@($monitorChecks);$retainedHost=Join-Path $runRoot 'artifacts\host-state.json'
            if(Test-Path -LiteralPath $retainedHost -PathType Leaf){Write-P2JsonFile -Path $retainedHost -Value $hostState}}
        $converted=Convert-P2UnpublishedPassToFailure -RunRoot $runRoot -ErrorRecord $terminalError
        if (-not $converted -and -not (Test-Path -LiteralPath (Join-Path $runRoot 'evidence.json'))) {
            $null = Write-P2FailureRun -RunRoot $runRoot -RunId $runId -StartedAt $started -Stopwatch $watch `
                -SourceIdentitySha256 $sourceHash -P1BDependency $p1b -Gates $gates -Commands @($commands) -Errors @($errors) `
                -CleanupAttempted $cleanupAttempted -TemporaryRootRemoved $temporaryRemoved -ProcessTreesTerminated $treesTerminated `
                -Policy $policy -DependencyInventory $dependencyInventory -FixtureManifest $fixtureManifest -HostState $hostState `
                -CpuIsolation $cpuIsolation -CandidateReferences @($candidateRefs) -Comparison $comparisonRef -Decision $decisionRef
        }
        return [pscustomobject][ordered]@{status='FAIL';phase_id='P2';run_id=$runId;evidence_path="runs/$runId/evidence.json";errors=@($errors)}
    }
    finally { $watch.Stop() }
}

function Get-P2NearestRankPercentile {
    [CmdletBinding()]
    param([Parameter(Mandatory)][Int64[]]$Values, [Parameter(Mandatory)][double]$Percentile)
    if ($Values.Count -eq 0 -or $Percentile -le 0 -or $Percentile -gt 1) {
        throw 'nearest-rank percentile inputs are invalid'
    }
    $sorted = @($Values | Sort-Object)
    $index = [Math]::Ceiling($Percentile * $sorted.Count) - 1
    return [int64]$sorted[[int]$index]
}

function Get-P2GeometricMean {
    [CmdletBinding()]
    param([Parameter(Mandatory)][double[]]$Values)
    if ($Values.Count -eq 0 -or @($Values | Where-Object { $_ -le 0 }).Count -gt 0) {
        throw 'geometric-mean values must be nonempty and positive'
    }
    return [Math]::Exp((($Values | ForEach-Object { [Math]::Log($_) } |
                    Measure-Object -Average).Average))
}

function Test-P2TimingResult {
    [CmdletBinding()]
    param([Parameter(Mandatory)]$Result)
    if ($null -eq $Result.timing -or [string]$Result.status -cne 'PASS' -or
        [string]$Result.mode -cne 'benchmark' -or
        [string]$Result.workload -notin @('projection', 'ffn-expansion')) { return $false }
    $timing = $Result.timing
    try {
        Assert-P2ClosedFields $timing @('shape','warmup_iterations','forward','forward_backward','context_ns','jit_ns','first_result_ns') 'candidate timing'
        Assert-P2ClosedFields $timing.shape @('m','k','n') 'candidate timing shape'
        foreach($series in @($timing.forward,$timing.forward_backward)) {
            Assert-P2ClosedFields $series @('samples_ns','sample_count','elapsed_ns','p50_ns','p95_ns','flop_count','gflops') 'candidate timing series'
            foreach($value in @($series.samples_ns)) { [void](Assert-P2IntegerNumber $value 'timing sample' 1) }
            foreach($field in @('sample_count','elapsed_ns','p50_ns','p95_ns','flop_count')) {
                [void](Assert-P2IntegerNumber $series.$field "timing.$field" 1)
            }
        }
        foreach($field in @('m','k','n')) { [void](Assert-P2IntegerNumber $timing.shape.$field "timing.shape.$field" 1) }
        [void](Assert-P2IntegerNumber $timing.warmup_iterations 'timing.warmup_iterations' 0)
        foreach($field in @('context_ns','jit_ns','first_result_ns')) { [void](Assert-P2IntegerNumber $timing.$field "timing.$field" 0) }
    } catch { return $false }
    $expectedShape = if ([string]$Result.workload -ceq 'projection') { @(8192, 768, 768) } else { @(8192, 768, 2432) }
    if ([int]$timing.warmup_iterations -ne 50 -or
        [int64]$timing.shape.m -ne $expectedShape[0] -or [int64]$timing.shape.k -ne $expectedShape[1] -or
        [int64]$timing.shape.n -ne $expectedShape[2] -or
        $null -eq $timing.context_ns -or $null -eq $timing.jit_ns -or $null -eq $timing.first_result_ns -or
        [int64]$timing.context_ns -lt 0 -or [int64]$timing.jit_ns -lt 0 -or [int64]$timing.first_result_ns -lt 0) {
        return $false
    }
    $m = [int64]$expectedShape[0]; $k = [int64]$expectedShape[1]; $n = [int64]$expectedShape[2]
    $seriesIndex = 0
    foreach ($series in @($timing.forward, $timing.forward_backward)) {
        $samples = @($series.samples_ns | ForEach-Object { [int64]$_ })
        $count = [int]$series.sample_count; $elapsed = [int64]$series.elapsed_ns
        $factor = if ($seriesIndex -eq 0) { [int64]2 } else { [int64]6 }
        $expectedFlops = $factor * $m * $n * $k
        if ($count -lt 200 -or $count -gt 10000 -or $samples.Count -ne $count -or
            $elapsed -le 0 -or $elapsed -gt 60000000000 -or
            ($elapsed -lt 5000000000 -and $count -ne 10000) -or
            @($samples | Where-Object { $_ -le 0 }).Count -gt 0 -or
            [int64]$series.p50_ns -ne (Get-P2NearestRankPercentile -Values $samples -Percentile 0.5) -or
            [int64]$series.p95_ns -ne (Get-P2NearestRankPercentile -Values $samples -Percentile 0.95) -or
            [int64]$series.flop_count -ne $expectedFlops -or $null -eq $series.gflops) { return $false }
        $actualGflops = [double]$series.gflops; $expectedGflops = [double]$expectedFlops / [double]$series.p50_ns
        if ([double]::IsNaN($actualGflops) -or [double]::IsInfinity($actualGflops) -or
            [Math]::Abs(($actualGflops - $expectedGflops) / $expectedGflops) -gt 1e-9) { return $false }
        $seriesIndex++
    }
    return $true
}

function Get-P2CandidateComparison {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$CandidateId, [Parameter(Mandatory)][object[]]$BenchmarkResults,
        [Parameter(Mandatory)][int]$LockedDependencyCount, [Parameter(Mandatory)][Int64]$ObservedPeakBytes)
    $projection = @($BenchmarkResults | Where-Object { $_.workload -ceq 'projection' })
    $ffn = @($BenchmarkResults | Where-Object { $_.workload -ceq 'ffn-expansion' })
    if ($projection.Count -ne 2 -or $ffn.Count -ne 2) { throw "$CandidateId does not have two ABBA rounds per workload" }
    foreach ($result in @($projection + $ffn)) {
        if (-not (Test-P2TimingResult -Result $result)) { throw "$CandidateId contains an invalid timing result" }
    }
    $p50 = @($projection + $ffn | ForEach-Object { [double]$_.timing.forward_backward.p50_ns })
    $p95 = @($projection + $ffn | ForEach-Object { [double]$_.timing.forward_backward.p95_ns })
    foreach ($set in @($projection, $ffn)) {
        foreach ($seriesName in @('forward', 'forward_backward')) {
            $values = @($set | ForEach-Object { [double]$_.timing.$seriesName.p50_ns })
            if (([Math]::Max($values[0], $values[1]) / [Math]::Min($values[0], $values[1])) - 1.0 -gt 0.1) {
                throw "$CandidateId $seriesName between-round p50 drift exceeds ten percent"
            }
        }
    }
    return [pscustomobject][ordered]@{
        candidate_id = $CandidateId; status = 'PASS'
        comparison = [pscustomobject][ordered]@{
            geomean_fwbw_p50_ns = Get-P2GeometricMean -Values $p50
            geomean_fwbw_p95_ns = Get-P2GeometricMean -Values $p95
            observed_peak_bytes = $ObservedPeakBytes; locked_dependency_count = $LockedDependencyCount
        }
    }
}

function Assert-P2CandidateResult {
    [CmdletBinding()]
    param([Parameter(Mandatory)]$Result, [Parameter(Mandatory)][string]$CandidateId,
        [Parameter(Mandatory)][string]$Mode, [Parameter(Mandatory)][string]$Workload)
    Assert-P2ClosedObject -Value $Result -Fields @(
        'schema', 'candidate_id', 'mode', 'status', 'workload', 'fixture_hashes', 'allocation',
        'correctness', 'timing', 'memory', 'provenance', 'diagnostics') -Name 'candidate result'
    if ([string]$Result.schema -cne $script:P2Schema.Candidate -or
        [string]$Result.candidate_id -cne $CandidateId -or [string]$Result.mode -cne $Mode -or
        [string]$Result.workload -cne $Workload -or [string]$Result.status -notin @('PASS', 'FAIL') -or
        $CandidateId -notin @('burn-cubecl','candle','cudarc-fallback') -or
        $Mode -notin @('cpu-smoke','correctness','benchmark') -or
        $Workload -notin @('allocation','correctness','projection','ffn-expansion') -or
        ($Mode -ceq 'cpu-smoke' -and $Workload -cne 'correctness') -or
        ($Mode -ceq 'correctness' -and $Workload -notin @('allocation','correctness')) -or
        ($Mode -ceq 'benchmark' -and $Workload -notin @('projection','ffn-expansion'))) {
        throw 'candidate result identity does not match its invocation'
    }
    Assert-P2ClosedFields $Result.provenance @('crate_name','crate_version','feature_set','device','device_ordinal','explicit_synchronization','fp32_accumulation_evidence','framework_rng_used') 'candidate provenance'
    if([string]$Result.provenance.crate_name-cnotmatch'^[a-z0-9][a-z0-9_-]*$'-or[string]$Result.provenance.crate_version-cnotmatch'^[0-9]+\.[0-9]+\.[0-9]+$'-or
        [string]::IsNullOrWhiteSpace([string]$Result.provenance.device)-or[string]::IsNullOrWhiteSpace([string]$Result.provenance.fp32_accumulation_evidence)-or
        $Result.provenance.explicit_synchronization-isnot[bool]-or$Result.provenance.framework_rng_used-isnot[bool]){throw 'candidate provenance types are invalid'}
    if($null-ne$Result.provenance.device_ordinal){[void](Assert-P2IntegerNumber $Result.provenance.device_ordinal 'device ordinal' 0)}
    $features=@($Result.provenance.feature_set)
    if(@($features|Sort-Object -Unique).Count-ne$features.Count-or@($features|Where-Object{[string]$_-cnotmatch'^[A-Za-z0-9_-]+$'}).Count-ne0){throw 'candidate feature set is invalid'}
    foreach($diagnostic in @($Result.diagnostics)) {
        Assert-P2ClosedFields $diagnostic @('code','message') 'candidate diagnostic'
        if([string]$diagnostic.code-cnotmatch'^[A-Z][A-Z0-9_]*$'-or[string]::IsNullOrWhiteSpace([string]$diagnostic.message)-or
            [string]$diagnostic.message-match '(?:[A-Za-z]:[\\/]|\\\\|[\r\n])'){throw 'candidate diagnostic is not receipt-safe'}
    }
    if($null-ne$Result.fixture_hashes){
        Assert-P2ClosedFields $Result.fixture_hashes @('algorithm','a_sha256','b_sha256','a_elements','b_elements') 'candidate fixture hashes'
        Assert-P2Sha256 $Result.fixture_hashes.a_sha256 'fixture A hash'
        if($null-ne$Result.fixture_hashes.b_sha256){Assert-P2Sha256 $Result.fixture_hashes.b_sha256 'fixture B hash'}
        [void](Assert-P2IntegerNumber $Result.fixture_hashes.a_elements 'fixture A elements' 1)
        [void](Assert-P2IntegerNumber $Result.fixture_hashes.b_elements 'fixture B elements' 0)
    }
    if($null-ne$Result.allocation){
        Assert-P2ClosedFields $Result.allocation @('shape','elements','input_sha256','output_sha256','bitwise_equal') 'candidate allocation'
        if(@($Result.allocation.shape).Count-ne3-or$Result.allocation.bitwise_equal-isnot[bool]){throw 'candidate allocation types are invalid'}
        foreach($value in @($Result.allocation.shape)){[void](Assert-P2IntegerNumber $value 'allocation shape' 1)}
        [void](Assert-P2IntegerNumber $Result.allocation.elements 'allocation elements' 1);Assert-P2Sha256 $Result.allocation.input_sha256 'allocation input hash';Assert-P2Sha256 $Result.allocation.output_sha256 'allocation output hash'
    }
    if($null-ne$Result.memory){
        Assert-P2ClosedFields $Result.memory @('free_bytes_after_context','free_bytes_after_allocation','free_bytes_after_forward','free_bytes_after_backward') 'candidate memory'
        foreach($field in @('free_bytes_after_context','free_bytes_after_allocation','free_bytes_after_forward','free_bytes_after_backward')){if($null-ne$Result.memory.$field){[void](Assert-P2IntegerNumber $Result.memory.$field "memory.$field" 0)}}
    }
    if($null-ne$Result.correctness){Assert-P2CorrectnessStructure -Correctness $Result.correctness}
    if($null-ne$Result.timing){Assert-P2TimingStructure -Timing $Result.timing}
    if ([string]$Result.status -ceq 'PASS') {
        if ([bool]$Result.provenance.framework_rng_used -or -not [bool]$Result.provenance.explicit_synchronization -or
            [string]::IsNullOrWhiteSpace([string]$Result.provenance.fp32_accumulation_evidence) -or
            $null -eq $Result.fixture_hashes -or @($Result.diagnostics).Count -ne 0) {
            throw 'passing candidate provenance violates the frozen policy'
        }
        $fixtureExpected = (Get-P2CanonicalFixtureTable)[$Workload]
        $fixture = $Result.fixture_hashes
        if ([string]$fixture.algorithm -cne 'sha256' -or [string]$fixture.a_sha256 -cne [string]$fixtureExpected.a_sha256 -or
            [uint64]$fixture.a_elements -ne [uint64]$fixtureExpected.a_elements -or
            [uint64]$fixture.b_elements -ne [uint64]$fixtureExpected.b_elements -or
            (($null -eq $fixtureExpected.b_sha256) -ne ($null -eq $fixture.b_sha256)) -or
            ($null -ne $fixtureExpected.b_sha256 -and [string]$fixture.b_sha256 -cne [string]$fixtureExpected.b_sha256)) {
            throw 'candidate result fixture identity is not canonical'
        }
        $accumulationEvidence = [string]$Result.provenance.fp32_accumulation_evidence
        $expectedCrate=@{'burn-cubecl'=@('burn','0.21.0');candle=@('candle-core','0.11.0');'cudarc-fallback'=@('cudarc','0.19.8')}[$CandidateId]
        if([string]$Result.provenance.crate_name-cne$expectedCrate[0]-or[string]$Result.provenance.crate_version-cne$expectedCrate[1]){throw 'candidate crate identity is not pinned'}
        $expectedFeatures=if($Mode-ceq'cpu-smoke'){
            if($CandidateId-ceq'burn-cubecl'){@('autodiff','flex')}elseif($CandidateId-ceq'candle'){@()}else{throw 'diagnostic fallback cannot satisfy CPU smoke'}
        }elseif($CandidateId-ceq'burn-cubecl'){@('autodiff','autotune','cuda','fusion')}
        elseif($CandidateId-ceq'candle'){@('cuda')}else{@('cublas','cublaslt','cuda-13010','driver')}
        if(($features-join',')-cne($expectedFeatures-join',')){throw 'candidate feature set does not match the pinned mode'}
        if($Mode-ceq'cpu-smoke'){if([string]$Result.provenance.device-cne'CPU'-or$null-ne$Result.provenance.device_ordinal){throw 'CPU provenance device identity is invalid'}}
        else{
            if($null-eq$Result.provenance.device_ordinal){throw 'GPU provenance device identity is invalid'}
            [void](Assert-P2IntegerNumber $Result.provenance.device_ordinal 'device ordinal' 0)
            if([int]$Result.provenance.device_ordinal-ne0-or[string]$Result.provenance.device-cne'CUDA device 0'){throw 'GPU provenance device identity is invalid'}
        }
        if($Mode-ceq'cpu-smoke'){
            if($null-ne$Result.memory-or$null-ne$Result.allocation-or$null-ne$Result.correctness-or$null-ne$Result.timing){
                throw 'CPU smoke result contains GPU-only result sections'
            }
        }else{
            if($null-eq$Result.memory){throw 'GPU candidate result is missing memory checkpoints'}
            Assert-P2ClosedFields $Result.memory @('free_bytes_after_context','free_bytes_after_allocation','free_bytes_after_forward','free_bytes_after_backward') 'candidate memory'
            foreach($field in @('free_bytes_after_context','free_bytes_after_allocation')){if($null-eq$Result.memory.$field-or[int64]$Result.memory.$field-lt0){throw 'CUDA memory checkpoint is missing'}}
            if($Workload-ceq'allocation'){
                if($null-ne$Result.memory.free_bytes_after_forward-or$null-ne$Result.memory.free_bytes_after_backward){throw 'allocation memory includes nonexistent graph stages'}
            }else{foreach($field in @('free_bytes_after_forward','free_bytes_after_backward')){if($null-eq$Result.memory.$field-or[int64]$Result.memory.$field-lt0){throw 'CUDA memory checkpoint is missing'}}}
        }
        if ($Mode -cne 'cpu-smoke' -and $CandidateId -ceq 'burn-cubecl' -and
            $accumulationEvidence -cne 'crate=cubek-matmul@0.2.0;crate-checksum-sha256=83a4cea5f0f439907dc953c7638a6204b3f055f1bcbd10db91dfc5faa030ac1c;source-sha256=f74fe23b585d7b359d7c04f5a9e586c517684ff4e788807ee67897383160c655;locator=cubek-matmul-0.2.0/src/definition/spec.rs:85-91;assertion=BF16_MATMUL_PRECISION_ACCUMULATOR_F32;cfg=not-macos') {
            throw 'Burn accumulation evidence is not an inspectable source/hash locator'
        }
        if ($Mode -cne 'cpu-smoke' -and $CandidateId -ceq 'candle' -and
            $accumulationEvidence -cne 'runtime_getter=candle_core::cuda::gemm_reduced_precision_bf16;observed=false;compute=CUBLAS_COMPUTE_32F') {
            throw 'Candle did not verify reduced-precision BF16 accumulation is disabled at runtime'
        }
        if ($Mode -ceq 'correctness' -and $Workload -ceq 'allocation' -and (-not [bool]$Result.allocation.bitwise_equal -or
                (@($Result.allocation.shape) -join ',') -cne '16,2048,768' -or
                [uint64]$Result.allocation.elements -ne 25165824 -or
                [string]$Result.allocation.input_sha256 -cne [string]$fixtureExpected.a_sha256 -or
                [string]$Result.allocation.output_sha256 -cne [string]$fixtureExpected.a_sha256)) { throw 'BF16 allocation round-trip failed' }
        if($Mode-ceq'correctness'-and$Workload-ceq'allocation'){
            Assert-P2ClosedFields $Result.allocation @('shape','elements','input_sha256','output_sha256','bitwise_equal') 'candidate allocation'
            foreach($value in @($Result.allocation.shape)){[void](Assert-P2IntegerNumber $value 'allocation shape' 1)}
            [void](Assert-P2IntegerNumber $Result.allocation.elements 'allocation elements' 1)
            Assert-P2Sha256 $Result.allocation.input_sha256 'allocation input hash';Assert-P2Sha256 $Result.allocation.output_sha256 'allocation output hash'
            if($null-ne$Result.correctness-or$null-ne$Result.timing){throw 'allocation result contains non-applicable sections'}
        }
        if ($Mode -ceq 'correctness' -and $Workload -ceq 'correctness') {
            $c = $Result.correctness
            Assert-P2ClosedFields $c @('shape','accumulation','output_dtype','loss_dtype','forward','loss','grad_a','grad_b','nan_count','infinite_count','envelope_violation_count') 'candidate correctness'
            Assert-P2ClosedFields $c.shape @('m','k','n') 'correctness shape'
            foreach($field in @('m','k','n')){[void](Assert-P2IntegerNumber $c.shape.$field "correctness.shape.$field" 1)}
            foreach($field in @('nan_count','infinite_count','envelope_violation_count')){[void](Assert-P2IntegerNumber $c.$field "correctness.$field" 0)}
            if($null-ne$Result.allocation-or$null-ne$Result.timing){throw 'correctness result contains non-applicable sections'}
            if ($null -eq $c -or [string]$c.accumulation -cne 'fp32' -or [string]$c.output_dtype -cne 'bf16' -or
                [string]$c.loss_dtype -cne 'fp32' -or [int]$c.shape.m -ne 17 -or [int]$c.shape.k -ne 31 -or [int]$c.shape.n -ne 29 -or
                [int]$c.nan_count -ne 0 -or
                [int]$c.infinite_count -ne 0 -or [int]$c.envelope_violation_count -ne 0 -or
                [double]$c.forward.relative_l2 -gt 0.015 -or -not [bool]$c.loss.passed -or
                [double]$c.grad_a.relative_l2 -gt 0.03 -or [double]$c.grad_b.relative_l2 -gt 0.03 -or
                [double]$c.grad_a.cosine_similarity -lt 0.999 -or [double]$c.grad_b.cosine_similarity -lt 0.999) {
                throw 'candidate correctness metrics violate the frozen tolerance policy'
            }
            foreach($metricSpec in @(@('forward',493,0.015,-1.0),@('grad_a',527,0.03,0.999),@('grad_b',899,0.03,0.999))){
                $metric=$c.($metricSpec[0]);Assert-P2ClosedFields $metric @('elements','max_absolute_error','max_relative_error','relative_l2','cosine_similarity','envelope_violation_count') "correctness.$($metricSpec[0])"
                if([int]$metric.elements-ne[int]$metricSpec[1]-or[int]$metric.envelope_violation_count-ne0){throw 'correctness metric element/envelope count mismatch'}
                [void](Assert-P2FiniteNumber $metric.max_absolute_error 'max_absolute_error' 0)
                [void](Assert-P2FiniteNumber $metric.max_relative_error 'max_relative_error' 0)
                [void](Assert-P2FiniteNumber $metric.relative_l2 'relative_l2' 0 ([double]$metricSpec[2]))
                [void](Assert-P2FiniteNumber $metric.cosine_similarity 'cosine_similarity' ([double]$metricSpec[3]) 1)
            }
            Assert-P2ClosedFields $c.loss @('actual','reference','absolute_error','allowed_error','passed') 'correctness.loss'
            $lossActual=Assert-P2FiniteNumber $c.loss.actual 'loss.actual';$lossReference=Assert-P2FiniteNumber $c.loss.reference 'loss.reference'
            $lossAbsolute=Assert-P2FiniteNumber $c.loss.absolute_error 'loss.absolute_error' 0
            $lossAllowed=Assert-P2FiniteNumber $c.loss.allowed_error 'loss.allowed_error' 0
            $expectedAbsolute=[Math]::Abs($lossActual-$lossReference);$expectedAllowed=0.00001+0.01*[Math]::Abs($lossReference)
            if(-not[bool]$c.loss.passed-or[Math]::Abs($expectedAbsolute-$lossAbsolute)-gt1e-12-or
                [Math]::Abs($expectedAllowed-$lossAllowed)-gt1e-12-or$lossAbsolute-gt($lossAllowed+1e-12)){throw 'loss arithmetic or tolerance is invalid'}
        }
        if ($Mode -ceq 'benchmark' -and -not (Test-P2TimingResult -Result $Result)) {
            throw 'candidate timing evidence violates the frozen timing policy'
        }
        if($Mode-ceq'benchmark'-and($null-ne$Result.allocation-or$null-ne$Result.correctness)){throw 'benchmark result contains non-applicable sections'}
    }
    elseif (@($Result.diagnostics).Count -eq 0) { throw 'failed candidate result contains no diagnostic' }
    return $Result
}

function Assert-P2MetricStructure {
    param([Parameter(Mandatory)]$Metric,[Parameter(Mandatory)][string]$Name)
    Assert-P2ClosedFields $Metric @('elements','max_absolute_error','max_relative_error','relative_l2','cosine_similarity','envelope_violation_count') $Name
    [void](Assert-P2IntegerNumber $Metric.elements "$Name.elements" 1);[void](Assert-P2IntegerNumber $Metric.envelope_violation_count "$Name.envelope_violation_count" 0)
    foreach($field in @('max_absolute_error','max_relative_error','relative_l2')){[void](Assert-P2FiniteNumber $Metric.$field "$Name.$field" 0)}
    [void](Assert-P2FiniteNumber $Metric.cosine_similarity "$Name.cosine_similarity" -1 1)
}

function Assert-P2CorrectnessStructure {
    param([Parameter(Mandatory)]$Correctness)
    Assert-P2ClosedFields $Correctness @('shape','accumulation','output_dtype','loss_dtype','forward','loss','grad_a','grad_b','nan_count','infinite_count','envelope_violation_count') 'candidate correctness'
    Assert-P2ClosedFields $Correctness.shape @('m','k','n') 'correctness shape';foreach($field in @('m','k','n')){[void](Assert-P2IntegerNumber $Correctness.shape.$field "correctness.shape.$field" 1)}
    if([string]$Correctness.accumulation-notin@('fp32','unverified')-or[string]$Correctness.output_dtype-cne'bf16'-or[string]$Correctness.loss_dtype-cne'fp32'){throw 'correctness dtype/accumulation is invalid'}
    Assert-P2MetricStructure $Correctness.forward 'correctness.forward';Assert-P2MetricStructure $Correctness.grad_a 'correctness.grad_a';Assert-P2MetricStructure $Correctness.grad_b 'correctness.grad_b'
    Assert-P2ClosedFields $Correctness.loss @('actual','reference','absolute_error','allowed_error','passed') 'correctness.loss'
    foreach($field in @('actual','reference')){[void](Assert-P2FiniteNumber $Correctness.loss.$field "loss.$field")};foreach($field in @('absolute_error','allowed_error')){[void](Assert-P2FiniteNumber $Correctness.loss.$field "loss.$field" 0)}
    if($Correctness.loss.passed-isnot[bool]){throw 'loss pass field is not Boolean'};foreach($field in @('nan_count','infinite_count','envelope_violation_count')){[void](Assert-P2IntegerNumber $Correctness.$field "correctness.$field" 0)}
}

function Assert-P2TimingStructure {
    param([Parameter(Mandatory)]$Timing)
    Assert-P2ClosedFields $Timing @('shape','warmup_iterations','forward','forward_backward','context_ns','jit_ns','first_result_ns') 'candidate timing'
    Assert-P2ClosedFields $Timing.shape @('m','k','n') 'candidate timing shape';foreach($field in @('m','k','n')){[void](Assert-P2IntegerNumber $Timing.shape.$field "timing.shape.$field" 1)}
    foreach($field in @('warmup_iterations','context_ns','jit_ns','first_result_ns')){[void](Assert-P2IntegerNumber $Timing.$field "timing.$field" 0)}
    foreach($series in @($Timing.forward,$Timing.forward_backward)){
        Assert-P2ClosedFields $series @('samples_ns','sample_count','elapsed_ns','p50_ns','p95_ns','flop_count','gflops') 'candidate timing series'
        if(@($series.samples_ns).Count-lt1-or@($series.samples_ns).Count-gt10000){throw 'timing sample array length is invalid'};foreach($value in @($series.samples_ns)){[void](Assert-P2IntegerNumber $value 'timing sample' 1)}
        foreach($field in @('sample_count','elapsed_ns','p50_ns','p95_ns','flop_count')){[void](Assert-P2IntegerNumber $series.$field "timing.$field" 1)};[void](Assert-P2FiniteNumber $series.gflops 'timing.gflops' ([double]::Epsilon))
    }
}

function Assert-P2BurnAccumulationEvidence {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Evidence, [Parameter(Mandatory)][string]$CargoHome)
    if ($Evidence -cnotmatch '^crate=cubek-matmul@0\.2\.0;crate-checksum-sha256=(?<checksum>[0-9a-f]{64});source-sha256=(?<hash>[0-9a-f]{64});locator=(?<locator>cubek-matmul-0\.2\.0/src/definition/spec\.rs):(?<first>85)-(?<last>91);assertion=(?<assertion>BF16_MATMUL_PRECISION_ACCUMULATOR_F32);cfg=not-macos$') {
        throw 'Burn accumulation evidence grammar is invalid'
    }
    if($Matches['checksum']-cne'83a4cea5f0f439907dc953c7638a6204b3f055f1bcbd10db91dfc5faa030ac1c'-or
        $Matches['hash']-cne'f74fe23b585d7b359d7c04f5a9e586c517684ff4e788807ee67897383160c655'){throw 'Burn accumulation evidence pin is not canonical'}
    $expectedHash = $Matches['hash']; $locator = $Matches['locator']; $firstLine = [int]$Matches['first']; $lastLine = [int]$Matches['last']
    $assertion = $Matches['assertion']
    $candidates = @(Get-ChildItem -LiteralPath (Join-Path $CargoHome 'registry\src') -Recurse -File `
        -Filter spec.rs -ErrorAction Stop | Where-Object {
            $_.FullName.Replace('\', '/') -match '/cubek-matmul-0\.2\.0/src/definition/spec\.rs$'
        })
    if ($candidates.Count -ne 1) { throw 'Burn CubeCL accumulation source locator is missing or ambiguous' }
    if ((Get-P2Sha256 -Path $candidates[0].FullName) -cne $expectedHash) {
        throw 'Burn CubeCL accumulation source hash does not match the candidate evidence'
    }
    $lines = [IO.File]::ReadAllLines($candidates[0].FullName, $script:P2Utf8NoBom)
    $window = $lines[($firstLine - 1)..($lastLine - 1)] -join "`n"
    if ($window -notmatch '(?s)MatmulPrecision.*bf16.*Acc.*\(bf16, f32\)') {
        throw 'Burn matmul precision locator does not contain the asserted BF16/F32 accumulator path'
    }
    return [pscustomobject][ordered]@{
        source_sha256 = $expectedHash; locator = $locator + ':' + $firstLine + '-' + $lastLine
        assertion = $assertion; verified = $true
    }
}

function Select-P2Candidate {
    [CmdletBinding()]
    param([Parameter(Mandatory)][object[]]$Candidates)
    $frameworks = @($Candidates | Where-Object {
            $_.candidate_id -in @('burn-cubecl', 'candle') -and $_.status -ceq 'PASS'
        } | Sort-Object candidate_id)
    if ($frameworks.Count -eq 0) {
        return [pscustomobject][ordered]@{ status = 'FAIL'; selected_candidate_id = $null; criterion = 'no_framework_passed' }
    }
    if ($frameworks.Count -eq 1) {
        return [pscustomobject][ordered]@{ status = 'PASS'; selected_candidate_id = $frameworks[0].candidate_id; criterion = 'only_passing_framework' }
    }
    $a = $frameworks[0]
    $b = $frameworks[1]
    $metrics = @(
        @{ Name = 'geomean_fwbw_p50_ns'; Lower = $true },
        @{ Name = 'geomean_fwbw_p95_ns'; Lower = $true },
        @{ Name = 'observed_peak_bytes'; Lower = $true }
    )
    foreach ($metric in $metrics) {
        $av = [double]$a.comparison.($metric.Name)
        $bv = [double]$b.comparison.($metric.Name)
        $best = [Math]::Min($av, $bv)
        $worst = [Math]::Max($av, $bv)
        if ($best -le 0) { throw "candidate metric is invalid: $($metric.Name)" }
        if (($worst / $best) -gt 1.05) {
            $winner = if ($av -lt $bv) { $a } else { $b }
            return [pscustomobject][ordered]@{
                status = 'PASS'; selected_candidate_id = $winner.candidate_id; criterion = $metric.Name
            }
        }
    }
    $ad = [int]$a.comparison.locked_dependency_count
    $bd = [int]$b.comparison.locked_dependency_count
    if ($ad -ne $bd) {
        $winner = if ($ad -lt $bd) { $a } else { $b }
        return [pscustomobject][ordered]@{
            status = 'PASS'; selected_candidate_id = $winner.candidate_id; criterion = 'locked_dependency_count'
        }
    }
    return [pscustomobject][ordered]@{
        status = 'PASS'; selected_candidate_id = @($a.candidate_id, $b.candidate_id | Sort-Object)[0]
        criterion = 'stable_candidate_id'
    }
}

function Assert-P2ClosedObject {
    param([Parameter(Mandatory)]$Value, [Parameter(Mandatory)][string[]]$Fields,
        [Parameter(Mandatory)][string]$Name)
    if ($null -eq $Value) { throw "$Name is null" }
    $actual = @($(if($Value-is[Collections.IDictionary]){$Value.Keys}else{$Value.PSObject.Properties.Name}) | Sort-Object)
    $expected = @($Fields | Sort-Object)
    if (($actual -join "`n") -cne ($expected -join "`n")) {
        throw "$Name fields do not match its closed schema (actual=$($actual -join ','))"
    }
}

function Assert-P2Sha256 {
    param([AllowNull()][string]$Value, [Parameter(Mandatory)][string]$Name)
    if ($Value -cnotmatch '^[0-9a-f]{64}$') { throw "$Name is not a canonical SHA-256" }
}

function Assert-P2ClosedFields {
    param([Parameter(Mandatory)]$Value,[Parameter(Mandatory)][string[]]$Fields,[Parameter(Mandatory)][string]$Name)
    Assert-P2ClosedObject -Value $Value -Fields $Fields -Name $Name
}

function Assert-P2FiniteNumber {
    param($Value,[Parameter(Mandatory)][string]$Name,[double]$Minimum=[double]::NegativeInfinity,
        [double]$Maximum=[double]::PositiveInfinity)
    if($null -eq $Value -or $Value -is [string] -or $Value -is [bool]){throw "$Name is not numeric"}
    $number=[double]$Value
    if([double]::IsNaN($number)-or[double]::IsInfinity($number)-or$number-lt$Minimum-or$number-gt$Maximum){throw "$Name is not a finite in-range number"}
    return $number
}

function Assert-P2IntegerNumber {
    param($Value,[Parameter(Mandatory)][string]$Name,[Int64]$Minimum=[Int64]::MinValue,
        [Int64]$Maximum=[Int64]::MaxValue)
    if($null-eq$Value-or($Value-isnot[byte]-and$Value-isnot[sbyte]-and$Value-isnot[int16]-and$Value-isnot[uint16]-and
            $Value-isnot[int32]-and$Value-isnot[uint32]-and$Value-isnot[int64]-and$Value-isnot[uint64])){throw "$Name is not an integer"}
    try{$number=[int64]$Value}catch{throw "$Name is outside the supported integer range"}
    if($number-lt$Minimum-or$number-gt$Maximum){throw "$Name is outside the permitted range"}
    return $number
}

function Resolve-P2ContainedFile {
    param([Parameter(Mandatory)][string]$Root, [Parameter(Mandatory)][string]$Relative,
        [Parameter(Mandatory)][string]$Name)
    if ($Relative -notmatch '^[A-Za-z0-9_-][A-Za-z0-9._-]*(?:/[A-Za-z0-9_-][A-Za-z0-9._-]*)*$' -or
        $Relative -match '(^|/)\.\.?(/|$)' -or [IO.Path]::IsPathRooted($Relative)) {
        throw "$Name is not a canonical relative path"
    }
    $full = [IO.Path]::GetFullPath((Join-Path $Root $Relative.Replace('/', '\')))
    if (-not (Test-P2PathWithin -Path $full -Root $Root) -or
        -not (Test-Path -LiteralPath $full -PathType Leaf)) {
        throw "$Name does not resolve to a contained file"
    }
    $cursor = $full
    while (Test-P2PathWithin -Path $cursor -Root $Root) {
        if (((Get-Item -Force -LiteralPath $cursor).Attributes -band
                [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "$Name resolves through a reparse point"
        }
        if ([string]::Equals($cursor.TrimEnd('\', '/'),
                [IO.Path]::GetFullPath($Root).TrimEnd('\', '/'),
                [StringComparison]::OrdinalIgnoreCase)) { break }
        $cursor = Split-Path -Parent $cursor
    }
    return $full
}

function New-P2FileReference {
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][string]$RunRoot)
    return [pscustomobject][ordered]@{
        path = ([IO.Path]::GetFullPath($Path).Substring(
                [IO.Path]::GetFullPath($RunRoot).TrimEnd('\', '/').Length + 1)).Replace('\', '/')
        sha256 = Get-P2Sha256 -Path $Path
        bytes = [int64](Get-Item -LiteralPath $Path).Length
    }
}

function Assert-P2FileReference {
    param([Parameter(Mandatory)]$Reference, [Parameter(Mandatory)][string]$RunRoot,
        [Parameter(Mandatory)][string]$Name)
    Assert-P2ClosedObject -Value $Reference -Fields @('path', 'sha256', 'bytes') -Name $Name
    Assert-P2Sha256 -Value ([string]$Reference.sha256) -Name "$Name.sha256"
    $path = Resolve-P2ContainedFile -Root $RunRoot -Relative ([string]$Reference.path) -Name "$Name.path"
    if ((Get-P2Sha256 -Path $path) -cne [string]$Reference.sha256 -or
        [int64](Get-Item -LiteralPath $path).Length -ne [int64]$Reference.bytes) {
        throw "$Name does not match its referenced file"
    }
    return $path
}

function Assert-P2DecisionReference {
    param([Parameter(Mandatory)]$Reference, [Parameter(Mandatory)][string]$RunRoot,
        [Parameter(Mandatory)][string]$Name)
    Assert-P2ClosedObject -Value $Reference -Fields @(
        'path', 'sha256', 'bytes', 'status', 'selected_candidate_id', 'owner_review_status') -Name $Name
    $basic = [pscustomobject][ordered]@{
        path = [string]$Reference.path; sha256 = [string]$Reference.sha256; bytes = [int64]$Reference.bytes
    }
    $path = Assert-P2FileReference -Reference $basic -RunRoot $RunRoot -Name $Name
    if ([string]$Reference.path -cne 'artifacts/decision.json' -or
        [string]$Reference.status -notin @('PASS', 'FAIL') -or
        [string]$Reference.owner_review_status -cne 'AWAITING_REVIEW' -or
        ($null -ne $Reference.selected_candidate_id -and
            [string]$Reference.selected_candidate_id -notin @('burn-cubecl', 'candle'))) {
        throw "$Name decision metadata is invalid"
    }
    return $path
}

function New-P2Seal {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$RunRoot)
    $root = [IO.Path]::GetFullPath($RunRoot)
    $sealPath = Join-Path $root 'SHA256SUMS'
    if (Test-Path -LiteralPath $sealPath) { throw 'refusing to replace an existing P2 run seal' }
    $lines = [Collections.Generic.List[string]]::new()
    Get-ChildItem -LiteralPath $root -Recurse -File | Sort-Object {
        $_.FullName.Substring($root.TrimEnd('\').Length + 1).Replace('\', '/')
    } | ForEach-Object {
        $relative = $_.FullName.Substring($root.TrimEnd('\').Length + 1).Replace('\', '/')
        $lines.Add("$(Get-P2Sha256 -Path $_.FullName)  $relative")
    }
    if ($lines.Count -eq 0) { throw 'cannot seal an empty P2 run' }
    Write-P2Utf8LfFile -Path $sealPath -Text ($lines -join "`n") -CreateNew
    if (-not (Test-P2Seal -RunRoot $root)) { throw 'new P2 run seal did not verify' }
    return [pscustomobject][ordered]@{
        path = 'SHA256SUMS'; entries = $lines.Count; coverage_rule = 'all_run_files_except_seal'
    }
}

function Test-P2Seal {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$RunRoot)
    try {
        $root = [IO.Path]::GetFullPath($RunRoot)
        $sealPath = Join-Path $root 'SHA256SUMS'
        if (-not (Test-Path -LiteralPath $sealPath -PathType Leaf)) { return $false }
        $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
        $previous = $null
        foreach ($line in [IO.File]::ReadAllLines($sealPath, $script:P2Utf8NoBom)) {
            if ($line -notmatch '^([0-9a-f]{64})  ([A-Za-z0-9_-][A-Za-z0-9._-]*(?:/[A-Za-z0-9_-][A-Za-z0-9._-]*)*)$') {
                return $false
            }
            $hash = $Matches[1]; $relative = $Matches[2]
            if (-not $seen.Add($relative) -or ($null -ne $previous -and
                    [string]::CompareOrdinal($previous, $relative) -ge 0)) { return $false }
            $previous = $relative
            $path = Resolve-P2ContainedFile -Root $root -Relative $relative -Name 'sealed path'
            if ((Get-P2Sha256 -Path $path) -cne $hash) { return $false }
        }
        $actual = @(Get-ChildItem -LiteralPath $root -Recurse -File |
            Where-Object { $_.FullName -cne $sealPath } | ForEach-Object {
                $_.FullName.Substring($root.TrimEnd('\').Length + 1).Replace('\', '/')
            })
        if ($actual.Count -ne $seen.Count) { return $false }
        foreach ($relative in $actual) { if (-not $seen.Contains($relative)) { return $false } }
        return $seen.Count -gt 0
    }
    catch { return $false }
}

function Get-P2SelectedP1BDependency {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$RepositoryRoot)
    $repository = [IO.Path]::GetFullPath($RepositoryRoot)
    $root = Join-Path $repository 'docs\receipts\P1B'
    $pointerPath = Join-Path $root 'evidence.json'
    if (-not (Test-Path -LiteralPath $pointerPath -PathType Leaf)) { throw 'selected P1B pointer is missing' }
    $pointer = [IO.File]::ReadAllText($pointerPath, $script:P2Utf8NoBom) | ConvertFrom-Json
    Assert-P2ClosedObject -Value $pointer -Fields @(
        'schema', 'phase_id', 'acceptance_path', 'acceptance_sha256', 'updated_at') -Name 'P1B pointer'
    if ([string]$pointer.schema -cne 'python-slm-phase-evidence-pointer-v2' -or
        [string]$pointer.phase_id -cne 'P1B' -or
        [string]$pointer.acceptance_path -cnotmatch '^acceptances/(?<sequence>[0-9]{8})\.json$') {
        throw 'selected P1B pointer identity is invalid'
    }
    $sequence = [int]$Matches['sequence']
    $acceptancePath = Resolve-P2ContainedFile -Root $root -Relative ([string]$pointer.acceptance_path) `
        -Name 'P1B acceptance'
    if ((Get-P2Sha256 -Path $acceptancePath) -cne [string]$pointer.acceptance_sha256) {
        throw 'selected P1B pointer hash does not match its acceptance'
    }
    $acceptance = [IO.File]::ReadAllText($acceptancePath, $script:P2Utf8NoBom) | ConvertFrom-Json
    Assert-P2ClosedObject -Value $acceptance -Fields @(
        'schema', 'phase_id', 'sequence', 'status', 'acceptance_kind', 'required_approvals',
        'run_path', 'run_evidence_sha256', 'seal_path', 'seal_sha256', 'environment_path',
        'environment_sha256', 'source_identity_sha256', 'previous_acceptance_sha256', 'created_at') `
        -Name 'P1B acceptance'
    if ([string]$acceptance.schema -cne 'python-slm-phase-acceptance-v2' -or
        [string]$acceptance.phase_id -cne 'P1B' -or [int]$acceptance.sequence -ne $sequence -or
        [string]$acceptance.status -cne 'PASS' -or
        [string]$acceptance.acceptance_kind -cne 'automatic_machine_qualification' -or
        @($acceptance.required_approvals).Count -ne 0 -or
        [string]$acceptance.run_path -cnotmatch '^runs/(?<run>[0-9]{8}T[0-9]{9}Z-[0-9a-f]{24})$') {
        throw 'selected P1B acceptance is invalid'
    }
    $runId = $Matches['run']; $runRoot = Join-Path $root ([string]$acceptance.run_path).Replace('/', '\')
    $evidencePath = Join-Path $runRoot 'evidence.json'; $sealPath = Join-Path $runRoot 'SHA256SUMS'
    $environmentPath = Join-Path $runRoot 'artifacts\environment.json'
    foreach ($binding in @(
            @($evidencePath, $acceptance.run_evidence_sha256), @($sealPath, $acceptance.seal_sha256),
            @($environmentPath, $acceptance.environment_sha256))) {
        if (-not (Test-Path -LiteralPath $binding[0] -PathType Leaf) -or
            (Get-P2Sha256 -Path $binding[0]) -cne [string]$binding[1]) {
            throw 'selected P1B acceptance artifact binding failed'
        }
    }
    if (-not (Test-P2Seal -RunRoot $runRoot)) { throw 'selected P1B run seal is invalid' }
    $evidence = [IO.File]::ReadAllText($evidencePath, $script:P2Utf8NoBom) | ConvertFrom-Json
    Assert-P2ClosedObject $evidence @('schema','phase_id','run_id','mode','status','started_at','finished_at','duration_ms',
        'invocation','source_identity_sha256','p0_dependency','p1a_dependency','environment','commands','gates','errors','cleanup','seal') 'P1B evidence'
    if ([string]$evidence.schema -cne 'python-slm-phase-evidence-v2' -or
        [string]$evidence.phase_id -cne 'P1B' -or [string]$evidence.mode -cne 'Cuda' -or [string]$evidence.status -cne 'PASS' -or
        [string]$evidence.run_id -cne $runId -or
        [string]$evidence.source_identity_sha256 -cne [string]$acceptance.source_identity_sha256) {
        throw 'selected P1B run evidence is not a matching PASS'
    }
    if ([string]$evidence.p0_dependency.status -cne 'PASS' -or
        [string]$evidence.p0_dependency.receipt_commit -cne '86fb1e4cc68efeb651e5362c4aca85c2827d8e4d' -or
        [string]$evidence.p1a_dependency.status -cne 'PASS' -or
        [string]$evidence.p1a_dependency.review_closure_commit -cne
            '9359c989fa63d4a300abc509e735b7e81a24a2ea') {
        throw 'selected P1B nested P1A/P0 dependency is not approved'
    }
    $verifyModule=Import-Module -Name (Join-Path $repository 'scripts\lib\VerifyEnv.psm1') -Force -PassThru
    try{$liveP1A=& "$($verifyModule.Name)\Get-P1BSelectedP1ADependency" -RepositoryRoot $repository}
    finally{Remove-Module -Name $verifyModule.Name -Force}
    foreach($field in @('status','pointer_path','pointer_sha256','acceptance_path','acceptance_sha256','acceptance_sequence','run_path',
        'run_evidence_sha256','seal_path','seal_sha256','environment_path','environment_sha256','source_identity_sha256',
        'verifier_sha256','schema_bundle_sha256','review_closure_commit')){
        if([string]$liveP1A[$field]-cne[string]$evidence.p1a_dependency.$field){throw "selected P1B nested P1A binding drifted: $field"}
    }
    $p1aPointer = Join-Path $repository ([string]$evidence.p1a_dependency.pointer_path).Replace('/', '\')
    $p1aAcceptance = Join-Path $repository ([string]$evidence.p1a_dependency.acceptance_path).Replace('/', '\')
    $p1aEvidence = Join-Path $repository (([string]$evidence.p1a_dependency.run_path + '/evidence.json').Replace('/', '\'))
    $p1aSeal = Join-Path $repository ([string]$evidence.p1a_dependency.seal_path).Replace('/', '\')
    $p1aEnvironment = Join-Path $repository ([string]$evidence.p1a_dependency.environment_path).Replace('/', '\')
    foreach ($binding in @(
            @($p1aPointer, $evidence.p1a_dependency.pointer_sha256),
            @($p1aAcceptance, $evidence.p1a_dependency.acceptance_sha256),
            @($p1aEvidence, $evidence.p1a_dependency.run_evidence_sha256),
            @($p1aSeal, $evidence.p1a_dependency.seal_sha256),
            @($p1aEnvironment, $evidence.p1a_dependency.environment_sha256))) {
        if (-not (Test-Path -LiteralPath $binding[0] -PathType Leaf) -or
            (Get-P2Sha256 -Path $binding[0]) -cne [string]$binding[1]) {
            throw 'selected P1B nested P1A artifact binding failed'
        }
    }
    $p1aPointerValue = [IO.File]::ReadAllText($p1aPointer, $script:P2Utf8NoBom) | ConvertFrom-Json
    if ([string]$p1aPointerValue.acceptance_sha256 -cne [string]$evidence.p1a_dependency.acceptance_sha256) {
        throw 'selected P1B nested P1A pointer no longer selects its acceptance'
    }
    $git = Get-Command git.exe -ErrorAction Stop
    $ancestor = & $git.Source merge-base --is-ancestor $script:P2ReviewCommit HEAD 2>$null
    if ($LASTEXITCODE -ne 0) { throw 'P1B owner-review closure commit is not an ancestor of HEAD' }
    return [pscustomobject][ordered]@{
        status = 'PASS'; pointer_path = 'docs/receipts/P1B/evidence.json'
        pointer_sha256 = Get-P2Sha256 -Path $pointerPath
        acceptance_path = 'docs/receipts/P1B/' + [string]$pointer.acceptance_path
        acceptance_sha256 = Get-P2Sha256 -Path $acceptancePath; acceptance_sequence = $sequence
        run_path = 'docs/receipts/P1B/' + [string]$acceptance.run_path
        run_evidence_sha256 = Get-P2Sha256 -Path $evidencePath
        seal_path = 'docs/receipts/P1B/' + [string]$acceptance.seal_path
        seal_sha256 = Get-P2Sha256 -Path $sealPath
        environment_path = 'docs/receipts/P1B/' + [string]$acceptance.environment_path
        environment_sha256 = Get-P2Sha256 -Path $environmentPath
        source_identity_sha256 = [string]$acceptance.source_identity_sha256
        verifier_sha256 = [string]$evidence.p1a_dependency.verifier_sha256
        schema_bundle_sha256 = [string]$evidence.p1a_dependency.schema_bundle_sha256
        review_closure_commit = $script:P2ReviewCommit
    }
}

function Assert-P2PassRun {
    param([Parameter(Mandatory)][string]$RunRoot, [Parameter(Mandatory)][string]$RunId,
        [Parameter(Mandatory)][string]$SourceIdentitySha256)
    Initialize-P2NativeInterop
    if ($RunId -cnotmatch '^[0-9]{8}T[0-9]{9}Z-[0-9a-f]{24}$' -or
        (Split-Path -Leaf $RunRoot) -cne $RunId -or -not (Test-P2Seal -RunRoot $RunRoot)) {
        throw 'accepted P2 run identity or seal is invalid'
    }
    $evidencePath = Join-Path $RunRoot 'evidence.json'; $decisionPath = Join-Path $RunRoot 'artifacts\decision.json'
    foreach ($path in @($evidencePath, $decisionPath)) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw 'accepted P2 run is incomplete' }
    }
    $evidence = [IO.File]::ReadAllText($evidencePath, $script:P2Utf8NoBom) | ConvertFrom-Json
    Assert-P2ClosedObject $evidence @('schema','phase_id','run_id','status','owner_review_status','started_at','finished_at','duration_ms',
        'invocation','source_identity_sha256','p1b_dependency','policy','dependency_inventory','fixture_manifest','host_state','candidates',
        'comparison','decision','cpu_isolation','commands','gates','errors','cleanup','seal') 'P2 phase evidence'
    if ([string]$evidence.schema -cne $script:P2Schema.Evidence -or
        [string]$evidence.phase_id -cne 'P2' -or [string]$evidence.run_id -cne $RunId -or
        [string]$evidence.status -cne 'PASS' -or [string]$evidence.owner_review_status -cne 'AWAITING_REVIEW' -or
        [string]$evidence.source_identity_sha256 -cne $SourceIdentitySha256 -or @($evidence.errors).Count -ne 0) {
        throw 'only a matching P2 PASS awaiting owner review may be accepted'
    }
    $startedAt=[DateTime]::MinValue;$finishedAt=[DateTime]::MinValue
    if(-not[DateTime]::TryParse([string]$evidence.started_at,[Globalization.CultureInfo]::InvariantCulture,[Globalization.DateTimeStyles]::RoundtripKind,[ref]$startedAt)-or
        -not[DateTime]::TryParse([string]$evidence.finished_at,[Globalization.CultureInfo]::InvariantCulture,[Globalization.DateTimeStyles]::RoundtripKind,[ref]$finishedAt)-or
        -not([string]$evidence.started_at).EndsWith('Z')-or-not([string]$evidence.finished_at).EndsWith('Z')-or$finishedAt-lt$startedAt-or
        [int64]$evidence.duration_ms-lt0){throw 'P2 evidence timestamps or duration are invalid'}
    Assert-P2ClosedObject $evidence.invocation @('argv','cwd') 'P2 invocation'
    $expectedInvocation=@('powershell','-NoProfile','-ExecutionPolicy','Bypass','-File','scripts/qualify-backend.ps1','-OutputRoot','docs/receipts/P2')
    if(($evidence.invocation.argv|ConvertTo-Json -Compress)-cne($expectedInvocation|ConvertTo-Json -Compress)-or[string]$evidence.invocation.cwd-cne'${REPO}'){
        throw 'P2 evidence invocation is not normative'
    }
    Assert-P2ClosedObject $evidence.gates @('p1b_dependency','input_stability','dependency_policy','fixture_determinism','cpu_isolation',
        'sm120_launch','bf16_roundtrip','correctness','synchronization','runtime_provenance','measurement_validity','redaction','cleanup') 'P2 gates'
    $resolved=@{}
    foreach($referenceName in @('policy','dependency_inventory','fixture_manifest','host_state','comparison','cpu_isolation')){
        $resolved[$referenceName]=Assert-P2FileReference -Reference $evidence.$referenceName -RunRoot $RunRoot -Name "P2 $referenceName"
    }
    $policy=[IO.File]::ReadAllText($resolved.policy,$script:P2Utf8NoBom)|ConvertFrom-Json
    if(($policy|ConvertTo-Json -Depth 64 -Compress)-cne((New-P2Policy)|ConvertTo-Json -Depth 64 -Compress)){throw 'P2 qualification policy artifact is not canonical'}
    $cpu=[IO.File]::ReadAllText($resolved.cpu_isolation,$script:P2Utf8NoBom)|ConvertFrom-Json
    if([string]$cpu.status-cne'PASS'-or[bool]$cpu.cuda_or_python_discovered-or@($cpu.forbidden_hits).Count-ne0){throw 'P2 CPU isolation artifact is not PASS'}
    $host=[IO.File]::ReadAllText($resolved.host_state,$script:P2Utf8NoBom)|ConvertFrom-Json
    Assert-P2ClosedObject $host @('schema','status','gpu','compute_capability','driver_version','cuda_toolkit_version','foreign_gpu_process_count','nvml_library','cuda_health_recoveries','benchmark_monitor_checks') 'P2 host state'
    if([string]$host.status-cne'PASS'-or[string]$host.gpu-cne'NVIDIA GeForce RTX 5090'-or[string]$host.compute_capability-cne'12.0'-or[int]$host.foreign_gpu_process_count-ne0){throw 'P2 host state artifact is invalid'}
    $hostRecoveryIds=[Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach($recovery in @($host.cuda_health_recoveries)){Assert-P2ClosedObject $recovery @('candidate_id','failed_command_id','trigger','result') 'CUDA health recovery'
        if([string]$recovery.candidate_id-notin@('burn-cubecl','candle','cudarc-fallback')-or[string]$recovery.failed_command_id-cnotmatch'^C[0-9]{2,4}$'-or
            [string]$recovery.trigger-notin@('candidate_result_fail','candidate_process_failure','candidate_build_process_failure','benchmark_result_fail','benchmark_process_failure','fallback_result_fail','fallback_process_failure')-or
            [string]$recovery.result-cne'PASS'-or-not$hostRecoveryIds.Add([string]$recovery.failed_command_id)){throw 'CUDA health recovery record is invalid or duplicated'}}
    $monitorCheckIds=[Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach($check in @($host.benchmark_monitor_checks)){
        Assert-P2ClosedObject $check @('candidate_id','measurement') 'benchmark monitor check'
        if([string]$check.candidate_id-notin@('burn-cubecl','candle')){throw 'benchmark monitor check candidate is invalid'}
        $measurement=$check.measurement
        Assert-P2ClosedObject $measurement @('command_id','round','workload','baseline_samples','baseline_interval_ms','sample_interval_ms','total_bytes','baseline_bytes','peak_bytes','delta_bytes','sample_count','max_gap_ms','baseline_foreign_process_count','maximum_foreign_process_count','error') 'benchmark monitor measurement'
        if(-not(Test-P2NvmlMeasurementPolicy $measurement $measurement)-or-not$monitorCheckIds.Add([string]$measurement.command_id)){throw 'benchmark monitor check is invalid or duplicated'}
    }
    $liveNvml=New-P2NvmlLibraryRecord -Path ([P2NvmlMonitor]::EnsureQualifiedLibrary()) -WindowsRoot $env:SystemRoot
    if(($liveNvml|ConvertTo-Json -Compress)-cne($host.nvml_library|ConvertTo-Json -Compress)){throw 'P2 NVML monitor library drifted from retained host state'}
    $sourcePath=Join-Path $RunRoot 'artifacts\source-identity.json'
    if(-not(Test-Path -LiteralPath $sourcePath -PathType Leaf)-or(Get-P2Sha256 $sourcePath)-cne$SourceIdentitySha256){throw 'P2 source identity artifact binding failed'}
    $p1bPath=Join-Path $RunRoot 'artifacts\p1b-dependency.json'
    if(-not(Test-Path -LiteralPath $p1bPath -PathType Leaf)){throw 'P2 retained P1B dependency is missing'}
    $p1b=[IO.File]::ReadAllText($p1bPath,$script:P2Utf8NoBom)|ConvertFrom-Json
    if(($p1b|ConvertTo-Json -Depth 64 -Compress)-cne($evidence.p1b_dependency|ConvertTo-Json -Depth 64 -Compress)-or[string]$p1b.status-cne'PASS'){
        throw 'P2 evidence does not bind its retained P1B dependency'
    }
    $repoRoot=Get-P2RepositoryRootFromRunRoot -RunRoot $RunRoot
    $selected=Get-P2SelectedP1BDependency -RepositoryRoot $repoRoot
    if(($selected|ConvertTo-Json -Depth 64 -Compress)-cne($p1b|ConvertTo-Json -Depth 64 -Compress)){throw 'P2 retained P1B dependency is no longer selected'}
    $p1bEnvironment=[IO.File]::ReadAllText((Join-Path $repoRoot ([string]$p1b.environment_path).Replace('/','\')),$script:P2Utf8NoBom)|ConvertFrom-Json
    $expectedModules=@{[string]$p1bEnvironment.driver.library.path=[string]$p1bEnvironment.driver.library.sha256}
    foreach($dll in @($p1bEnvironment.cuda_toolkit.runtime_dlls)){$expectedModules[[string]$dll.path]=[string]$dll.sha256}
    $dependency=[IO.File]::ReadAllText($resolved.dependency_inventory,$script:P2Utf8NoBom)|ConvertFrom-Json
    $candidateValues=[Collections.Generic.List[object]]::new()
    $candidateRefs=@($evidence.candidates);if($candidateRefs.Count-ne2-or(@($candidateRefs.candidate_id)-join',')-cne'burn-cubecl,candle'){throw 'P2 PASS candidate reference order is invalid'}
    foreach($candidateRef in $candidateRefs){
        Assert-P2ClosedObject $candidateRef @('candidate_id','role','status','path','sha256','bytes') 'candidate evidence reference'
        if([string]$candidateRef.status-ceq'NOT_RUN'){if($null-ne$candidateRef.sha256-or$null-ne$candidateRef.bytes){throw 'NOT_RUN candidate has artifact bytes'};continue}
        $basic=[pscustomobject][ordered]@{path=$candidateRef.path;sha256=$candidateRef.sha256;bytes=$candidateRef.bytes}
        $path=Assert-P2FileReference $basic $RunRoot 'candidate aggregate';$aggregate=[IO.File]::ReadAllText($path,$script:P2Utf8NoBom)|ConvertFrom-Json
        $null=Assert-P2CandidateAggregate $aggregate $RunRoot
        if([string]$aggregate.status-ceq'PASS'){
            foreach($moduleRecord in @($aggregate.runtime_provenance.loaded_modules)){
                $moduleLeaf=Split-Path -Leaf ([string]$moduleRecord.path)
                if([string]$moduleRecord.path-cnotmatch '^\$\{CUDA_TOOLKIT\}/(?:bin/x64/)?[^/]+\.dll$|^\$\{WINDOWS\}/System32/(?:nvcuda|nvml)\.dll$'-or
                    ($aggregate.candidate_id-ceq'candle'-and$moduleLeaf-imatch'^cudnn')-or$moduleLeaf-imatch'^nccl'){
                    throw 'candidate retained runtime module path violates the qualified boundary'
                }
                if($moduleLeaf-imatch'^(?:nvcuda|cudart|cublas|cublaslt)[^/\\]*\.dll$'){
                    if(-not$expectedModules.ContainsKey([string]$moduleRecord.path)-or[string]$moduleRecord.sha256-cne[string]$expectedModules[[string]$moduleRecord.path]){throw 'candidate required runtime module path/hash is not P1B-bound'}
                }
            }
            $rawBenchmarks=[Collections.Generic.List[object]]::new()
            foreach($ref in @($aggregate.benchmark_rounds)){$rawBenchmarks.Add((Assert-P2InvocationReference -Reference $ref -RunRoot $RunRoot -CandidateId ([string]$aggregate.candidate_id)))}
            $graph=$dependency.candidate_graphs.([string]$aggregate.candidate_id)
            if($null-eq$graph){throw 'candidate dependency graph is missing from retained inventory'}
            $peak=[int64](($aggregate.nvml_measurements|Measure-Object delta_bytes -Maximum).Maximum)
            $recomputed=Get-P2CandidateComparison -CandidateId ([string]$aggregate.candidate_id) -BenchmarkResults @($rawBenchmarks) `
                -LockedDependencyCount ([int]$graph.package_count) -ObservedPeakBytes $peak
            $expectedSummary=[ordered]@{geomean_fwbw_p50_ns=$recomputed.comparison.geomean_fwbw_p50_ns;geomean_fwbw_p95_ns=$recomputed.comparison.geomean_fwbw_p95_ns
                observed_peak_bytes=$peak;locked_dependency_count=[int]$graph.package_count
                max_nvml_gap_ms=[double](($aggregate.nvml_measurements|Measure-Object max_gap_ms -Maximum).Maximum)
                foreign_process_count=[int](($aggregate.nvml_measurements|Measure-Object maximum_foreign_process_count -Maximum).Maximum)}
            if(($expectedSummary|ConvertTo-Json -Compress)-cne($aggregate.summary|ConvertTo-Json -Compress)){throw 'candidate summary is not derivable from retained raw measurements'}
        }
        if([string]$aggregate.candidate_id-cne[string]$candidateRef.candidate_id-or[string]$aggregate.role-cne[string]$candidateRef.role-or[string]$aggregate.status-cne[string]$candidateRef.status){throw 'candidate reference does not match its aggregate'}
        $candidateValues.Add($aggregate)
    }
    if(@($candidateValues|Where-Object{$_.candidate_id-in@('burn-cubecl','candle')-and$_.status-eq'PASS'}).Count-lt1){throw 'P2 evidence has no passing framework'}
    $decisionResolved = Assert-P2DecisionReference -Reference $evidence.decision -RunRoot $RunRoot -Name 'P2 decision'
    $decision=[IO.File]::ReadAllText($decisionResolved,$script:P2Utf8NoBom)|ConvertFrom-Json;$null=Assert-P2Decision $decision $RunId
    $derived=Select-P2Candidate @($candidateValues|Where-Object role -eq framework|ForEach-Object{[pscustomobject]@{candidate_id=$_.candidate_id;status=$_.status;comparison=$_.summary}})
    $comparison=[IO.File]::ReadAllText($resolved.comparison,$script:P2Utf8NoBom)|ConvertFrom-Json
    $expectedComparison=[ordered]@{schema='python-slm-p2-backend-comparison-v1';policy_id='backend-qualification-v1'
        candidates=@($candidateValues|Where-Object role -eq framework|ForEach-Object{[pscustomobject]@{candidate_id=$_.candidate_id;status=$_.status;comparison=$_.summary}})
        selection=$derived}
    if(($comparison|ConvertTo-Json -Depth 32 -Compress)-cne($expectedComparison|ConvertTo-Json -Depth 32 -Compress)){throw 'P2 comparison artifact is not derivable from retained candidates'}
    if($derived.selected_candidate_id-cne$decision.selected_candidate_id-or$derived.criterion-cne$decision.selection_criterion){throw 'P2 decision is not derivable from candidate aggregates'}
    foreach($name in @('p1b_dependency','input_stability','dependency_policy','fixture_determinism','cpu_isolation','sm120_launch',
        'bf16_roundtrip','correctness','synchronization','runtime_provenance','measurement_validity','redaction','cleanup')){
        Assert-P2ClosedObject $evidence.gates.$name @('status','detail') "P2 gate $name"
        if([string]$evidence.gates.$name.status-cne'PASS'){throw "P2 PASS gate is not closed: $name"}
        if([string]::IsNullOrWhiteSpace([string]$evidence.gates.$name.detail)){throw "P2 PASS gate has no detail: $name"}
    }
    Assert-P2ClosedObject $evidence.cleanup @('attempted','temporary_root_removed','process_trees_terminated') 'P2 cleanup'
    if(-not[bool]$evidence.cleanup.attempted-or-not[bool]$evidence.cleanup.temporary_root_removed-or-not[bool]$evidence.cleanup.process_trees_terminated){throw 'P2 cleanup assertions are incomplete'}
    Assert-P2ClosedObject $evidence.seal @('path','entries','coverage_rule') 'P2 seal descriptor'
    if([string]$evidence.seal.path-cne'SHA256SUMS'-or[string]$evidence.seal.coverage_rule-cne'all_run_files_except_seal'-or[int]$evidence.seal.entries-lt1){throw 'P2 seal descriptor is invalid'}
    $failedCommandOwners=@{};$recoveredCommandIds=[Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach($aggregate in @($candidateValues|Where-Object status -eq FAIL)){
        foreach($failure in @($aggregate.failures)){
            if($null-eq$failure.command_id){continue};$failureId=[string]$failure.command_id
            if($failedCommandOwners.ContainsKey($failureId)-and[string]$failedCommandOwners[$failureId]-cne[string]$aggregate.candidate_id){throw 'candidate failure command is claimed by multiple candidates'}
            $failedCommandOwners[$failureId]=[string]$aggregate.candidate_id
            if([string]$failure.code-ceq'CUDA_HEALTH_RECOVERY_PASSED'){[void]$recoveredCommandIds.Add($failureId)}
        }
    }
    if((@($recoveredCommandIds|Sort-Object)-join',')-cne(@($hostRecoveryIds|Sort-Object)-join',')){throw 'candidate recovery assertions do not match retained successful CUDA health probes'}
    $commandMap=@{};$primaryCommandIndex=0;$lastPrimaryId=$null
    foreach($command in @($evidence.commands)){
        Assert-P2ClosedObject $command @('id','argv','cwd','environment_delta_names','configuration_sha256','expected_exit_codes','expectation_met','exit_code','duration_ms','status','stdout','stderr') 'P2 command evidence'
        $id=[string]$command.id
        if($id-cmatch'^C1(?<parent>[0-9]{3})$'){
            $parentNumber=[int]$Matches.parent;$expectedParent='C'+$parentNumber.ToString('00')
            if($lastPrimaryId-cne$expectedParent-or@($command.argv).Count-lt2-or[string]$command.argv[1]-cne'tree'){throw 'P2 auxiliary dependency command is not paired with its parent'}
        }else{$primaryCommandIndex++;$expectedId='C'+$primaryCommandIndex.ToString('00');if($id-cne$expectedId){throw 'P2 primary command order is invalid'};$lastPrimaryId=$id}
        if($commandMap.ContainsKey($id)-or@($command.argv).Count-lt1-or[string]::IsNullOrWhiteSpace([string]$command.cwd)-or
            @($command.expected_exit_codes).Count-lt1-or@($command.expected_exit_codes|Sort-Object -Unique).Count-ne@($command.expected_exit_codes).Count-or[int64]$command.duration_ms-lt0-or
            @($command.environment_delta_names|Sort-Object -Unique).Count-ne@($command.environment_delta_names).Count){throw 'P2 PASS contains an invalid command protocol record'}
        [void](Assert-P2CommandOutcome -Command $command -FailedCommandOwners $failedCommandOwners -RecoveredCommandIds $recoveredCommandIds)
        $commandMap[$id]=$command
        Assert-P2Sha256 $command.configuration_sha256 'command configuration hash'
        $configurationPath=Resolve-P2ContainedFile -Root $RunRoot -Relative ("commands/$($command.id).configuration.json") -Name 'command configuration'
        if((Get-P2Sha256 $configurationPath)-cne[string]$command.configuration_sha256){throw 'P2 command configuration hash does not match its sealed configuration'}
        $configuration=[IO.File]::ReadAllText($configurationPath,$script:P2Utf8NoBom)|ConvertFrom-Json
        Assert-P2ClosedObject $configuration @('argv','cwd','environment_delta_names','effective_build_environment') 'command configuration'
        if(($configuration.argv|ConvertTo-Json -Compress)-cne($command.argv|ConvertTo-Json -Compress)-or[string]$configuration.cwd-cne[string]$command.cwd-or
            ($configuration.environment_delta_names|ConvertTo-Json -Compress)-cne($command.environment_delta_names|ConvertTo-Json -Compress)){
            throw 'P2 command configuration is not bound to its command record'
        }
        [void](Assert-P2CommandEnvironmentPolicy -Command $command -Configuration $configuration)
        if([string]$command.stdout.path-cne"commands/$($command.id).stdout.txt"-or[string]$command.stderr.path-cne"commands/$($command.id).stderr.txt"){throw 'P2 command transcript path is not canonical'}
        $null=Assert-P2FileReference $command.stdout $RunRoot 'command stdout';$null=Assert-P2FileReference $command.stderr $RunRoot 'command stderr'}
    foreach($aggregate in @($candidateValues)){
        foreach($reference in @($aggregate.cpu_smoke,$aggregate.allocation,$aggregate.correctness)+@($aggregate.benchmark_rounds)){
            if($null-ne$reference-and(-not$commandMap.ContainsKey([string]$reference.command_id))){throw 'candidate invocation references an unknown command'}
        }
        foreach($measurement in @($aggregate.nvml_measurements)){if(-not$commandMap.ContainsKey([string]$measurement.command_id)){throw 'NVML measurement references an unknown command'}
            if(-not$monitorCheckIds.Contains([string]$measurement.command_id)){throw 'candidate NVML measurement is absent from run-global monitor checks'}}
    }
    foreach($id in $monitorCheckIds){if(-not$commandMap.ContainsKey([string]$id)){throw 'run-global monitor check references an unknown command'}}
    [void](Assert-P2AcceptedCommandProtocol -Commands @($evidence.commands) -Aggregates @($candidateValues) -FailedCommandOwners $failedCommandOwners)
    if([int]$evidence.seal.entries-ne@((Get-Content (Join-Path $RunRoot 'SHA256SUMS'))).Count){throw 'P2 evidence seal count is wrong'}
    [void](Test-P2ReceiptRedaction -RunRoot $RunRoot)
    return [pscustomobject][ordered]@{
        evidence_sha256 = Get-P2Sha256 -Path $evidencePath
        seal_sha256 = Get-P2Sha256 -Path (Join-Path $RunRoot 'SHA256SUMS')
        decision_sha256 = Get-P2Sha256 -Path $decisionPath
    }
}

function Get-P2RepositoryRootFromRunRoot {
    param([Parameter(Mandatory)][string]$RunRoot)
    $cursor=[IO.Path]::GetFullPath($RunRoot)
    $expected=@('runs','P2','receipts','docs')
    if((Split-Path -Leaf $cursor)-cnotmatch'^[0-9]{8}T[0-9]{9}Z-[0-9a-f]{24}$'){throw 'P2 run root leaf is invalid'}
    $cursor=Split-Path -Parent $cursor
    foreach($segment in $expected){
        if((Split-Path -Leaf $cursor)-cne$segment){throw 'P2 run root does not use the canonical repository receipt layout'}
        $cursor=Split-Path -Parent $cursor
    }
    return [IO.Path]::GetFullPath($cursor)
}

function Get-P2ArgumentValue {
    param([Parameter(Mandatory)][object[]]$Argv,[Parameter(Mandatory)][string]$Name)
    $indices=@(0..($Argv.Count-1)|Where-Object{[string]$Argv[$_]-ceq$Name})
    if($indices.Count-ne1-or$indices[0]+1-ge$Argv.Count){throw "command argument is missing or duplicated: $Name"}
    return [string]$Argv[$indices[0]+1]
}

function Assert-P2CandidateInvocationCommand {
    param([Parameter(Mandatory)]$Command,[Parameter(Mandatory)]$Reference,[Parameter(Mandatory)][string]$CandidateId)
    $binary=@{'burn-cubecl'='p2-burn-cubecl.exe';candle='p2-candle.exe';'cudarc-fallback'='p2-cudarc-fallback.exe'}[$CandidateId]
    if([string]$Command.argv[0]-cne$binary-or@($Command.argv).Count-ne9-or[string]$Command.argv[1]-cne'--mode'-or
        [string]$Command.argv[2]-cne[string]$Reference.mode-or[string]$Command.argv[3]-cne'--workload'-or
        [string]$Command.argv[4]-cne[string]$Reference.workload-or[string]$Command.argv[5]-cne'--fixture-dir'-or
        [string]$Command.argv[6]-cnotmatch'^\$\{TEMP\}/fixtures$'-or[string]$Command.argv[7]-cne'--output'-or
        [string]$Command.argv[8]-cnotmatch'^\$\{TEMP\}/invocations/(?:burn-cubecl|candle|cudarc-fallback)/(?:cpu-smoke-correctness|correctness-(?:allocation|correctness)|round-[1-4]-(?:projection|ffn-expansion))/raw-result\.json$'){
        throw 'candidate command does not match its retained invocation reference'
    }
    $suffix=if($null-eq$Reference.round){"$($Reference.mode)-$($Reference.workload)"}else{"round-$($Reference.round)-$($Reference.workload)"}
    if([string]$Command.cwd-cne"`${TEMP}/invocations/$CandidateId/$suffix"){throw 'candidate command cache/work directory is not invocation-owned'}
}

function Get-P2AttemptedBenchmarkSlot {
    param([Parameter(Mandatory)]$Command,[Parameter(Mandatory)][string]$CandidateId)
    $argv=@($Command.argv);$binary=@{'burn-cubecl'='p2-burn-cubecl.exe';candle='p2-candle.exe'}[$CandidateId]
    if($null-eq$binary-or@($argv).Count-ne9-or[string]$argv[0]-cne$binary-or[string]$argv[1]-cne'--mode'-or[string]$argv[2]-cne'benchmark'-or
        [string]$argv[3]-cne'--workload'-or[string]$argv[4]-notin@('projection','ffn-expansion')-or[string]$argv[5]-cne'--fixture-dir'-or
        [string]$argv[6]-cne'${TEMP}/fixtures'-or[string]$argv[7]-cne'--output'){throw 'failed benchmark command is not a canonical candidate invocation'}
    $pattern='^\$\{TEMP\}/invocations/'+[regex]::Escape($CandidateId)+'/round-(?<round>[1-4])-(?<workload>projection|ffn-expansion)/raw-result\.json$'
    if([string]$argv[8]-cnotmatch$pattern){throw 'failed benchmark output is not in a canonical round-owned path'}
    $round=[int]$Matches.round;$workload=[string]$Matches.workload
    if($workload-cne[string]$argv[4]-or[string]$Command.cwd-cne"`${TEMP}/invocations/$CandidateId/round-$round-$workload"){throw 'failed benchmark cache/output/workload binding is inconsistent'}
    $allowedRounds=if($CandidateId-ceq'burn-cubecl'){@(1,4)}else{@(2,3)};if($round-notin$allowedRounds){throw 'failed benchmark occupies another candidate round'}
    return [pscustomobject]@{candidate_id=$CandidateId;round=$round;workload=$workload}
}

function Assert-P2AttemptedBenchmarkSchedule {
    param([Parameter(Mandatory)][object[]]$Rows)
    $canonical=@('burn-cubecl:1:projection','candle:2:projection','candle:3:projection','burn-cubecl:4:projection',
        'burn-cubecl:1:ffn-expansion','candle:2:ffn-expansion','candle:3:ffn-expansion','burn-cubecl:4:ffn-expansion')
    $actual=@($Rows|Sort-Object order|ForEach-Object{"$($_.candidate_id):$($_.round):$($_.workload)"})
    $eligible=@($Rows.candidate_id|Sort-Object -Unique)
    $expected=@($canonical|Where-Object{$candidate=($_-split':')[0];$candidate-in$eligible})
    if(($actual-join',')-cne($expected-join',')){throw 'attempted benchmark commands do not occupy the canonical per-workload ABBA schedule'}
    return $true
}

function Assert-P2CommandOutcome {
    param([Parameter(Mandatory)]$Command,[Parameter(Mandatory)][hashtable]$FailedCommandOwners,
        [Parameter(Mandatory)][Collections.Generic.HashSet[string]]$RecoveredCommandIds)
    $id=[string]$Command.id;$candidateLocal=$FailedCommandOwners.ContainsKey($id)
    $strictSuccess=[bool]$Command.expectation_met-and$null-ne$Command.exit_code-and[int]$Command.exit_code-eq0-and[string]$Command.status-ceq'PASS'
    if($strictSuccess){ return $true }
    if(-not$candidateLocal-or[string]$Command.status-notin@('FAIL','TIMEOUT','OOM','CRASH')){throw 'P2 PASS contains a non-candidate command failure'}
    $isGpuCandidateInvocation=[string]$Command.argv[0]-cmatch'^p2-(?:burn-cubecl|candle|cudarc-fallback)\.exe$'-and
        @($Command.argv).Count-gt2-and[string]$Command.argv[1]-ceq'--mode'-and[string]$Command.argv[2]-cne'cpu-smoke'
    $isExceptionalCandidateBuild=[string]$Command.argv[0]-ceq'cargo.exe'-and@($Command.argv).Count-gt1-and[string]$Command.argv[1]-ceq'build'-and
        [string]$Command.status-in@('TIMEOUT','OOM','CRASH')
    if(($isGpuCandidateInvocation-or$isExceptionalCandidateBuild)-and-not$RecoveredCommandIds.Contains($id)){throw 'failed GPU/candidate-build command lacks a sealed successful health recovery assertion'}
    return $true
}

function Assert-P2CommandEnvironmentPolicy {
    param([Parameter(Mandatory)]$Command,[Parameter(Mandatory)]$Configuration)
    $effective=$Configuration.effective_build_environment
    if($null-eq$effective){throw 'command configuration omits its effective build environment'}
    $properties=@{};foreach($property in $effective.PSObject.Properties){$properties[$property.Name]=[string]$property.Value}
    $require={param([string]$Name,[string]$Expected)
        if(-not$properties.ContainsKey($Name)-or[string]$properties[$Name]-cne$Expected){throw "command environment policy mismatch: $Name"}}
    & $require CARGO_NET_OFFLINE 'true';& $require CARGO_INCREMENTAL '0';& $require CARGO_TERM_COLOR 'never'
    foreach($name in @('RUSTC_WRAPPER','RUSTC_WORKSPACE_WRAPPER','RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUSTDOCFLAGS','CARGO_ENCODED_RUSTDOCFLAGS',
            'PYTHONHOME','PYTHONPATH','PYTHONNOUSERSITE','VIRTUAL_ENV','CONDA_PREFIX','PIP_CONFIG_FILE')){&$require $name '<CLEARED>'}
    if(@($properties.Keys|Where-Object{$_-match'(?i)(token|secret|password|credential|api[_-]?key)'}).Count-ne0){throw 'command configuration retains a credential-bearing environment key'}
    $permittedActive=@('CARGO_NET_OFFLINE','CARGO_INCREMENTAL','CARGO_TERM_COLOR','CARGO_TARGET_DIR','CARGO_HOME',
        'RUSTUP_HOME','RUSTUP_TOOLCHAIN','CUDA_PATH','PATH','TEMP','TMP','USERPROFILE','HOME','LIB','INCLUDE','LIBPATH',
        'WindowsSdkDir','WindowsSDKVersion','VCToolsInstallDir','VisualStudioVersion','CUDA_CACHE_PATH')
    foreach($name in @($properties.Keys|Where-Object{$_-match'^(?i)(?:CARGO|RUST|CUDA|CUDNN|NVCC|PYTHON|PIP|VIRTUAL_ENV|CONDA_PREFIX)'})){
        if($name-notin$permittedActive-and[string]$properties[$name]-cne'<CLEARED>'){throw "unapproved active build environment variable: $name"}
    }
    $argv=@($Command.argv);$isCandidate=[string]$argv[0]-cmatch'^p2-(?:burn-cubecl|candle|cudarc-fallback)\.exe$'
    $isCpu=$false
    if($isCandidate){
        $isCpu=[string]$argv[2]-ceq'cpu-smoke';$cwd=[string]$Command.cwd
        foreach($name in @('USERPROFILE','HOME','TEMP','TMP')){&$require $name $cwd}
        &$require CUDA_CACHE_PATH ($cwd+'/cuda-cache')
    }elseif([string]$argv[0]-ceq'cargo.exe'){
        if([string]$argv[1]-in@('fmt','clippy','test')){&$require CARGO_TARGET_DIR '${TEMP}/root-target';$isCpu=$true}
        elseif($argv-contains'--features'){$isCpu=(Get-P2ArgumentValue $argv '--features')-ceq'cpu'}
        elseif($argv-contains'p2-backend-common'){$isCpu=$true}
    }elseif([string]$argv[0]-ceq'dumpbin.exe'){$isCpu=[string]$argv[3]-cmatch'^\$\{TEMP\}/cpu-target/'}
    if($isCpu){
        foreach($name in @('CUDA_PATH','CUDA_HOME','CUDA_ROOT','CUDA_TOOLKIT_ROOT_DIR','CUDNN_PATH','CUDNN_ROOT','NVCC','NVCC_PREPEND_FLAGS','NVCC_APPEND_FLAGS')){&$require $name '<CLEARED>'}
        if(-not$properties.ContainsKey('Path')-or[string]$properties.Path-cnotmatch'^\$\{TEMP\}/cpu-canaries;\$\{TEMP\}/python-canaries(?:;|$)'-or
            [string]$properties.Path-cmatch'(?:^|;)\$\{CUDA_TOOLKIT\}(?:/|;|$)'){throw 'CPU command PATH does not enforce executable canaries and CUDA-toolkit isolation'}
    }elseif($isCandidate-or[string]$argv[0]-in@('cargo.exe','dumpbin.exe')){
        &$require CUDA_PATH '${CUDA_TOOLKIT}'
        if(-not$properties.ContainsKey('Path')-or[string]$properties.Path-cnotmatch'^\$\{TEMP\}/python-canaries(?:;|$)'){throw 'GPU command PATH does not begin with the executable Python canaries'}
    }
    return $true
}

function Assert-P2AcceptedCommandProtocol {
    param([Parameter(Mandatory)][object[]]$Commands,[Parameter(Mandatory)][object[]]$Aggregates,[hashtable]$FailedCommandOwners=@{})
    if($Commands.Count-lt8){throw 'P2 accepted command protocol is incomplete'}
    $claimed=[Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $rootExpected=@(
        @('cargo.exe','fmt','--all','--','--check'),
        @('cargo.exe','clippy','--locked','--all-targets','--features','cpu-reference','--','-D','warnings'),
        @('cargo.exe','test','--locked','--features','cpu-reference'))
    for($i=0;$i-lt3;$i++){if(($Commands[$i].argv|ConvertTo-Json -Compress)-cne($rootExpected[$i]|ConvertTo-Json -Compress)-or[string]$Commands[$i].cwd-cne'${REPO}'){
            throw 'root CPU quality-gate command protocol changed'};[void]$claimed.Add([string]$Commands[$i].id)}
    $passingIds=@($Aggregates|Where-Object status -eq PASS|ForEach-Object{[string]$_.candidate_id})
    foreach($candidateId in @('burn-cubecl','candle')){
        $package=if($candidateId-ceq'burn-cubecl'){'p2-burn-cubecl'}else{'p2-candle'}
        $builds=@($Commands|Where-Object{@($_.argv).Count-gt2-and[string]$_.argv[1]-ceq'build'-and($_.argv-contains$package)})
        if($builds.Count-ne2){throw "$candidateId does not have exactly two isolated build commands"}
        foreach($mode in @('cpu','cuda')){
            $build=@($builds|Where-Object{try{(Get-P2ArgumentValue -Argv @($_.argv) -Name '--features')-ceq$mode}catch{$false}})
            if($build.Count-ne1){throw "$candidateId $mode build feature selection is invalid"};$argv=@($build[0].argv)
            foreach($flag in @('-vv','--locked','--offline','--release','--manifest-path','--package','--no-default-features','--features','--target-dir')){if($flag-notin$argv){throw "$candidateId $mode build omitted $flag"}}
            if((Get-P2ArgumentValue $argv '--package')-cne$package-or(Get-P2ArgumentValue $argv '--manifest-path')-cne'${REPO}/experiments/p2-backends/Cargo.toml'-or
                (Get-P2ArgumentValue $argv '--target-dir')-cnotmatch("^\$\{TEMP\}/"+$mode+"-target/"+[regex]::Escape($candidateId)+"$")){throw "$candidateId $mode build path/package binding is invalid"}
            if($mode-ceq'cuda'){if((Get-P2ArgumentValue $argv '--target')-cne'x86_64-pc-windows-msvc'){throw "$candidateId CUDA build target is invalid"}}
            elseif('--target'-in$argv){throw "$candidateId CPU build unexpectedly declares a CUDA/MSVC target override"}
            [void]$claimed.Add([string]$build[0].id)
        }
        $trees=@($Commands|Where-Object{@($_.argv).Count-gt2-and[string]$_.argv[1]-ceq'tree'-and($_.argv-contains$package)})
        foreach($tree in $trees){$argv=@($tree.argv)
            foreach($flag in @('--locked','--offline','--manifest-path','--package','--no-default-features','--features','--target','-e','--prefix','--format')){if($flag-notin$argv){throw "$candidateId dependency tree omitted $flag"}}
            if((Get-P2ArgumentValue $argv '--manifest-path')-cne'${REPO}/experiments/p2-backends/Cargo.toml'-or(Get-P2ArgumentValue $argv '--package')-cne$package-or
                (Get-P2ArgumentValue $argv '--target')-cne'x86_64-pc-windows-msvc'-or(Get-P2ArgumentValue $argv '--features')-notin@('cpu','cuda')-or
                (Get-P2ArgumentValue $argv '-e')-notin@('features','normal,build')){throw "$candidateId dependency-tree role is invalid"}
            [void]$claimed.Add([string]$tree.id)
        }
        $cudaFeature=@($trees|Where-Object{(Get-P2ArgumentValue @($_.argv) '--features')-ceq'cuda'-and(Get-P2ArgumentValue @($_.argv) '-e')-ceq'features'})
        $cudaClosure=@($trees|Where-Object{(Get-P2ArgumentValue @($_.argv) '--features')-ceq'cuda'-and(Get-P2ArgumentValue @($_.argv) '-e')-ceq'normal,build'})
        $cpuFeature=@($trees|Where-Object{(Get-P2ArgumentValue @($_.argv) '--features')-ceq'cpu'-and(Get-P2ArgumentValue @($_.argv) '-e')-ceq'features'})
        $pe=@($Commands|Where-Object{@($_.argv).Count-eq4-and[string]$_.argv[0]-ceq'dumpbin.exe'-and[string]$_.argv[1]-ceq'/HEADERS'-and
                [string]$_.argv[2]-ceq'/DEPENDENTS'-and[string]$_.argv[3]-cmatch("^\$\{TEMP\}/(?:cpu|cuda)-target/"+[regex]::Escape($candidateId)+"/.+/p2-"+[regex]::Escape(($candidateId-replace'-cubecl','-cubecl'))+"\.exe$")})
        foreach($record in $pe){[void]$claimed.Add([string]$record.id)}
        if($candidateId-in$passingIds-and($cudaFeature.Count-ne1-or$cudaClosure.Count-ne1-or$cpuFeature.Count-ne1-or$pe.Count-ne2)){
            throw "$candidateId passing protocol lacks exact CPU/CUDA graph or PE audits"
        }
    }
    $fixture=@($Commands|Where-Object{@($_.argv).Count-gt2-and[string]$_.argv[1]-ceq'run'-and($_.argv-contains'p2-backend-common')})
    if($fixture.Count-ne1-or'--locked'-notin$fixture[0].argv-or'--offline'-notin$fixture[0].argv-or'--release'-notin$fixture[0].argv-or
        (Get-P2ArgumentValue @($fixture[0].argv) '--package')-cne'p2-backend-common'){throw 'canonical fixture-generation command is missing'}
    [void]$claimed.Add([string]$fixture[0].id)
    $commandMap=@{};$order=@{};for($i=0;$i-lt$Commands.Count;$i++){$commandMap[[string]$Commands[$i].id]=$Commands[$i];$order[[string]$Commands[$i].id]=$i}
    $benchmarkRows=[Collections.Generic.List[object]]::new()
    foreach($aggregate in $Aggregates){
        foreach($reference in @($aggregate.cpu_smoke,$aggregate.allocation,$aggregate.correctness)+@($aggregate.benchmark_rounds)){
            if($null-eq$reference){continue};if(-not$commandMap.ContainsKey([string]$reference.command_id)){throw 'candidate reference command is absent from accepted protocol'}
            Assert-P2CandidateInvocationCommand $commandMap[[string]$reference.command_id] $reference ([string]$aggregate.candidate_id)
            [void]$claimed.Add([string]$reference.command_id)
            if([string]$reference.mode-ceq'benchmark'){$benchmarkRows.Add([pscustomobject]@{candidate_id=[string]$aggregate.candidate_id;round=[int]$reference.round
                        workload=[string]$reference.workload;command_id=[string]$reference.command_id;order=[int]$order[[string]$reference.command_id]})}
        }
    }
    $passing=@($Aggregates|Where-Object{$_.candidate_id-in@('burn-cubecl','candle')-and$_.status-ceq'PASS'})
    $orderedRows=@($benchmarkRows|Sort-Object order)
    if($passing.Count-eq2){$rows=@($orderedRows|ForEach-Object{[pscustomobject]@{candidate_id=$_.candidate_id;round=$_.round;workload=$_.workload}});[void](Assert-P2AbbaOrder $rows)}
    elseif($passing.Count-eq1){$id=[string]$passing[0].candidate_id;$rounds=if($id-ceq'burn-cubecl'){'1:projection,4:projection,1:ffn-expansion,4:ffn-expansion'}else{'2:projection,3:projection,2:ffn-expansion,3:ffn-expansion'}
        if((@($orderedRows|Where-Object candidate_id -eq $id|ForEach-Object{"$($_.round):$($_.workload)"})-join',')-cne$rounds){throw 'single passing framework timing order is invalid'}}
    foreach($failedId in @($FailedCommandOwners.Keys)){
        if(-not$commandMap.ContainsKey([string]$failedId)){throw 'failed candidate aggregate references an absent command'}
        $failed=$commandMap[[string]$failedId];$owner=[string]$FailedCommandOwners[$failedId];$expectedBinary=@{'burn-cubecl'='p2-burn-cubecl.exe';candle='p2-candle.exe';'cudarc-fallback'='p2-cudarc-fallback.exe'}[$owner]
        if([string]$failed.argv[0]-ceq$expectedBinary-and@($failed.argv).Count-eq9-and[string]$failed.argv[1]-ceq'--mode'-and
            [string]$failed.argv[2]-in@('cpu-smoke','correctness','benchmark')-and[string]$failed.argv[3]-ceq'--workload'-and
            [string]$failed.argv[4]-in@('allocation','correctness','projection','ffn-expansion')-and[string]$failed.argv[5]-ceq'--fixture-dir'-and
            [string]$failed.argv[6]-ceq'${TEMP}/fixtures'-and[string]$failed.argv[7]-ceq'--output'-and[string]$failed.cwd-cmatch("^\$\{TEMP\}/invocations/"+[regex]::Escape($owner)+"/")){
            if([string]$failed.argv[2]-ceq'benchmark'){
                $slot=Get-P2AttemptedBenchmarkSlot -Command $failed -CandidateId $owner
                if(@($benchmarkRows|Where-Object command_id -eq $failedId).Count-eq0){$benchmarkRows.Add([pscustomobject]@{candidate_id=$slot.candidate_id;round=$slot.round;workload=$slot.workload;command_id=$failedId;order=[int]$order[$failedId]})}
            }
            [void]$claimed.Add([string]$failedId)
        }
    }
    $allAttempted=@($benchmarkRows|Sort-Object order)
    [void](Assert-P2AttemptedBenchmarkSchedule -Rows $allAttempted)
    $unclaimed=@($Commands|Where-Object{-not$claimed.Contains([string]$_.id)})
    if($unclaimed.Count-ne0){throw "accepted P2 command protocol contains unclassified commands: $(@($unclaimed.id)-join',')"}
    return $true
}

function Assert-P2Acceptance {
    param([Parameter(Mandatory)]$Acceptance, [int]$Sequence, [AllowNull()][string]$PreviousHash,
        [Parameter(Mandatory)][string]$OutputRoot)
    Assert-P2ClosedObject -Value $Acceptance -Fields @(
        'schema', 'phase_id', 'sequence', 'status', 'acceptance_kind',
        'required_approvals', 'run_path', 'run_evidence_sha256', 'seal_path', 'seal_sha256',
        'decision_path', 'decision_sha256', 'source_identity_sha256',
        'previous_acceptance_sha256', 'created_at') -Name 'P2 acceptance'
    if ([string]$Acceptance.schema -cne $script:P2Schema.Acceptance -or
        [string]$Acceptance.phase_id -cne 'P2' -or [int]$Acceptance.sequence -ne $Sequence -or
        [string]$Acceptance.status -cne 'PASS' -or
        [string]$Acceptance.acceptance_kind -cne 'automatic_machine_qualification' -or
        @($Acceptance.required_approvals).Count -ne 0 -or
        [string]$Acceptance.previous_acceptance_sha256 -cne [string]$PreviousHash -or
        [string]$Acceptance.run_path -cnotmatch '^runs/(?<run>[0-9]{8}T[0-9]{9}Z-[0-9a-f]{24})$') {
        throw 'P2 acceptance is invalid'
    }
    $run = $Matches['run']; $validated = Assert-P2PassRun `
        -RunRoot (Join-Path $OutputRoot ([string]$Acceptance.run_path).Replace('/', '\')) `
        -RunId $run -SourceIdentitySha256 ([string]$Acceptance.source_identity_sha256)
    if ([string]$Acceptance.run_evidence_sha256 -cne $validated.evidence_sha256 -or
        [string]$Acceptance.seal_sha256 -cne $validated.seal_sha256 -or
        [string]$Acceptance.decision_sha256 -cne $validated.decision_sha256 -or
        [string]$Acceptance.seal_path -cne "runs/$run/SHA256SUMS" -or
        [string]$Acceptance.decision_path -cne "runs/$run/artifacts/decision.json") {
        throw 'P2 acceptance artifact binding is invalid'
    }
}

function Publish-P2Acceptance {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$OutputRoot, [Parameter(Mandatory)][string]$RunId,
        [Parameter(Mandatory)][string]$SourceIdentitySha256,
        [Parameter(Mandatory)][string]$RepositoryRoot,
        [Parameter(Mandatory)][string]$SourceInputFingerprint)
    $root = [IO.Path]::GetFullPath($OutputRoot); $runRoot = Join-Path (Join-Path $root 'runs') $RunId
    $validated = Assert-P2PassRun -RunRoot $runRoot -RunId $RunId -SourceIdentitySha256 $SourceIdentitySha256
    $mutexName = 'Local\python-slm-p2-' + (Get-P2TextSha256 -Text $root.ToLowerInvariant())
    $mutex = [Threading.Mutex]::new($false, $mutexName); $locked = $false
    $newAcceptancePath=$null;$acceptanceTemporary=$null;$pointerTemporary=$null;$pointerBackup=$null
    try {
        $locked = $mutex.WaitOne([TimeSpan]::FromSeconds(30))
        if (-not $locked) { throw 'timed out waiting for P2 publication mutex' }
        $publicationIdentity=Get-P2RepositoryIdentity -RepositoryRoot $RepositoryRoot
        if($publicationIdentity.fingerprint-cne$SourceInputFingerprint){throw 'repository inputs changed before P2 publication'}
        $acceptanceRoot = Join-Path $root 'acceptances'; [void][IO.Directory]::CreateDirectory($acceptanceRoot)
        if (((Get-Item -Force -LiteralPath $acceptanceRoot).Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw 'P2 acceptance directory is a reparse point'
        }
        $files = @(Get-ChildItem -LiteralPath $acceptanceRoot -File | Sort-Object Name)
        $previousHash = $null; $last = $null
        for ($index = 0; $index -lt $files.Count; $index++) {
            $sequence = $index + 1
            if ($files[$index].Name -cne ($sequence.ToString('00000000') + '.json')) {
                throw 'P2 acceptance chain has a gap or noncanonical file'
            }
            $prior = [IO.File]::ReadAllText($files[$index].FullName, $script:P2Utf8NoBom) | ConvertFrom-Json
            Assert-P2Acceptance -Acceptance $prior -Sequence $sequence -PreviousHash $previousHash -OutputRoot $root
            $previousHash = Get-P2Sha256 -Path $files[$index].FullName; $last = $prior
        }
        $pointerPath = Join-Path $root 'evidence.json'; $oldPointerBytes = $null
        if (Test-Path -LiteralPath $pointerPath -PathType Leaf) {
            $oldPointerBytes = [IO.File]::ReadAllBytes($pointerPath)
            $old = [IO.File]::ReadAllText($pointerPath, $script:P2Utf8NoBom) | ConvertFrom-Json
            Assert-P2ClosedObject -Value $old -Fields @('schema', 'phase_id', 'acceptance_path', 'acceptance_sha256', 'updated_at') -Name 'P2 pointer'
            if ([string]$old.schema -cne $script:P2Schema.Pointer -or [string]$old.phase_id -cne 'P2' -or
                [string]$old.acceptance_path -cnotmatch '^acceptances/[0-9]{8}\.json$') { throw 'P2 pointer is invalid' }
            $oldSelected = Resolve-P2ContainedFile -Root $root -Relative ([string]$old.acceptance_path) -Name 'P2 pointer acceptance'
            if ((Get-P2Sha256 -Path $oldSelected) -cne [string]$old.acceptance_sha256) { throw 'P2 pointer hash is invalid' }
        }
        $recover = $null -ne $last -and [string]$last.run_path -ceq "runs/$RunId" -and
            [string]$last.source_identity_sha256 -ceq $SourceIdentitySha256
        if ($recover) {
            $sequence = [int]$last.sequence; $relative = 'acceptances/' + $sequence.ToString('00000000') + '.json'
            $acceptanceHash = $previousHash
        }
        else {
            $sequence = $files.Count + 1; $relative = 'acceptances/' + $sequence.ToString('00000000') + '.json'
            $acceptance = [ordered]@{
                schema = $script:P2Schema.Acceptance; phase_id = 'P2'; sequence = $sequence
                status = 'PASS'
                acceptance_kind = 'automatic_machine_qualification'; required_approvals = @()
                run_path = "runs/$RunId"; run_evidence_sha256 = $validated.evidence_sha256
                seal_path = "runs/$RunId/SHA256SUMS"; seal_sha256 = $validated.seal_sha256
                decision_path = "runs/$RunId/artifacts/decision.json"; decision_sha256 = $validated.decision_sha256
                source_identity_sha256 = $SourceIdentitySha256; previous_acceptance_sha256 = $previousHash
                created_at = [DateTime]::UtcNow.ToString('o')
            }
            $acceptancePath = Join-Path $root $relative.Replace('/', '\')
            $acceptanceTemporary=Join-Path $acceptanceRoot ('.acceptance.'+$RunId+'.tmp')
            Write-P2JsonFile -Path $acceptanceTemporary -Value $acceptance -CreateNew
            Assert-P2Acceptance -Acceptance $acceptance -Sequence $sequence -PreviousHash $previousHash -OutputRoot $root
            $acceptanceHash = Get-P2Sha256 -Path $acceptanceTemporary
            [IO.File]::Move($acceptanceTemporary,$acceptancePath);$acceptanceTemporary=$null;$newAcceptancePath=$acceptancePath
        }
        $pointer = [ordered]@{
            schema = $script:P2Schema.Pointer; phase_id = 'P2'; acceptance_path = $relative
            acceptance_sha256 = $acceptanceHash; updated_at = [DateTime]::UtcNow.ToString('o')
        }
        $temporary = Join-Path $root ('.evidence.' + $RunId + '.tmp');$pointerTemporary=$temporary
        try {
            Write-P2JsonFile -Path $temporary -Value $pointer -CreateNew
            if (Test-Path -LiteralPath $pointerPath) {
                $pointerBackup=Join-Path $root ('.pointer-backup.'+$RunId+'.tmp')
                [IO.File]::Replace($temporary, $pointerPath, $pointerBackup, $true)
            }
            else { [IO.File]::Move($temporary, $pointerPath) }
            $pointerTemporary=$null
            $selected = [IO.File]::ReadAllText($pointerPath, $script:P2Utf8NoBom) | ConvertFrom-Json
            Assert-P2ClosedObject -Value $selected -Fields @('schema','phase_id','acceptance_path','acceptance_sha256','updated_at') -Name 'published P2 pointer'
            if ([string]$selected.schema -cne $script:P2Schema.Pointer -or [string]$selected.phase_id -cne 'P2' -or
                [string]$selected.acceptance_path -cne $relative -or [string]$selected.acceptance_sha256 -cne $acceptanceHash) {
                throw 'published P2 pointer reread failed'
            }
            $selectedAcceptancePath=Resolve-P2ContainedFile -Root $root -Relative ([string]$selected.acceptance_path) -Name 'published P2 acceptance'
            if((Get-P2Sha256 -Path $selectedAcceptancePath)-cne[string]$selected.acceptance_sha256){throw 'published P2 pointer acceptance hash failed reread'}
            $selectedAcceptance = [IO.File]::ReadAllText($selectedAcceptancePath, $script:P2Utf8NoBom) | ConvertFrom-Json
            $selectedPrevious = if ($sequence -eq 1) { $null } else {
                Get-P2Sha256 -Path (Join-Path $acceptanceRoot (($sequence - 1).ToString('00000000') + '.json'))
            }
            Assert-P2Acceptance -Acceptance $selectedAcceptance -Sequence $sequence `
                -PreviousHash $selectedPrevious -OutputRoot $root
            if($null-ne$pointerBackup-and(Test-Path -LiteralPath $pointerBackup)){Remove-Item -LiteralPath $pointerBackup -Force;$pointerBackup=$null}
        }
        catch {
            if ($null-ne$pointerBackup-and(Test-Path -LiteralPath $pointerBackup)) {
                $failedPointer=Join-Path $root ('.failed-pointer.'+$RunId+'.tmp')
                [IO.File]::Replace($pointerBackup,$pointerPath,$failedPointer,$true);$pointerBackup=$null
                if(Test-Path -LiteralPath $failedPointer){Remove-Item -LiteralPath $failedPointer -Force}
            }
            elseif($null-eq$oldPointerBytes-and(Test-Path -LiteralPath $pointerPath)){Remove-Item -LiteralPath $pointerPath -Force}
            if($null-ne$oldPointerBytes-and([Convert]::ToBase64String([IO.File]::ReadAllBytes($pointerPath))-cne[Convert]::ToBase64String($oldPointerBytes))){throw 'P2 pointer rollback verification failed'}
            if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
            if($null-ne$newAcceptancePath-and(Test-Path -LiteralPath $newAcceptancePath)){Remove-Item -LiteralPath $newAcceptancePath -Force}
            throw
        }
        return [pscustomobject][ordered]@{
            sequence = $sequence; acceptance_path = $relative; acceptance_sha256 = $acceptanceHash
            pointer_path = 'evidence.json'
        }
    }
    finally {
        foreach($transient in @($acceptanceTemporary,$pointerTemporary,$pointerBackup)){if($null-ne$transient-and(Test-Path -LiteralPath $transient)){Remove-Item -LiteralPath $transient -Force -ErrorAction SilentlyContinue}}
        if ($locked) { try { [void]$mutex.ReleaseMutex() } catch { } }
        $mutex.Dispose()
    }
}

Export-ModuleMember -Function @(
    'Test-P2PathWithin', 'Resolve-P2OutputRoot', 'Write-P2Utf8LfFile', 'Write-P2JsonFile',
    'Get-P2Sha256', 'Get-P2TextSha256', 'New-P2RunId', 'Protect-P2Text',
    'Get-P2NearestRankPercentile', 'Get-P2GeometricMean', 'Select-P2Candidate',
    'Invoke-P2Process', 'Invoke-P2RecordedCommand', 'Test-P2TimingResult',
    'Get-P2CandidateComparison', 'Assert-P2CandidateResult', 'New-P2Policy',
    'Test-P2DependencyPolicy', 'Test-P2CpuIsolationEvidence', 'Get-P2LoadedModuleProvenance',
    'Assert-P2AbbaOrder', 'New-P2CandidateAggregate', 'New-P2InvocationReference',
    'New-P2FileReference', 'Assert-P2FileReference', 'New-P2Seal', 'Test-P2Seal',
    'Get-P2SelectedP1BDependency', 'Assert-P2PassRun', 'Publish-P2Acceptance',
    'Invoke-P2Qualification'
)

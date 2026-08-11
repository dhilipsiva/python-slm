[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Mode,
    [Parameter(Mandatory)][string]$OutputRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$modulePath = Join-Path $PSScriptRoot 'lib\VerifyEnv.psm1'
Import-Module -Name $modulePath -Force
$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))

try {
    $parameters = @{
        Mode = $Mode
        OutputRoot = $OutputRoot
        RepositoryRoot = $repositoryRoot
    }
    $result = Invoke-P1AVerification @parameters
    if ($result.status -ceq 'PASS') {
        [Console]::Out.WriteLine(($result | ConvertTo-Json -Depth 12 -Compress))
        exit 0
    }
    $categories = [Collections.Generic.List[int]]::new()
    foreach ($failure in @($result.errors)) {
        $category = if ($null -ne $failure.category) { [int]$failure.category } else { 5 }
        $categories.Add($category)
        $diagnostic = [ordered]@{
            schema = 'python-slm-script-diagnostic-v1'
            phase_id = [string]$result.phase_id
            run_id = [string]$result.run_id
            code = [string]$failure.code
            category = $category
            message = [string]$failure.message
            remediation = [string]$failure.remediation
        }
        [Console]::Error.WriteLine(($diagnostic | ConvertTo-Json -Compress))
    }
    $exitCategory = if ($categories.Count -eq 0) { 5 } else {
        [int](($categories | Measure-Object -Maximum).Maximum)
    }
    $terminal = [ordered]@{
        schema = 'python-slm-script-error-v1'
        phase_id = [string]$result.phase_id
        run_id = [string]$result.run_id
        code = 'QUALIFICATION_FAILED'
        category = $exitCategory
        message = 'Environment qualification failed; the immutable run was not selected.'
        remediation = 'Inspect the selected failed run and command transcripts, then retry.'
        evidence_path = [string]$result.evidence_path
    }
    [Console]::Error.WriteLine(($terminal | ConvertTo-Json -Compress))
    exit $exitCategory
}
catch {
    $category = 1
    $code = 'INTERNAL_FAILURE'
    $remediation = 'Inspect the selected failed run and command transcripts.'
    if ($_.Exception.Data.Contains('P1ACategory')) {
        $category = [int]$_.Exception.Data['P1ACategory']
    }
    if ($_.Exception.Data.Contains('P1ACode')) {
        $code = [string]$_.Exception.Data['P1ACode']
    }
    if ($_.Exception.Data.Contains('P1ARemediation')) {
        $remediation = [string]$_.Exception.Data['P1ARemediation']
    }
    $safeMessage = try {
        Protect-P1AText -Text ([string]$_.Exception.Message) -RepositoryRoot $repositoryRoot `
            -ExtraRoots @($env:USERPROFILE, $env:SystemRoot, $env:TEMP, $env:TMP)
    }
    catch {
        'Environment verification failed before a safe evidence run could be completed.'
    }
    $errorRecord = [ordered]@{
        schema = 'python-slm-script-error-v1'
        phase_id = if ($Mode -ieq 'Cuda') { 'P1B' } else { 'P1A' }
        code = $code
        category = $category
        message = $safeMessage
        remediation = $remediation
    }
    $diagnostic = [ordered]@{
        schema = 'python-slm-script-diagnostic-v1'
        phase_id = $errorRecord.phase_id
        code = $errorRecord.code
        category = $errorRecord.category
        message = $errorRecord.message
        remediation = $errorRecord.remediation
    }
    [Console]::Error.WriteLine(($diagnostic | ConvertTo-Json -Compress))
    [Console]::Error.WriteLine(($errorRecord | ConvertTo-Json -Compress))
    exit $category
}

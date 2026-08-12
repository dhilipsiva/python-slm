[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$OutputRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$modulePath = Join-Path $PSScriptRoot 'lib\QualifyBackend.psm1'
Import-Module -Name $modulePath -Force

try {
    $result = Invoke-P2Qualification -RepositoryRoot $repositoryRoot -OutputRoot $OutputRoot
    if ([string]$result.status -ceq 'PASS') {
        [Console]::Out.WriteLine(($result | ConvertTo-Json -Depth 16 -Compress))
        exit 0
    }
    $categories = [Collections.Generic.List[int]]::new()
    foreach ($failure in @($result.errors)) {
        $category = if ($null -eq $failure.category) { 5 } else { [int]$failure.category }
        $categories.Add($category)
        [Console]::Error.WriteLine(([ordered]@{
                    schema = 'python-slm-script-diagnostic-v1'
                    phase_id = 'P2'
                    run_id = [string]$result.run_id
                    code = [string]$failure.code
                    category = $category
                    message = [string]$failure.message
                    remediation = [string]$failure.remediation
                } | ConvertTo-Json -Compress))
    }
    $exitCategory = if ($categories.Count -eq 0) { 5 } else {
        [int](($categories | Measure-Object -Maximum).Maximum)
    }
    [Console]::Error.WriteLine(([ordered]@{
                schema = 'python-slm-script-error-v1'
                phase_id = 'P2'
                run_id = [string]$result.run_id
                code = 'QUALIFICATION_FAILED'
                category = $exitCategory
                message = 'Backend qualification failed; the immutable run was not selected.'
                remediation = 'Inspect the failed run and command transcripts, then retry from clean inputs.'
                evidence_path = [string]$result.evidence_path
            } | ConvertTo-Json -Compress))
    exit $exitCategory
}
catch {
    $category = if ($_.Exception.Data.Contains('P2Category')) {
        [int]$_.Exception.Data['P2Category']
    }
    else { 1 }
    $code = if ($_.Exception.Data.Contains('P2Code')) {
        [string]$_.Exception.Data['P2Code']
    }
    else { 'INTERNAL_FAILURE' }
    $message = Protect-P2Text -Text ([string]$_.Exception.Message) `
        -RepositoryRoot $repositoryRoot -ExtraRoots @($env:USERPROFILE, $env:SystemRoot, $env:TEMP, $env:TMP)
    $record = [ordered]@{
        schema = 'python-slm-script-error-v1'
        phase_id = 'P2'
        code = $code
        category = $category
        message = $message
        remediation = 'Inspect the P2 verifier inputs and retry.'
    }
    if ($_.Exception.Data.Contains('P2EvidencePath')) {
        $record['evidence_path'] = [string]$_.Exception.Data['P2EvidencePath']
    }
    [Console]::Error.WriteLine(([ordered]@{
                schema = 'python-slm-script-diagnostic-v1'
                phase_id = 'P2'
                code = $code
                category = $category
                message = $message
                remediation = $record.remediation
            } | ConvertTo-Json -Compress))
    [Console]::Error.WriteLine(($record | ConvertTo-Json -Compress))
    exit $category
}

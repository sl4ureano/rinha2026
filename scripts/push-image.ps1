param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$Tag,

    [string]$Registry = "ghcr.io/sl4ureano/rinha2026",
    [string]$LocalName = "rinha2026",
    [switch]$BuildOnly,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$Remote = "${Registry}:${Tag}"
$Local = "${LocalName}:${Tag}"

Push-Location $Root
try {
    if (-not $SkipBuild) {
        Write-Host "==> docker build -t $Local ."
        docker build -t $Local .
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }

    Write-Host "==> docker tag $Local $Remote"
    docker tag $Local $Remote
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    if ($BuildOnly) {
        Write-Host "BuildOnly: skipped push. Local=$Local Remote=$Remote"
        exit 0
    }

    Write-Host "==> docker push $Remote"
    docker push $Remote
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    Write-Host ""
    Write-Host "OK  $Remote"
    Write-Host "    local: $Local"
}
finally {
    Pop-Location
}

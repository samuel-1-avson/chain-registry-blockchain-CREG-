# Provision GCP VM + static IP + firewall for HOSTING-301 (gcloud SDK).
#
# Usage:
#   .\testnet\gcp\provision-vm.ps1
#   .\testnet\gcp\provision-vm.ps1 -ProjectId my-project -Confirm

param(
    [string]$ProjectId = "",
    [string]$Region = "",
    [string]$Zone = "",
    [string]$VmName = "",
    [string]$MachineType = "",
    [int]$BootDiskGb = 0,
    [string]$StaticIpName = "",
    [switch]$Confirm,
    [switch]$SkipIfExists
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Region) { $Region = $cfg.GCP_REGION }
if (-not $Zone) { $Zone = $cfg.GCP_ZONE }
if (-not $VmName) { $VmName = $cfg.GCP_VM_NAME }
if (-not $MachineType) { $MachineType = $cfg.GCP_MACHINE_TYPE }
if (-not $BootDiskGb) { $BootDiskGb = [int]$cfg.GCP_BOOT_DISK_GB }
if (-not $StaticIpName) { $StaticIpName = $cfg.GCP_STATIC_IP_NAME }

function Log($m) { Write-Host "[gcp-provision] $m" }
function Require-Gcloud {
    if (-not (Get-Command gcloud -ErrorAction SilentlyContinue)) {
        throw "gcloud not found. Install: https://cloud.google.com/sdk/docs/install"
    }
}

Require-Gcloud
if (-not $ProjectId) { throw "Set GCP_PROJECT in testnet/gcp/hosting.env or gcloud config set project" }

Log "Project=$ProjectId Region=$Region Zone=$Zone VM=$VmName"

$prevEap = $ErrorActionPreference
$ErrorActionPreference = "SilentlyContinue"
$null = gcloud compute instances describe $VmName --zone=$Zone --project=$ProjectId 2>&1
$existing = ($LASTEXITCODE -eq 0)
$ErrorActionPreference = $prevEap
if ($existing) {
    if ($SkipIfExists) {
        Log "VM already exists (SkipIfExists)"
    } else {
        throw "VM '$VmName' already exists. Use -SkipIfExists or delete it first."
    }
} elseif (-not $Confirm) {
    Write-Host ""
    Write-Host "About to create in project $ProjectId :" -ForegroundColor Yellow
    Write-Host "  Static IP: $StaticIpName ($Region)"
    Write-Host "  VM: $VmName ($MachineType, ${BootDiskGb}GB, $Zone)"
    Write-Host "  Firewall: tcp 22,80,443 on tag creg-testnet"
    Write-Host ""
    Write-Host "Re-run with -Confirm to proceed." -ForegroundColor Yellow
    exit 0
}

gcloud config set project $ProjectId | Out-Null
Log "Enabling Compute API..."
gcloud services enable compute.googleapis.com --project=$ProjectId | Out-Null

$fwHttp = "creg-testnet-allow-http-https"
$fwSsh = "creg-testnet-allow-ssh"
$tag = "creg-testnet"

foreach ($pair in @(
    @{ Name = $fwHttp; Allow = "tcp:80,tcp:443" }
    @{ Name = $fwSsh; Allow = "tcp:22" }
)) {
    $exists = gcloud compute firewall-rules describe $pair.Name --project=$ProjectId 2>$null
    if ($LASTEXITCODE -ne 0) {
        Log "Creating firewall $($pair.Name)..."
        gcloud compute firewall-rules create $pair.Name `
            --project=$ProjectId `
            --direction=INGRESS `
            --priority=1000 `
            --network=default `
            --action=ALLOW `
            --rules=$($pair.Allow) `
            --target-tags=$tag | Out-Null
    } else {
        Log "Firewall $($pair.Name) exists"
    }
}

$ipExists = gcloud compute addresses describe $StaticIpName --region=$Region --project=$ProjectId 2>$null
if ($LASTEXITCODE -ne 0) {
    Log "Reserving static IP $StaticIpName..."
    gcloud compute addresses create $StaticIpName --region=$Region --project=$ProjectId | Out-Null
} else {
    Log "Static IP $StaticIpName exists"
}

$staticIp = (gcloud compute addresses describe $StaticIpName --region=$Region --project=$ProjectId --format="get(address)").Trim()
Log "Static IP: $staticIp"

if (-not $existing) {
    $bootstrap = Join-Path $gcpDir "vm-bootstrap.sh"
    Log "Creating VM (startup script installs Docker)..."
    gcloud compute instances create $VmName `
        --project=$ProjectId `
        --zone=$Zone `
        --machine-type=$MachineType `
        --boot-disk-size="${BootDiskGb}GB" `
        --boot-disk-type=pd-balanced `
        --image-family=ubuntu-2204-lts `
        --image-project=ubuntu-os-cloud `
        --tags=$tag `
        --address=$StaticIpName `
        --metadata-from-file=startup-script=$bootstrap `
        --scopes=default
    if ($LASTEXITCODE -ne 0) {
        throw "VM create failed (static IP $staticIp is reserved; re-run with -Confirm after fixing the error)"
    }
    Log "Waiting for VM to be RUNNING..."
    for ($i = 0; $i -lt 36; $i++) {
        $prevEap = $ErrorActionPreference
        $ErrorActionPreference = "SilentlyContinue"
        $st = gcloud compute instances describe $VmName --zone=$Zone --project=$ProjectId --format="get(status)" 2>$null
        $ErrorActionPreference = $prevEap
        if ($st) { $st = $st.Trim() }
        if ($st -eq "RUNNING") { break }
        Start-Sleep -Seconds 5
    }
    if ($st -ne "RUNNING") {
        throw "VM did not reach RUNNING state (last status: '$st')"
    }
}

$statePath = Join-Path $gcpDir "hosting-state.json"
$state = @{
    project    = $ProjectId
    region     = $Region
    zone       = $Zone
    vmName     = $VmName
    staticIp   = $staticIp
    baseDomain = $cfg.BASE_DOMAIN
    updatedAt  = (Get-Date).ToUniversalTime().ToString("o")
}
$state | ConvertTo-Json | Set-Content -Path $statePath -Encoding utf8
Log "Wrote $statePath"

Write-Host ""
Write-Host "=== Next: DNS (Cloudflare) ===" -ForegroundColor Cyan
Write-Host "  .\testnet\gcp\set-cloudflare-dns.ps1 -StaticIp $staticIp"
Write-Host ""
Write-Host "=== Next: local prep ===" -ForegroundColor Cyan
Write-Host "  .\testnet\prepare-public-hosting.ps1 -BaseDomain $($cfg.BASE_DOMAIN) -AcmeEmail $($cfg.ACME_EMAIL) -StaticIp $staticIp"
Write-Host ""
Write-Host "=== Next: deploy ===" -ForegroundColor Cyan
Write-Host "  .\testnet\gcp\deploy-stack.ps1"

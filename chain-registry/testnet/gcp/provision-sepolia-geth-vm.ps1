# Provision internal-only Sepolia Geth VM + firewall (Option A).
#
# Usage:
#   .\testnet\gcp\provision-sepolia-geth-vm.ps1
#   .\testnet\gcp\provision-sepolia-geth-vm.ps1 -Confirm

param(
    [string]$ProjectId = "",
    [string]$Region = "",
    [string]$Zone = "",
    [string]$VmName = "",
    [string]$MachineType = "",
    [int]$BootDiskGb = 0,
    [string]$InternalIpName = "",
    [string]$TestnetTag = "",
    [string]$GethTag = "",
    [switch]$Confirm,
    [switch]$SkipIfExists
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Region) { $Region = $cfg.GCP_REGION }
if (-not $Zone) { $Zone = $cfg.GCP_ZONE }
if (-not $VmName) { $VmName = if ($cfg.GCP_SEPOLIA_GETH_VM_NAME) { $cfg.GCP_SEPOLIA_GETH_VM_NAME } else { "creg-sepolia-geth-vm" } }
if (-not $MachineType) { $MachineType = if ($cfg.GCP_SEPOLIA_GETH_MACHINE_TYPE) { $cfg.GCP_SEPOLIA_GETH_MACHINE_TYPE } else { "e2-standard-2" } }
if (-not $BootDiskGb) { $BootDiskGb = if ($cfg.GCP_SEPOLIA_GETH_DISK_GB) { [int]$cfg.GCP_SEPOLIA_GETH_DISK_GB } else { 100 } }
if (-not $InternalIpName) { $InternalIpName = if ($cfg.GCP_SEPOLIA_GETH_INTERNAL_IP_NAME) { $cfg.GCP_SEPOLIA_GETH_INTERNAL_IP_NAME } else { "creg-sepolia-geth-internal-ip" } }
if (-not $TestnetTag) { $TestnetTag = if ($cfg.GCP_TESTNET_TAG) { $cfg.GCP_TESTNET_TAG } else { "creg-testnet" } }
if (-not $GethTag) { $GethTag = if ($cfg.GCP_SEPOLIA_GETH_TAG) { $cfg.GCP_SEPOLIA_GETH_TAG } else { "creg-sepolia-geth" } }

$fwRpc = "creg-sepolia-geth-allow-rpc-from-testnet"
$fwIap = "creg-sepolia-geth-allow-iap-ssh"

function Log($m) { Write-Host "[gcp-sepolia-geth] $m" }

if (-not (Get-Command gcloud -ErrorAction SilentlyContinue)) {
    throw "gcloud not found. Install: https://cloud.google.com/sdk/docs/install"
}
if (-not $ProjectId) { throw "Set GCP_PROJECT in testnet/gcp/hosting.env" }

Log "Project=$ProjectId VM=$VmName (no public IP) internalIp=$InternalIpName"

$prevEap = $ErrorActionPreference
$ErrorActionPreference = "SilentlyContinue"
$null = gcloud compute instances describe $VmName --zone=$Zone --project=$ProjectId 2>&1
$vmExists = ($LASTEXITCODE -eq 0)
$ErrorActionPreference = $prevEap

if ($vmExists -and -not $SkipIfExists -and -not $Confirm) {
    Log "VM '$VmName' already exists. Use -SkipIfExists or -Confirm to refresh state file only."
    exit 0
}

if (-not $vmExists -and -not $Confirm) {
    Write-Host ""
    Write-Host "About to create (internal Sepolia Geth - Option A):" -ForegroundColor Yellow
    Write-Host "  VM: $VmName ($MachineType, ${BootDiskGb}GB, $Zone, tag=$GethTag, --no-address)"
    Write-Host "  Internal IP: $InternalIpName ($Region)"
    Write-Host "  Firewall: tcp:8545 from tag $TestnetTag -> tag $GethTag"
    Write-Host "  Firewall: tcp:22 from IAP 35.235.240.0/20 -> tag $GethTag"
    Write-Host "  Est. cost: ~`$69/mo extra (see docs/GCP-SEPOLIA-GETH-INTERNAL.md)"
    Write-Host ""
    Write-Host "Re-run with -Confirm to proceed." -ForegroundColor Yellow
    exit 0
}

gcloud config set project $ProjectId | Out-Null
gcloud services enable compute.googleapis.com --project=$ProjectId | Out-Null

function Ensure-CloudNat {
    param([string]$RouterName = "creg-nat-router", [string]$NatName = "creg-nat")
    $prev = $ErrorActionPreference
    $ErrorActionPreference = "SilentlyContinue"
    gcloud compute routers describe $RouterName --region=$Region --project=$ProjectId 2>$null | Out-Null
    $routerExists = ($LASTEXITCODE -eq 0)
    $ErrorActionPreference = $prev
    if (-not $routerExists) {
        Log "Creating Cloud Router $RouterName (egress for VMs without public IP)..."
        gcloud compute routers create $RouterName `
            --project=$ProjectId `
            --network=default `
            --region=$Region | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "Failed to create Cloud Router $RouterName" }
    } else {
        Log "Cloud Router $RouterName exists"
    }
    $prev = $ErrorActionPreference
    $ErrorActionPreference = "SilentlyContinue"
    gcloud compute routers nats describe $NatName --router=$RouterName --region=$Region --project=$ProjectId 2>$null | Out-Null
    $natExists = ($LASTEXITCODE -eq 0)
    $ErrorActionPreference = $prev
    if (-not $natExists) {
        Log "Creating Cloud NAT $NatName..."
        gcloud compute routers nats create $NatName `
            --project=$ProjectId `
            --router=$RouterName `
            --region=$Region `
            --nat-all-subnet-ip-ranges `
            --auto-allocate-nat-external-ips | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "Failed to create Cloud NAT $NatName" }
    } else {
        Log "Cloud NAT $NatName exists"
    }
}

Ensure-CloudNat

function Ensure-Firewall {
    param([string]$Name, [string]$Rules, [string]$SourceRanges, [string]$SourceTags, [string]$TargetTags)
    $prev = $ErrorActionPreference
    $ErrorActionPreference = "SilentlyContinue"
    gcloud compute firewall-rules describe $Name --project=$ProjectId 2>$null | Out-Null
    $exists = ($LASTEXITCODE -eq 0)
    $ErrorActionPreference = $prev
    if ($exists) {
        Log "Firewall $Name exists"
        return
    }
    Log "Creating firewall $Name..."
    $args = @(
        "compute", "firewall-rules", "create", $Name,
        "--project=$ProjectId",
        "--direction=INGRESS",
        "--priority=1000",
        "--network=default",
        "--action=ALLOW",
        "--rules=$Rules",
        "--target-tags=$TargetTags"
    )
    if ($SourceRanges) { $args += "--source-ranges=$SourceRanges" }
    if ($SourceTags) { $args += "--source-tags=$SourceTags" }
    & gcloud @args | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Failed to create firewall $Name" }
}

Ensure-Firewall -Name $fwRpc -Rules "tcp:8545" -SourceTags $TestnetTag -TargetTags $GethTag
Ensure-Firewall -Name $fwIap -Rules "tcp:22" -SourceRanges "35.235.240.0/20" -TargetTags $GethTag

$prev = $ErrorActionPreference
$ErrorActionPreference = "SilentlyContinue"
gcloud compute addresses describe $InternalIpName --region=$Region --project=$ProjectId 2>$null | Out-Null
$ipReserved = ($LASTEXITCODE -eq 0)
$ErrorActionPreference = $prev

if (-not $ipReserved) {
    Log "Reserving internal IP $InternalIpName..."
    gcloud compute addresses create $InternalIpName `
        --project=$ProjectId `
        --region=$Region `
        --subnet=default | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Failed to reserve internal IP" }
}

$internalIp = (gcloud compute addresses describe $InternalIpName --region=$Region --project=$ProjectId --format="get(address)").Trim()
Log "Internal IP: $internalIp"

if (-not $vmExists) {
    $bootstrap = Join-Path $gcpDir "vm-bootstrap.sh"
    Log "Creating VM (no external IP)..."
    gcloud compute instances create $VmName `
        --project=$ProjectId `
        --zone=$Zone `
        --machine-type=$MachineType `
        --boot-disk-size="${BootDiskGb}GB" `
        --boot-disk-type=pd-balanced `
        --image-family=ubuntu-2204-lts `
        --image-project=ubuntu-os-cloud `
        --tags=$GethTag `
        --no-address `
        --private-network-ip=$internalIp `
        --metadata-from-file=startup-script=$bootstrap `
        --scopes=default
    if ($LASTEXITCODE -ne 0) { throw "VM create failed" }
    Log "Wait 60s for Docker bootstrap..."
    Start-Sleep -Seconds 60
}

$rpcUrl = "http://${internalIp}:8545"
$statePath = Join-Path $gcpDir "sepolia-geth-state.json"
@{
    project    = $ProjectId
    region     = $Region
    zone       = $Zone
    vmName     = $VmName
    internalIp = $internalIp
    rpcUrl     = $rpcUrl
    gethTag    = $GethTag
    testnetTag = $TestnetTag
    updatedAt  = (Get-Date).ToUniversalTime().ToString("o")
} | ConvertTo-Json | Set-Content -Path $statePath -Encoding utf8
Log "Wrote $statePath"

Write-Host ""
Write-Host "=== Next ===" -ForegroundColor Cyan
Write-Host "  .\testnet\gcp\deploy-sepolia-geth.ps1"
Write-Host "  .\testnet\gcp\get-sepolia-geth-rpc-url.ps1"
Write-Host "  Add SEPOLIA_RPC_URL / CREG_ETH_RPC to sepolia-3node.env -> deploy-stack.ps1"
Write-Host ""
Write-Host "RPC URL (internal only): $rpcUrl" -ForegroundColor Green

# Provision internal-only validator fleet VM + firewall (Option A).
#
# Usage:
#   .\testnet\gcp\provision-validator-vm.ps1
#   .\testnet\gcp\provision-validator-vm.ps1 -Confirm

param(
    [string]$ProjectId = "",
    [string]$Region = "",
    [string]$Zone = "",
    [string]$VmName = "",
    [string]$MachineType = "",
    [int]$BootDiskGb = 0,
    [string]$InternalIpName = "",
    [string]$TestnetTag = "",
    [string]$ValidatorTag = "",
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
if (-not $VmName) { $VmName = if ($cfg.GCP_VALIDATOR_VM_NAME) { $cfg.GCP_VALIDATOR_VM_NAME } else { "creg-validator-vm" } }
if (-not $MachineType) { $MachineType = if ($cfg.GCP_VALIDATOR_MACHINE_TYPE) { $cfg.GCP_VALIDATOR_MACHINE_TYPE } else { "e2-standard-8" } }
if (-not $BootDiskGb) { $BootDiskGb = if ($cfg.GCP_VALIDATOR_DISK_GB) { [int]$cfg.GCP_VALIDATOR_DISK_GB } else { 50 } }
if (-not $InternalIpName) { $InternalIpName = if ($cfg.GCP_VALIDATOR_INTERNAL_IP_NAME) { $cfg.GCP_VALIDATOR_INTERNAL_IP_NAME } else { "creg-validator-internal-ip" } }
if (-not $TestnetTag) { $TestnetTag = if ($cfg.GCP_TESTNET_TAG) { $cfg.GCP_TESTNET_TAG } else { "creg-testnet" } }
if (-not $ValidatorTag) { $ValidatorTag = if ($cfg.GCP_VALIDATOR_TAG) { $cfg.GCP_VALIDATOR_TAG } else { "creg-validators" } }
if (-not $GethTag) { $GethTag = if ($cfg.GCP_SEPOLIA_GETH_TAG) { $cfg.GCP_SEPOLIA_GETH_TAG } else { "creg-sepolia-geth" } }

$fwApi = "creg-validators-allow-api-from-testnet"
$fwEdge = "creg-testnet-allow-edge-from-validators"
$fwGeth = "creg-sepolia-geth-allow-rpc-from-validators"
$fwIap = "creg-validators-allow-iap-ssh"

function Log($m) { Write-Host "[gcp-validator] $m" }

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
    Write-Host "About to create (validator fleet - Option A):" -ForegroundColor Yellow
    Write-Host "  VM: $VmName ($MachineType, ${BootDiskGb}GB, $Zone, tag=$ValidatorTag, --no-address)"
    Write-Host "  Internal IP: $InternalIpName ($Region)"
    Write-Host "  Firewall: tcp:28180-28199 from tag $TestnetTag -> tag $ValidatorTag"
    Write-Host "  Firewall: tcp:15001,tcp:18888 from tag $ValidatorTag -> tag $TestnetTag"
    Write-Host "  Firewall: tcp:8545 from tag $ValidatorTag -> tag $GethTag"
    Write-Host "  Firewall: tcp:22 from IAP 35.235.240.0/20 -> tag $ValidatorTag"
    Write-Host "  Est. cost: ~`$216/mo VM + shared NAT (see docs/GCP-VALIDATOR-FLEET.md)"
    Write-Host ""
    Write-Host "Re-run with -Confirm to proceed." -ForegroundColor Yellow
    exit 0
}

gcloud config set project $ProjectId | Out-Null
gcloud services enable compute.googleapis.com --project=$ProjectId | Out-Null


function Get-MachineTypeVcpuCount {
    param(
        [string]$TypeName,
        [string]$Zone,
        [string]$Project
    )
    $short = ($TypeName -split '/')[-1]
    if ($short -match '-(\d+)$') { return [int]$Matches[1] }
    $cpus = (gcloud compute machine-types describe $short --zone=$Zone --project=$Project --format="value(guestCpus)" 2>$null)
    if ($LASTEXITCODE -eq 0 -and $cpus) { return [int]$cpus }
    throw "Could not resolve vCPU count for machine type: $TypeName"
}

function Test-SsdQuotaPreflight {
    param(
        [string]$ProjectId,
        [string]$Region,
        [int]$NewValidatorDiskGb,
        [double]$DefaultLimit = 250
    )
    $ssdLimit = $DefaultLimit
    $prev = $ErrorActionPreference
    $ErrorActionPreference = "SilentlyContinue"
    $quotaJson = gcloud compute project-info describe --project=$ProjectId --format=json 2>$null
    $ErrorActionPreference = $prev
    if ($quotaJson) {
        $ssdQ = (($quotaJson | ConvertFrom-Json).quotas | Where-Object { $_.metric -eq "SSD_TOTAL_GB" } | Select-Object -First 1)
        if ($ssdQ) { $ssdLimit = [double]$ssdQ.limit }
    }

    $usedGb = 0
    $rows = @(gcloud compute disks list --project=$ProjectId --filter="zone:($Region*)" --format="csv[no-heading](sizeGb)" 2>$null)
    foreach ($row in $rows) {
        if ($row -match '^\d+$') { $usedGb += [int]$row.Trim() }
    }

    if (-not $vmExists) {
        $projected = $usedGb + $NewValidatorDiskGb
        if ($projected -gt $ssdLimit) {
            $edgeGb = if ($cfg.GCP_BOOT_DISK_GB) { [int]$cfg.GCP_BOOT_DISK_GB } else { 100 }
            $gethGb = if ($cfg.GCP_SEPOLIA_GETH_DISK_GB) { [int]$cfg.GCP_SEPOLIA_GETH_DISK_GB } else { 100 }
            $quotaUrl = "https://console.cloud.google.com/iam-admin/quotas?project=$ProjectId&metric=compute.googleapis.com%2Fssd_total_storage"
            Write-Host ""
            Write-Host "QUOTA WARNING: SSD_TOTAL_GB projected ${projected}GB exceeds limit ${ssdLimit}GB (in-use ~${usedGb}GB + new validator ${NewValidatorDiskGb}GB)." -ForegroundColor Red
            Write-Host "  Typical Option C layout: edge ${edgeGb}GB + geth ${gethGb}GB + validator <= $([math]::Max(0, $ssdLimit - $edgeGb - $gethGb))GB"
            Write-Host "  Set GCP_VALIDATOR_DISK_GB=50 in testnet/gcp/hosting.env (default) or request increase: $quotaUrl"
            throw "SSD quota preflight failed (SSD_TOTAL_GB)"
        }
        Log "SSD quota OK: projected $($usedGb + $NewValidatorDiskGb) / $ssdLimit GB (SSD_TOTAL_GB, in-use ~${usedGb}GB)"
    }
}

function Test-CpuQuotaPreflight {
    param(
        [string]$ProjectId,
        [string]$Zone,
        [string]$MachineType,
        [double]$DefaultLimit = 12
    )
    $cpuLimit = $DefaultLimit
    $prev = $ErrorActionPreference
    $ErrorActionPreference = "SilentlyContinue"
    $quotaJson = gcloud compute project-info describe --project=$ProjectId --format=json 2>$null
    $ErrorActionPreference = $prev
    if ($quotaJson) {
        $cpuQ = (($quotaJson | ConvertFrom-Json).quotas | Where-Object { $_.metric -eq "CPUS_ALL_REGIONS" } | Select-Object -First 1)
        if ($cpuQ) { $cpuLimit = [double]$cpuQ.limit }
    }

    $used = 0
    $rows = @(gcloud compute instances list --project=$ProjectId --format="csv[no-heading](machineType)" 2>$null)
    foreach ($row in $rows) {
        if (-not $row) { continue }
        $used += Get-MachineTypeVcpuCount -TypeName $row.Trim() -Zone $Zone -Project $ProjectId
    }

    $newCpus = Get-MachineTypeVcpuCount -TypeName $MachineType -Zone $Zone -Project $ProjectId
    $projected = $used + $newCpus
    if ($projected -gt $cpuLimit) {
        $quotaUrl = "https://console.cloud.google.com/iam-admin/quotas?project=$ProjectId&metric=compute.googleapis.com%2Fcpus_all_regions"
        Write-Host ""
        Write-Host "QUOTA WARNING: CPUS_ALL_REGIONS projected $projected vCPUs exceeds limit $cpuLimit (running ~$used + new $MachineType=$newCpus)." -ForegroundColor Red
        Write-Host "  Request increase: $quotaUrl (suggest limit 24)"
        Write-Host "  Or downsize edge: gcloud compute instances stop creg-testnet-vm --zone=$Zone --project=$ProjectId"
        Write-Host "                    gcloud compute instances set-machine-type creg-testnet-vm --zone=$Zone --project=$ProjectId --machine-type=e2-standard-2"
        Write-Host "                    gcloud compute instances start creg-testnet-vm --zone=$Zone --project=$ProjectId"
        Write-Host "  Or set GCP_VALIDATOR_MACHINE_TYPE=e2-standard-4 in testnet/gcp/hosting.env (3-node fleet)"
        throw "CPU quota preflight failed (CPUS_ALL_REGIONS)"
    }
    Log "CPU quota OK: projected $projected / $cpuLimit vCPUs (CPUS_ALL_REGIONS, in-use ~$used)"
}

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

Ensure-Firewall -Name $fwApi -Rules "tcp:28180-28199" -SourceTags $TestnetTag -TargetTags $ValidatorTag
Ensure-Firewall -Name $fwEdge -Rules "tcp:15001,tcp:18888" -SourceTags $ValidatorTag -TargetTags $TestnetTag
Ensure-Firewall -Name $fwGeth -Rules "tcp:8545" -SourceTags $ValidatorTag -TargetTags $GethTag
Ensure-Firewall -Name $fwIap -Rules "tcp:22" -SourceRanges "35.235.240.0/20" -TargetTags $ValidatorTag

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
    Test-SsdQuotaPreflight -ProjectId $ProjectId -Region $Region -NewValidatorDiskGb $BootDiskGb
    Test-CpuQuotaPreflight -ProjectId $ProjectId -Zone $Zone -MachineType $MachineType
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
        --tags=$ValidatorTag `
        --no-address `
        --private-network-ip=$internalIp `
        --metadata-from-file=startup-script=$bootstrap `
        --scopes=default
    if ($LASTEXITCODE -ne 0) { throw "VM create failed" }
    Log "Wait 60s for Docker bootstrap..."
    Start-Sleep -Seconds 60
}

$statePath = Join-Path $gcpDir "validator-fleet-state.json"
@{
    project      = $ProjectId
    region       = $Region
    zone         = $Zone
    vmName       = $VmName
    internalIp   = $internalIp
    validatorTag = $ValidatorTag
    testnetTag   = $TestnetTag
    gethTag      = $GethTag
    apiPortRange = "28180-28189"
    updatedAt    = (Get-Date).ToUniversalTime().ToString("o")
} | ConvertTo-Json | Set-Content -Path $statePath -Encoding utf8
Log "Wrote $statePath"

Write-Host ""
Write-Host "=== Next ===" -ForegroundColor Cyan
Write-Host "  Set CREG_VALIDATOR_VM_INTERNAL_IP=$internalIp in sepolia-3node.env"
Write-Host "  Set CREG_EDGE_INTERNAL_IP to creg-testnet-vm VPC IP (gcloud instances describe)"
Write-Host "  .\testnet\gcp\deploy-validator-fleet.ps1"
Write-Host ""
Write-Host "Validator fleet internal IP: $internalIp" -ForegroundColor Green

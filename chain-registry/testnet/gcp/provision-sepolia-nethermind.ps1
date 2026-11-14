# Provision the second CREG-run Sepolia node (Nethermind + Teku) in
# europe-west1. Dry run by default: prints the plan and checks quota and the
# existing network. Creates resources only with -Apply.
#
# Reuses, unchanged: VPC creg-fleet-vpc, subnet creg-eu-west1, the IAP SSH rule
# and the creg-sepolia-rpc firewall rules (RPC allowlist + Ethereum P2P), which
# apply through the creg-sepolia-rpc network tag.
param(
    [string]$ProjectId = 'fiffi-500411',
    [string]$Region = 'europe-west1',
    [string]$Zone = 'europe-west1-c',
    [string]$Network = 'creg-fleet-vpc',
    [string]$Subnet = 'creg-eu-west1',
    [string]$SubnetRange = '10.96.1.0/24',
    [string]$VmName = 'creg-sepolia-nethermind',
    # e2-standard-4 (4 vCPU / 16 GB): n2-standard-4 had no capacity in
    # europe-west1-c on 2026-11-14 (ZONE_RESOURCE_POOL_EXHAUSTED).
    [string]$MachineType = 'e2-standard-4',
    [string]$DataDisk = 'creg-sepolia-nethermind-data',
    [int]$DataDiskGb = 380,
    [int]$BootDiskGb = 30,
    [string]$AddressName = 'creg-sepolia-nethermind-ip',
    [switch]$Apply
)

$ErrorActionPreference = 'Stop'
$rpcTag = 'creg-sepolia-rpc'
$fleetTag = 'creg-fleet'
$bootstrap = Join-Path $PSScriptRoot 'sepolia-nethermind-bootstrap.sh'
$requiredRules = @(
    @{ Name = 'creg-sepolia-rpc-allow-clients'; Port = '8545'; Tag = $rpcTag },
    @{ Name = 'creg-sepolia-execution-p2p'; Port = '30303'; Tag = $rpcTag },
    @{ Name = 'creg-sepolia-beacon-p2p'; Port = '9000'; Tag = $rpcTag },
    @{ Name = 'creg-fleet-iap-ssh'; Port = '22'; Tag = $fleetTag }
)

if (-not (Test-Path -LiteralPath $bootstrap)) { throw 'sepolia-nethermind-bootstrap.sh is missing.' }
if ($DataDiskGb -lt 300) { throw 'Use at least 300 GiB: state alone is about 180 GiB on Sepolia.' }

function Get-GcloudJson([string[]]$Arguments) {
    $ErrorActionPreference = 'Continue'
    $text = & gcloud @Arguments --format=json 2>$null
    $code = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
    if ($code -ne 0) { return $null }
    return ($text | Out-String | ConvertFrom-Json)
}
function Invoke-Gcloud([string[]]$Arguments) {
    & gcloud @Arguments
    if ($LASTEXITCODE -ne 0) { throw "gcloud failed: $($Arguments[0..([Math]::Min(2, $Arguments.Length - 1))] -join ' ')" }
}

# --- Checks (read-only) -------------------------------------------------------
$problems = @()
$regionInfo = Get-GcloudJson @('compute', 'regions', 'describe', $Region, "--project=$ProjectId")
if (-not $regionInfo) { throw "Cannot read quota for $Region." }
$quota = @{}
foreach ($q in $regionInfo.quotas) { $quota[$q.metric] = $q }
$existingVm = Get-GcloudJson @('compute', 'instances', 'describe', $VmName, "--zone=$Zone", "--project=$ProjectId")
$existingDisk = Get-GcloudJson @('compute', 'disks', 'describe', $DataDisk, "--zone=$Zone", "--project=$ProjectId")
$existingIp = Get-GcloudJson @('compute', 'addresses', 'describe', $AddressName, "--region=$Region", "--project=$ProjectId")
# Count only disks this run would still create; a disk from an earlier partial
# run already counts in quota usage.
$ssdNeeded = $(if ($existingDisk) { 0 } else { $DataDiskGb }) + $(if ($existingVm) { 0 } else { $BootDiskGb })
$ssdFree = [double]$quota['SSD_TOTAL_GB'].limit - [double]$quota['SSD_TOTAL_GB'].usage
if ($ssdFree -lt $ssdNeeded) { $problems += "SSD quota in ${Region}: need $ssdNeeded GB, $ssdFree GB free." }
# Each machine family has its own regional CPU quota (N2_CPUS, E2_CPUS, ...).
$familyMetric = ($MachineType.Split('-')[0]).ToUpperInvariant() + '_CPUS'
if ($quota.ContainsKey($familyMetric)) {
    $familyFree = [double]$quota[$familyMetric].limit - [double]$quota[$familyMetric].usage
    if ($familyFree -lt 4) { $problems += "$familyMetric quota in ${Region}: need 4, $familyFree free." }
}
$regionCpuFree = [double]$quota['CPUS'].limit - [double]$quota['CPUS'].usage
if ($regionCpuFree -lt 4) { $problems += "CPU quota in ${Region}: need 4, $regionCpuFree free." }
$project = Get-GcloudJson @('compute', 'project-info', 'describe', "--project=$ProjectId")
$globalCpu = $project.quotas | Where-Object { $_.metric -eq 'CPUS_ALL_REGIONS' }
if ($globalCpu -and ([double]$globalCpu.limit - [double]$globalCpu.usage) -lt 4) { $problems += 'Global CPU quota: fewer than 4 vCPUs free.' }

$subnetInfo = Get-GcloudJson @('compute', 'networks', 'subnets', 'describe', $Subnet, "--region=$Region", "--project=$ProjectId")
if (-not $subnetInfo) { $problems += "Subnet $Subnet does not exist; this script will not create network resources." }
elseif ($subnetInfo.network -notmatch "/networks/$Network$" -or $subnetInfo.ipCidrRange -ne $SubnetRange) {
    $problems += "Subnet $Subnet is not $SubnetRange on $Network."
}
foreach ($rule in $requiredRules) {
    $r = Get-GcloudJson @('compute', 'firewall-rules', 'describe', $rule.Name, "--project=$ProjectId")
    if (-not $r) { $problems += "Firewall rule $($rule.Name) is missing."; continue }
    $ports = ($r.allowed | ForEach-Object { $_.ports }) -join ','
    if ($r.network -notmatch "/networks/$Network$" -or ($r.targetTags -notcontains $rule.Tag) -or $ports -notmatch $rule.Port) {
        $problems += "Firewall rule $($rule.Name) differs from the reviewed rule."
    }
}

Write-Host "Second Sepolia node plan: project=$ProjectId zone=$Zone VM=$VmName ($MachineType)"
Write-Host "Disks: ${BootDiskGb} GB boot + ${DataDiskGb} GB pd-balanced data ($DataDisk). SSD quota free in ${Region}: $ssdFree GB."
Write-Host "Network: $Network / $Subnet ($SubnetRange), reserved IPv4 $AddressName, tags $fleetTag,$rpcTag (reuses the existing RPC allowlist, P2P and IAP SSH rules)."
Write-Host "Clients (deployed separately by deploy-sepolia-nethermind.ps1): Nethermind 2.2.2 + Teku 26.10.3, pinned by digest."
Write-Host 'Estimated list price with e2-standard-4: about $140-170/month (VM, disks, reserved IPv4) before network egress and credits.'
Write-Host 'Egress depends on Ethereum peer traffic; peers are capped (25 execution, 50 consensus) to limit it.'
Write-Host ("Existing: VM={0} disk={1} address={2}" -f [bool]$existingVm, [bool]$existingDisk, [bool]$existingIp)
if ($problems.Count -gt 0) {
    $problems | ForEach-Object { Write-Host "BLOCKER: $_" }
    throw 'Pre-checks failed; nothing was changed.'
}
Write-Host 'Pre-checks passed.'
if (-not $Apply) {
    Write-Host 'Dry run: no cloud resources changed. Re-run with -Apply to create the reserved IP, data disk and VM.'
    return
}

# --- Apply --------------------------------------------------------------------
if ($existingIp) { $reservedIp = $existingIp.address }
else {
    Invoke-Gcloud @('compute', 'addresses', 'create', $AddressName, "--project=$ProjectId", "--region=$Region", '--network-tier=PREMIUM')
    $reservedIp = (& gcloud compute addresses describe $AddressName "--region=$Region" "--project=$ProjectId" --format='value(address)').Trim()
}
if ($existingDisk) {
    if ([int]$existingDisk.sizeGb -lt $DataDiskGb -or $existingDisk.type -notmatch '/diskTypes/pd-balanced$') {
        throw "Existing disk $DataDisk is smaller or not pd-balanced; refusing to replace or resize it."
    }
} else {
    Invoke-Gcloud @('compute', 'disks', 'create', $DataDisk, "--project=$ProjectId", "--zone=$Zone", "--size=${DataDiskGb}GB", '--type=pd-balanced', '--labels=app=creg-sepolia-nethermind,environment=testnet')
}
if ($existingVm) {
    $attached = @($existingVm.disks | ForEach-Object { ($_.source -split '/')[-1] })
    if ($existingVm.machineType -notmatch "/machineTypes/$MachineType$" -or $attached -notcontains $DataDisk) {
        throw "Existing VM $VmName differs from the plan; refusing to modify it."
    }
    Write-Host "VM $VmName already exists with the expected size and disk."
} else {
    Invoke-Gcloud @(
        'compute', 'instances', 'create', $VmName,
        "--project=$ProjectId", "--zone=$Zone", "--machine-type=$MachineType",
        "--network=$Network", "--subnet=$Subnet", "--address=$reservedIp", '--network-tier=PREMIUM',
        "--tags=$fleetTag,$rpcTag", "--boot-disk-size=${BootDiskGb}GB", '--boot-disk-type=pd-balanced',
        '--image-family=ubuntu-2404-lts-amd64', '--image-project=ubuntu-os-cloud',
        "--disk=name=$DataDisk,device-name=creg-sepolia-nethermind-data,mode=rw,boot=no,auto-delete=no",
        "--metadata-from-file=startup-script=$bootstrap", '--no-service-account', '--no-scopes',
        '--labels=app=creg-sepolia-nethermind,environment=testnet'
    )
}
Write-Host "Provisioned $VmName at $reservedIp. RPC will be http://${reservedIp}:8545 (allowlisted clients) or the VM's 10.96.1.x address inside the fleet VPC."
Write-Host 'Next: fetch-beacon-initial-state.ps1, then deploy-sepolia-nethermind.ps1.'

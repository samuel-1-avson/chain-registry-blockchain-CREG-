param(
    [string]$ProjectId = 'fiffi-500411',
    [string]$Region = 'us-west1',
    [string]$Zone = 'us-west1-b',
    [string]$Network = 'creg-fleet-vpc',
    [string]$Subnet = 'creg-us-west1',
    [string]$SubnetRange = '10.96.3.0/24',
    [string]$VmName = 'creg-sepolia-rpc',
    [string]$MachineType = 'e2-standard-4',
    [string]$DataDisk = 'creg-sepolia-rpc-data',
    [int]$DataDiskGb = 400,
    [string]$AddressName = 'creg-sepolia-rpc-ip',
    [string]$FirewallName = 'creg-sepolia-rpc-allow-clients',
    [switch]$Apply
)

$ErrorActionPreference = 'Stop'
$rpcTag = 'creg-sepolia-rpc'
$fleetTag = 'creg-fleet'
$bootstrap = Join-Path $PSScriptRoot 'sepolia-rpc-bootstrap.sh'
$fixedClientIps = @(
    '34.156.122.19', # validator-3
    '34.124.165.97', # validator-4
    '136.66.249.201', # observer-3
    '34.35.191.131' # observer-4
)
$privateFleetRange = '10.96.0.0/16'
$publicPeerRules = @(
    @{ Name = 'creg-sepolia-execution-p2p'; Port = '30303' },
    @{ Name = 'creg-sepolia-beacon-p2p'; Port = '9000' }
)

if (-not (Test-Path -LiteralPath $bootstrap)) {
    throw 'The pinned Sepolia RPC bootstrap script is missing.'
}
if ($DataDiskGb -lt 400) {
    throw 'Use at least 400 GiB for the Sepolia chain database.'
}

try {
    $laptopIp = (Invoke-RestMethod -Uri 'https://api.ipify.org' -TimeoutSec 10).ToString().Trim()
} catch {
    throw 'Could not determine the laptop public IPv4 address for the RPC firewall allowlist.'
}
if ($laptopIp -notmatch '^\d{1,3}(\.\d{1,3}){3}$') {
    throw 'The laptop egress address was not an IPv4 address.'
}
$sourceRanges = @($fixedClientIps + $laptopIp | Select-Object -Unique | ForEach-Object { "$_/32" }) + $privateFleetRange

Write-Host "R7 Sepolia RPC plan: project=$ProjectId zone=$Zone VM=$VmName machine=$MachineType dataDisk=${DataDiskGb}GiB"
Write-Host "VPC=$Network subnet=$Subnet ($SubnetRange); RPC clients limited to $($sourceRanges.Count - 1) fixed /32 sources plus the private fleet range and IAP SSH."
Write-Host 'Execution and beacon P2P ports are open for Ethereum peer discovery and block sync.'
Write-Host 'Monthly list-price estimate: about $150 before egress, taxes and credits (VM, boot disk, 400 GiB balanced disk, reserved IPv4).'
if (-not $Apply) {
    Write-Host 'No cloud resources changed. Re-run with -Apply to provision the reviewed GCP resources.'
    return
}

function Invoke-Gcloud([string[]]$Arguments) {
    & gcloud @Arguments
    if ($LASTEXITCODE -ne 0) { throw "gcloud failed: $($Arguments[0..([Math]::Min(2, $Arguments.Length - 1))] -join ' ')" }
}

$subnetJson = & gcloud compute networks subnets describe $Subnet "--region=$Region" "--project=$ProjectId" --format=json 2>$null
if ($LASTEXITCODE -eq 0) {
    $existingSubnet = $subnetJson | ConvertFrom-Json
    if ($existingSubnet.network -notmatch "/networks/$Network$" -or $existingSubnet.ipCidrRange -ne $SubnetRange) {
        throw "Existing subnet $Subnet does not match the reviewed VPC/CIDR; refusing to alter it."
    }
} else {
    Invoke-Gcloud @('compute', 'networks', 'subnets', 'create', $Subnet, "--project=$ProjectId", "--region=$Region", "--network=$Network", "--range=$SubnetRange", '--enable-private-ip-google-access')
}

$ipJson = & gcloud compute addresses describe $AddressName "--region=$Region" "--project=$ProjectId" --format=json 2>$null
if ($LASTEXITCODE -eq 0) {
    $reservedIp = ($ipJson | ConvertFrom-Json).address
} else {
    Invoke-Gcloud @('compute', 'addresses', 'create', $AddressName, "--project=$ProjectId", "--region=$Region", '--network-tier=PREMIUM')
    $reservedIp = (& gcloud compute addresses describe $AddressName "--region=$Region" "--project=$ProjectId" --format='value(address)').Trim()
}

$diskJson = & gcloud compute disks describe $DataDisk "--zone=$Zone" "--project=$ProjectId" --format=json 2>$null
if ($LASTEXITCODE -eq 0) {
    $existingDisk = $diskJson | ConvertFrom-Json
    if ([int]$existingDisk.sizeGb -lt $DataDiskGb -or $existingDisk.type -notmatch '/diskTypes/pd-balanced$') {
        throw "Existing disk $DataDisk is smaller or uses a different type; refusing to replace or resize it."
    }
} else {
    Invoke-Gcloud @('compute', 'disks', 'create', $DataDisk, "--project=$ProjectId", "--zone=$Zone", "--size=${DataDiskGb}GB", '--type=pd-balanced', '--labels=app=creg-sepolia-rpc,environment=testnet')
}

$firewallJson = & gcloud compute firewall-rules describe $FirewallName "--project=$ProjectId" --format=json 2>$null
if ($LASTEXITCODE -eq 0) {
    $existingFirewall = $firewallJson | ConvertFrom-Json
    if ($existingFirewall.network -notmatch "/networks/$Network$" -or
        (($existingFirewall.targetTags -join ',') -notmatch $rpcTag) -or
        (($existingFirewall.allowed | ForEach-Object { $_.ports }) -join ',') -notmatch '8545') {
        throw "Existing firewall rule $FirewallName differs from the reviewed RPC rule; refusing to broaden or replace it."
    }
    Invoke-Gcloud @('compute', 'firewall-rules', 'update', $FirewallName, "--project=$ProjectId", "--source-ranges=$($sourceRanges -join ',')", '--rules=tcp:8545', "--target-tags=$rpcTag", '--description=Sepolia JSON-RPC limited to the current R7 client egress IPs')
} else {
    Invoke-Gcloud @('compute', 'firewall-rules', 'create', $FirewallName, "--project=$ProjectId", "--network=$Network", '--direction=INGRESS', '--priority=1000', '--action=ALLOW', '--rules=tcp:8545', "--source-ranges=$($sourceRanges -join ',')", "--target-tags=$rpcTag", '--description=Sepolia JSON-RPC limited to the current R7 client egress IPs')
}

foreach ($peerRule in $publicPeerRules) {
    $ruleJson = & gcloud compute firewall-rules describe $peerRule.Name "--project=$ProjectId" --format=json 2>$null
    if ($LASTEXITCODE -eq 0) {
        $existingRule = $ruleJson | ConvertFrom-Json
        if ($existingRule.network -notmatch "/networks/$Network$" -or
            (($existingRule.targetTags -join ',') -notmatch $rpcTag)) {
            throw "Existing P2P firewall rule $($peerRule.Name) does not match the reviewed network/tag."
        }
    } else {
        Invoke-Gcloud @('compute', 'firewall-rules', 'create', $peerRule.Name, "--project=$ProjectId", "--network=$Network", '--direction=INGRESS', '--priority=1000', '--action=ALLOW', "--rules=tcp:$($peerRule.Port),udp:$($peerRule.Port)", '--source-ranges=0.0.0.0/0', "--target-tags=$rpcTag", '--description=Public Ethereum Sepolia peer-to-peer transport')
    }
}

$vmJson = & gcloud compute instances describe $VmName "--zone=$Zone" "--project=$ProjectId" --format=json 2>$null
if ($LASTEXITCODE -eq 0) {
    $vm = $vmJson | ConvertFrom-Json
    if ($vm.status -ne 'RUNNING' -or $vm.machineType -notmatch "/machineTypes/$MachineType$") {
        throw "An existing $VmName VM has unexpected status or size; refusing to modify it."
    }
    $attachedNames = @($vm.disks | ForEach-Object { ($_.source -split '/')[-1] })
    if ($attachedNames -notcontains $DataDisk) { throw "Existing VM $VmName does not have the expected persistent RPC data disk attached." }
} else {
    Invoke-Gcloud @(
        'compute', 'instances', 'create', $VmName,
        "--project=$ProjectId", "--zone=$Zone", "--machine-type=$MachineType",
        "--network=$Network", "--subnet=$Subnet", "--address=$reservedIp", '--network-tier=PREMIUM',
        "--tags=$fleetTag,$rpcTag", '--boot-disk-size=50GB', '--boot-disk-type=pd-balanced',
        '--image-family=ubuntu-2204-lts', '--image-project=ubuntu-os-cloud',
        "--disk=name=$DataDisk,device-name=creg-sepolia-rpc-data,mode=rw,boot=no,auto-delete=no",
        "--metadata-from-file=startup-script=$bootstrap", '--no-service-account', '--no-scopes',
        '--labels=app=creg-sepolia-rpc,environment=testnet'
    )
}

Write-Host "Provisioning complete. RPC address: http://${reservedIp}:8545"
Write-Host "Firewall allows RPC from the four active GCP nodes and laptop IPv4 /32. SSH uses the existing IAP-only fleet rule."
Write-Host 'Next: deploy pinned Geth v1.17.6 and Sepolia-compatible Lighthouse, verify sync and provider agreement, then configure nodes one at a time.'

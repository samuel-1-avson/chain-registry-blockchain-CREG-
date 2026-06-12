# GCP-006/007 — Provision observer read pool (MIG + internal TCP load balancer).
#
# Moves public API reads off the validator fleet VM onto dedicated observer VMs.
# Edge Caddy uses CREG_OBSERVER_POOL_LB_IP when set (see Caddyfile.fleet).
#
# Usage:
#   .\testnet\gcp\provision-observer-pool.ps1
#   .\testnet\gcp\provision-observer-pool.ps1 -Confirm
#   .\testnet\gcp\deploy-observer-pool.ps1 -Confirm   # sync repo + start observers on existing MIG

param(
    [string]$ProjectId = "",
    [string]$Region = "",
    [string]$Zone = "",
    [string]$MigName = "",
    [string]$TemplateName = "",
    [string]$MachineType = "",
    [int]$BootDiskGb = 0,
    [int]$MinReplicas = 1,
    [int]$MaxReplicas = 3,
    [string]$ObserverTag = "",
    [string]$TestnetTag = "",
    [string]$ValidatorTag = "",
    [string]$InternalLbName = "",
    [string]$InternalLbIpName = "",
    [int]$ApiPort = 28182,
    [string]$GithubRepo = "",
    [string]$GithubBranch = "",
    [switch]$Confirm,
    [switch]$SkipIfExists
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$testnetDir = Split-Path -Parent $gcpDir
. (Join-Path $gcpDir "_GcpCostLabels.ps1")
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Region) { $Region = $cfg.GCP_REGION }
if (-not $Zone) { $Zone = $cfg.GCP_ZONE }
if (-not $MigName) { $MigName = if ($cfg.GCP_OBSERVER_MIG_NAME) { $cfg.GCP_OBSERVER_MIG_NAME } else { "creg-observer-pool" } }
if (-not $TemplateName) { $TemplateName = if ($cfg.GCP_OBSERVER_TEMPLATE_NAME) { $cfg.GCP_OBSERVER_TEMPLATE_NAME } else { "creg-observer-pool-template" } }
if (-not $MachineType) { $MachineType = if ($cfg.GCP_OBSERVER_MACHINE_TYPE) { $cfg.GCP_OBSERVER_MACHINE_TYPE } else { "e2-medium" } }
if (-not $BootDiskGb) { $BootDiskGb = if ($cfg.GCP_OBSERVER_DISK_GB) { [int]$cfg.GCP_OBSERVER_DISK_GB } else { 30 } }
if (-not $ObserverTag) { $ObserverTag = if ($cfg.GCP_OBSERVER_TAG) { $cfg.GCP_OBSERVER_TAG } else { "creg-observers" } }
if (-not $TestnetTag) { $TestnetTag = if ($cfg.GCP_TESTNET_TAG) { $cfg.GCP_TESTNET_TAG } else { "creg-testnet" } }
if (-not $ValidatorTag) { $ValidatorTag = if ($cfg.GCP_VALIDATOR_TAG) { $cfg.GCP_VALIDATOR_TAG } else { "creg-validators" } }
if (-not $InternalLbName) { $InternalLbName = if ($cfg.GCP_OBSERVER_ILB_NAME) { $cfg.GCP_OBSERVER_ILB_NAME } else { "creg-observer-api-ilb" } }
if (-not $InternalLbIpName) { $InternalLbIpName = if ($cfg.GCP_OBSERVER_ILB_IP_NAME) { $cfg.GCP_OBSERVER_ILB_IP_NAME } else { "creg-observer-api-ilb-ip" } }
if (-not $GithubRepo) { $GithubRepo = $cfg.GITHUB_REPO }
if (-not $GithubBranch) { $GithubBranch = $cfg.GITHUB_BRANCH }

$fwApi = "creg-observers-allow-api-from-testnet"
$fwP2pToValidators = "creg-validators-allow-p2p-from-observers"
$fwP2pFromValidators = "creg-observers-allow-p2p-from-validators"
$fwIap = "creg-observers-allow-iap-ssh"
$fwLbHealth = "creg-observers-allow-lb-health-checks"
$hcName = "creg-observer-api-tcp"
$backendName = "creg-observer-api-backend"
$igName = $MigName

function Log($m) { Write-Host "[observer-pool] $m" }

if (-not (Get-Command gcloud -ErrorAction SilentlyContinue)) {
    throw "gcloud not found"
}
if (-not $ProjectId) { throw "Set GCP_PROJECT in hosting.env" }

$labels = Get-GcpServiceLabel -Service "observer" -Criticality "public"
$labelArg = Format-GcpLabelArg $labels

$startupScript = Join-Path $gcpDir "observer-pool-startup.sh"
if (-not (Test-Path $startupScript)) { throw "Missing $startupScript" }

Log "Project=$ProjectId MIG=$MigName machine=$MachineType replicas=$MinReplicas-$MaxReplicas API port=$ApiPort"

if (-not $Confirm) {
    Write-Host ""
    Write-Host "Will provision observer read pool (GCP-006/007):" -ForegroundColor Yellow
    Write-Host "  Instance template: $TemplateName ($MachineType, ${BootDiskGb}GB, tag=$ObserverTag, no public IP)"
    Write-Host "  Regional MIG: $MigName ($Zone, $MinReplicas-$MaxReplicas)"
    Write-Host "  Internal TCP LB: $InternalLbName -> port $ApiPort (IP: $InternalLbIpName)"
    Write-Host "  Firewall: tcp:$ApiPort from tag $TestnetTag -> tag $ObserverTag"
    Write-Host "  Firewall: P2P between tags $ObserverTag and $ValidatorTag"
    Write-Host "  Labels: $labelArg"
    Write-Host ""
    Write-Host "After deploy, set in sepolia-3node.env:" -ForegroundColor Cyan
    Write-Host "  CREG_OBSERVER_POOL_LB_IP=<internal LB IP>"
    Write-Host "  CREG_3NODE_NODE3_API_PORT=$ApiPort"
    Write-Host "Then redeploy edge stack (deploy-stack.ps1) to refresh Caddy."
    Write-Host ""
    Write-Host "Re-run with -Confirm to proceed." -ForegroundColor Yellow
    exit 0
}

gcloud config set project $ProjectId | Out-Null
gcloud services enable compute.googleapis.com --project=$ProjectId | Out-Null

# Reserved internal IP for ILB
$prevEap = $ErrorActionPreference
$ErrorActionPreference = "SilentlyContinue"
$null = gcloud compute addresses describe $InternalLbIpName --region=$Region --project=$ProjectId 2>&1
$ipExists = ($LASTEXITCODE -eq 0)
$ErrorActionPreference = $prevEap
if (-not $ipExists) {
    Log "Reserving internal IP $InternalLbIpName"
    gcloud compute addresses create $InternalLbIpName `
        --region=$Region `
        --subnet=default `
        --purpose=GCE_ENDPOINT `
        --project=$ProjectId | Out-Null
}

$ilbIp = (gcloud compute addresses describe $InternalLbIpName --region=$Region --project=$ProjectId --format="value(address)").Trim()
Log "Internal LB IP: $ilbIp"

# Firewalls
function Ensure-Firewall {
    param([string]$Name, [string]$Desc, [string]$Rules, [string]$TargetTag, [string]$SourceTags, [string]$SourceRanges)
    $exists = gcloud compute firewall-rules describe $Name --project=$ProjectId 2>$null
    if ($LASTEXITCODE -eq 0) {
        Log "Firewall exists: $Name"
        return
    }
    $args = @(
        "compute", "firewall-rules", "create", $Name,
        "--project=$ProjectId",
        "--description=$Desc",
        "--direction=INGRESS",
        "--action=ALLOW",
        "--rules=$Rules",
        "--target-tags=$TargetTag"
    )
    if ($SourceTags) { $args += "--source-tags=$SourceTags" }
    if ($SourceRanges) { $args += "--source-ranges=$SourceRanges" }
    Log "Creating firewall $Name"
    & gcloud @args | Out-Null
}

Ensure-Firewall -Name $fwApi -Desc "Observer API from edge VM" `
    -Rules "tcp:$ApiPort" -TargetTag $ObserverTag -SourceTags $TestnetTag -SourceRanges $null
Ensure-Firewall -Name $fwP2pToValidators -Desc "Validator P2P from observers" `
    -Rules "tcp:29100-29109" -TargetTag $ValidatorTag -SourceTags $ObserverTag -SourceRanges $null
Ensure-Firewall -Name $fwP2pFromValidators -Desc "Observer P2P from validators" `
    -Rules "tcp:29100-29109" -TargetTag $ObserverTag -SourceTags $ValidatorTag -SourceRanges $null
Ensure-Firewall -Name $fwIap -Desc "IAP SSH to observer pool" `
    -Rules "tcp:22" -TargetTag $ObserverTag -SourceTags $null -SourceRanges "35.235.240.0/20"

Ensure-Firewall -Name $fwLbHealth -Desc "ILB TCP health checks to observer API" `
    -Rules "tcp:$ApiPort" -TargetTag $ObserverTag -SourceTags $null -SourceRanges "130.211.0.0/22,35.191.0.0/16"

# Instance template
$meta = "github-repo=$GithubRepo,github-branch=$GithubBranch"
$templateExists = gcloud compute instance-templates describe $TemplateName --project=$ProjectId 2>$null
if ($LASTEXITCODE -ne 0) {
    Log "Creating instance template $TemplateName"
    gcloud compute instance-templates create $TemplateName `
        --project=$ProjectId `
        --machine-type=$MachineType `
        --boot-disk-size="${BootDiskGb}GB" `
        --boot-disk-type=pd-balanced `
        --image-family=ubuntu-2204-lts `
        --image-project=ubuntu-os-cloud `
        --tags=$ObserverTag `
        --no-address `
        --metadata="enable-oslogin=TRUE,$meta" `
        --metadata-from-file="startup-script=$startupScript" `
        --labels=$labelArg | Out-Null
} elseif (-not $SkipIfExists) {
    Log "Template $TemplateName exists (use new version name to roll forward)"
}

# Regional MIG
$migExists = gcloud compute instance-groups managed describe $MigName --zone=$Zone --project=$ProjectId 2>$null
if ($LASTEXITCODE -ne 0) {
    Log "Creating MIG $MigName"
    gcloud compute instance-groups managed create $MigName `
        --project=$ProjectId `
        --zone=$Zone `
        --template=$TemplateName `
        --size=$MinReplicas | Out-Null
} else {
    Log "MIG $MigName exists"
}

if ($MaxReplicas -gt $MinReplicas) {
    gcloud compute instance-groups managed update $MigName `
        --project=$ProjectId `
        --zone=$Zone `
        --autoscaling-mode=on `
        --min-num-replicas=$MinReplicas `
        --max-num-replicas=$MaxReplicas `
        --target-cpu-utilization=0.7 | Out-Null
}

# Named port for backend service
gcloud compute instance-groups managed set-named-ports $MigName `
    --project=$ProjectId `
    --zone=$Zone `
    --named-ports="api:${ApiPort}" | Out-Null

# Health check + backend + internal forwarding rule
$hcExists = gcloud compute health-checks describe $hcName --project=$ProjectId 2>$null
if ($LASTEXITCODE -ne 0) {
    gcloud compute health-checks create tcp $hcName `
        --project=$ProjectId `
        --port=$ApiPort `
        --check-interval=10s `
        --timeout=5s `
        --unhealthy-threshold=3 `
        --healthy-threshold=2 | Out-Null
}

$beExists = gcloud compute backend-services describe $backendName --region=$Region --project=$ProjectId 2>$null
if ($LASTEXITCODE -ne 0) {
    gcloud compute backend-services create $backendName `
        --project=$ProjectId `
        --region=$Region `
        --protocol=TCP `
        --health-checks=$hcName `
        --load-balancing-scheme=INTERNAL | Out-Null
    gcloud compute backend-services add-backend $backendName `
        --project=$ProjectId `
        --region=$Region `
        --instance-group=$MigName `
        --instance-group-zone=$Zone | Out-Null
} else {
    $backendGroups = @(gcloud compute backend-services describe $backendName --region=$Region --project=$ProjectId --format="value(backends.group)" 2>$null | Where-Object { $_ })
    if ($backendGroups.Count -eq 0) {
        Log "Backend service $backendName has no instance groups; attaching $MigName"
        gcloud compute backend-services add-backend $backendName `
            --project=$ProjectId `
            --region=$Region `
            --instance-group=$MigName `
            --instance-group-zone=$Zone | Out-Null
    }
}

$frExists = gcloud compute forwarding-rules describe $InternalLbName --region=$Region --project=$ProjectId 2>$null
if ($LASTEXITCODE -ne 0) {
    gcloud compute forwarding-rules create $InternalLbName `
        --project=$ProjectId `
        --region=$Region `
        --load-balancing-scheme=INTERNAL `
        --network=default `
        --subnet=default `
        --address=$InternalLbIpName `
        --ports=$ApiPort `
        --backend-service=$backendName | Out-Null
}

$stateFile = Join-Path $gcpDir "observer-pool-state.env"
@"
# Generated by provision-observer-pool.ps1
CREG_OBSERVER_POOL_LB_IP=$ilbIp
CREG_3NODE_NODE3_API_PORT=$ApiPort
GCP_OBSERVER_MIG_NAME=$MigName
GCP_OBSERVER_ILB_NAME=$InternalLbName
"@ | Set-Content -Path $stateFile -Encoding utf8

Log "Done. Add to testnet/sepolia-3node.env:"
Write-Host "  CREG_OBSERVER_POOL_LB_IP=$ilbIp" -ForegroundColor Green
Write-Host "  CREG_3NODE_NODE3_API_PORT=$ApiPort" -ForegroundColor Green
Write-Host "State file: $stateFile" -ForegroundColor Cyan

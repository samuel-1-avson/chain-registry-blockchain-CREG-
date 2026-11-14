$ErrorActionPreference = 'Stop'
# Read-only audit: no deployment, deletion, API activation or billing changes.
$auditDirectory = $PSScriptRoot
$auditToken = (& gcloud auth print-access-token 2>$null).Trim()
if (-not $auditToken) { throw 'No authenticated GCP access token available.' }
$auditHeaders = @{ Authorization = "Bearer $auditToken" }
function Read-AuditApi([string]$Uri) {
    try { Invoke-RestMethod -Uri $Uri -Headers $auditHeaders -Method Get -TimeoutSec 45 }
    catch { [pscustomobject]@{ auditError = $_.Exception.Message } }
}
$auditResults = [ordered]@{ capturedAtUtc = [DateTime]::UtcNow.ToString('o'); projects = @() }
foreach ($auditProject in @('gen-lang-client-0022105784', 'fiffi-500411', 'gen-lang-client-0098858574')) {
    $auditBase = "https://compute.googleapis.com/compute/v1/projects/$auditProject"
    $auditInstances = Read-AuditApi "$auditBase/aggregated/instances?maxResults=500"
    $auditDisks = Read-AuditApi "$auditBase/aggregated/disks?maxResults=500"
    $auditAddresses = Read-AuditApi "$auditBase/aggregated/addresses?maxResults=500"
    $auditSnapshots = Read-AuditApi "$auditBase/global/snapshots?maxResults=500"
    $auditRouters = Read-AuditApi "$auditBase/aggregated/routers?maxResults=500"
    $auditRules = Read-AuditApi "$auditBase/aggregated/forwardingRules?maxResults=500"
    $auditRun = Read-AuditApi "https://run.googleapis.com/v2/projects/$auditProject/locations/us-central1/services?pageSize=100"
    $auditEntry = [ordered]@{
        projectId = $auditProject
        billing = Read-AuditApi "https://cloudbilling.googleapis.com/v1/projects/$auditProject/billingInfo"
        instances = @($auditInstances.items.PSObject.Properties.Value.instances | Where-Object { $_ } | ForEach-Object {
            [pscustomobject]@{ name=$_.name; id=$_.id; zone=($_.zone -split '/')[-1]; machineType=($_.machineType -split '/')[-1]; status=$_.status; created=$_.creationTimestamp; provisioningModel=$_.scheduling.provisioningModel; externalIpv4Count=@($_.networkInterfaces.accessConfigs | Where-Object { $_.natIP }).Count; diskNames=@($_.disks.source | ForEach-Object { ($_ -split '/')[-1] }) }
        })
        disks = @($auditDisks.items.PSObject.Properties.Value.disks | Where-Object { $_ } | ForEach-Object {
            [pscustomobject]@{ name=$_.name; zone=($_.zone -split '/')[-1]; type=($_.type -split '/')[-1]; sizeGb=$_.sizeGb; users=@($_.users | ForEach-Object { ($_ -split '/')[-1] }) }
        })
        addresses = @($auditAddresses.items.PSObject.Properties.Value.addresses | Where-Object { $_ } | ForEach-Object {
            [pscustomobject]@{ name=$_.name; type=$_.addressType; status=$_.status; region=($_.region -split '/')[-1]; users=@($_.users | ForEach-Object { ($_ -split '/')[-1] }) }
        })
        routers = @($auditRouters.items.PSObject.Properties.Value.routers | Where-Object { $_ } | ForEach-Object {
            [pscustomobject]@{ name=$_.name; region=($_.region -split '/')[-1]; nats=@($_.nats | Select-Object name,natIpAllocateOption,sourceSubnetworkIpRangesToNat,subnetworks) }
        })
        forwardingRules = @($auditRules.items.PSObject.Properties.Value.forwardingRules | Where-Object { $_ } | Select-Object name,loadBalancingScheme)
        snapshots = @($auditSnapshots.items | Where-Object { $_ } | Select-Object name,storageBytes,storageLocations,status,creationTimestamp)
        cloudRun = @($auditRun.services | Where-Object { $_ } | ForEach-Object {
            [pscustomobject]@{ name=($_.name -split '/')[-1]; scaling=$_.scaling; revisionScaling=$_.template.scaling; concurrency=$_.template.maxInstanceRequestConcurrency; resources=@($_.template.containers.resources); traffic=@($_.traffic); latestReadyRevision=$_.latestReadyRevision }
        })
        errors = @(@($auditInstances,$auditDisks,$auditAddresses,$auditSnapshots,$auditRouters,$auditRules,$auditRun) | Where-Object { $_.auditError } | ForEach-Object {$_.auditError})
        pagination = [pscustomobject]@{instances=$auditInstances.nextPageToken;disks=$auditDisks.nextPageToken;addresses=$auditAddresses.nextPageToken;snapshots=$auditSnapshots.nextPageToken;routers=$auditRouters.nextPageToken;forwardingRules=$auditRules.nextPageToken;cloudRun=$auditRun.nextPageToken}
    }
    $auditResults.projects += [pscustomobject]$auditEntry
    Write-Output "$auditProject : $($auditEntry.instances.Count) VMs, $($auditEntry.disks.Count) disks, $($auditEntry.cloudRun.Count) Cloud Run services, $($auditEntry.errors.Count) API errors."
}
$auditResults | ConvertTo-Json -Depth 18 | Set-Content -LiteralPath (Join-Path $auditDirectory 'live-inventory.json') -Encoding utf8
$auditHeaders=$null
$auditToken=$null

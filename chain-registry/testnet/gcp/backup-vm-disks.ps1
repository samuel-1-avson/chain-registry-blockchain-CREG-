<#
.SYNOPSIS
  Create on-demand GCP persistent-disk snapshots for the CREG testnet VMs.

.DESCRIPTION
  Disaster-recovery backup for the single-zone GCP deployment (the node's
  authoritative state — RocksDB chain, validator-set cursor, bridge anchor
  journal, validator registrations — lives on the validator VM's persistent
  disk). Snapshots are point-in-time and can be restored into a new disk/VM,
  including in another zone for zonal-outage recovery.

  Run on a schedule (Task Scheduler / cron / Cloud Scheduler) and prune old
  snapshots with -RetentionDays. For continuous protection, prefer a GCP
  resource snapshot schedule (see docs/OPS_HARDENING_RUNBOOK.md); this script
  covers on-demand and Windows-operator workflows.

.EXAMPLE
  ./backup-vm-disks.ps1 -Project gen-lang-client-0022105784 -Zone us-central1-a

.EXAMPLE
  ./backup-vm-disks.ps1 -RetentionDays 14 -Prune
#>
[CmdletBinding()]
param(
    [string]$Project = $env:CREG_GCP_PROJECT,
    [string]$Zone = "us-central1-a",
    # Disks to snapshot. Defaults to the three CREG testnet VM boot disks.
    [string[]]$Disks = @("creg-validator-vm", "creg-testnet-vm", "creg-sepolia-geth-vm"),
    [int]$RetentionDays = 14,
    [switch]$Prune
)

$ErrorActionPreference = "Stop"

if (-not $Project) {
    throw "Set -Project or the CREG_GCP_PROJECT environment variable."
}

$gcloud = (Get-Command gcloud -ErrorAction SilentlyContinue)
if (-not $gcloud) {
    throw "gcloud CLI not found on PATH. Install the Google Cloud SDK first."
}

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
Write-Host "[backup] project=$Project zone=$Zone disks=$($Disks -join ',')"

foreach ($disk in $Disks) {
    $snapshot = "$disk-$stamp"
    Write-Host "[backup] snapshotting $disk -> $snapshot"
    & gcloud compute snapshots create $snapshot `
        --source-disk $disk `
        --source-disk-zone $Zone `
        --project $Project `
        --storage-location "us" `
        --labels "app=creg-testnet,disk=$disk,kind=dr-backup" 2>&1 | Write-Host
    if ($LASTEXITCODE -ne 0) {
        Write-Warning "[backup] snapshot FAILED for $disk (exit $LASTEXITCODE)"
    } else {
        Write-Host "[backup] OK $snapshot"
    }
}

if ($Prune) {
    $cutoff = (Get-Date).ToUniversalTime().AddDays(-$RetentionDays)
    Write-Host "[prune] removing creg-testnet snapshots older than $($cutoff.ToString('u'))"
    $json = & gcloud compute snapshots list `
        --project $Project `
        --filter "labels.app=creg-testnet" `
        --format "json" 2>&1
    if ($LASTEXITCODE -ne 0) {
        Write-Warning "[prune] could not list snapshots; skipping prune"
    } else {
        $snaps = $json | ConvertFrom-Json
        foreach ($s in $snaps) {
            $created = [datetime]::Parse($s.creationTimestamp).ToUniversalTime()
            if ($created -lt $cutoff) {
                Write-Host "[prune] deleting $($s.name) (created $($created.ToString('u')))"
                & gcloud compute snapshots delete $s.name --project $Project --quiet 2>&1 | Write-Host
            }
        }
    }
}

Write-Host "[backup] done."

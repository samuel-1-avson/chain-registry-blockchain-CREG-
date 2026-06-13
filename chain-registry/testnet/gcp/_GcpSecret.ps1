function Get-GcpSecretValue {
    param(
        [Parameter(Mandatory)]
        [string]$ProjectId,
        [Parameter(Mandatory)]
        [string]$SecretId,
        [string]$Version = "latest"
    )

    $raw = & gcloud secrets versions access $Version --secret=$SecretId --project=$ProjectId 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to read secret '$SecretId': $raw"
    }
    return ([string]$raw).Trim()
}

function Set-GcpSecretValue {
    param(
        [Parameter(Mandatory)]
        [string]$ProjectId,
        [Parameter(Mandatory)]
        [string]$SecretId,
        [Parameter(Mandatory)]
        [string]$Value
    )

    $tmp = [System.IO.Path]::GetTempFileName()
    try {
        [System.IO.File]::WriteAllText($tmp, $Value)
        $prevEap = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        & gcloud secrets describe $SecretId --project=$ProjectId 2>$null | Out-Null
        $exists = ($LASTEXITCODE -eq 0)
        $ErrorActionPreference = $prevEap
        if (-not $exists) {
            & gcloud secrets create $SecretId --project=$ProjectId --replication-policy=automatic --data-file=$tmp
        } else {
            & gcloud secrets versions add $SecretId --project=$ProjectId --data-file=$tmp
        }
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to write secret '$SecretId'"
        }
    } finally {
        Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue
    }
}

function Resolve-AlertConfigFromGsm {
    param(
        [hashtable]$Config,
        [string]$ProjectId,
        [scriptblock]$Log = { param($m) Write-Host $m }
    )

    $pairs = @(
        @{ Env = "GCP_ALERT_NTFY_TOPIC"; Secret = "GCP_ALERT_NTFY_TOPIC_SECRET"; Default = "creg-testnet-alert-ntfy-topic" }
        @{ Env = "GCP_ALERT_SLACK_WEBHOOK_URL"; Secret = "GCP_ALERT_SLACK_WEBHOOK_SECRET"; Default = "creg-testnet-alert-slack-webhook" }
        @{ Env = "GCP_ALERT_WEBHOOK_URL"; Secret = "GCP_ALERT_WEBHOOK_SECRET"; Default = "creg-testnet-alert-webhook" }
        @{ Env = "GCP_ALERT_PAGERDUTY_ROUTING_KEY"; Secret = "GCP_ALERT_PAGERDUTY_SECRET"; Default = "creg-testnet-alert-pagerduty-key" }
        @{ Env = "GCP_ALERT_SMTP_PASSWORD"; Secret = "GCP_ALERT_SMTP_PASSWORD_SECRET"; Default = "creg-testnet-alert-smtp-password" }
    )

    foreach ($pair in $pairs) {
        if ($Config[$pair.Env]) { continue }
        $secretId = $Config[$pair.Secret]
        if (-not $secretId) { $secretId = $pair.Default }
        try {
            $Config[$pair.Env] = Get-GcpSecretValue -ProjectId $ProjectId -SecretId $secretId
            & $Log "Loaded $($pair.Env) from Secret Manager ($secretId)"
        } catch {
            & $Log "Secret $($pair.Env) not in env or GSM ($secretId): skipped"
        }
    }

    return $Config
}

# Configure Alertmanager external receivers in hosting.env and redeploy monitoring.
#
# Default channel is ntfy.sh (mobile push, no Slack account). Alternatives:
#   -Email          SMTP email (set GCP_ALERT_SMTP_* in hosting.env)
#   -WebhookUrl     Discord / Google Chat / custom URL
#   -SlackWebhookUrl Slack incoming webhook (optional)
#
# Usage:
#   .\testnet\gcp\setup-alert-receiver.ps1
#   .\testnet\gcp\setup-alert-receiver.ps1 -Channel ntfy
#   .\testnet\gcp\setup-alert-receiver.ps1 -Channel webhook -WebhookUrl https://discord.com/api/webhooks/...

param(
    [ValidateSet("ntfy", "email", "webhook", "slack")]
    [string]$Channel = "ntfy",
    [string]$NtfyTopic = "",
    [string]$WebhookUrl = "",
    [string]$SlackWebhookUrl = "",
    [switch]$SkipDeploy
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$envFile = Join-Path $gcpDir "hosting.env"
. (Join-Path $gcpDir "_GcpSecret.ps1")
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")
$projectId = $cfg.GCP_PROJECT

function Log($m) { Write-Host "[setup-alert-receiver] $m" }

if (-not (Test-Path $envFile)) {
    throw "Missing $envFile - copy hosting.env.example first"
}

function Set-EnvLine {
    param([string]$Key, [string]$Value)
    $lines = Get-Content $envFile
    $pattern = "^\s*$([regex]::Escape($Key))\s*="
    $updated = $false
    $out = foreach ($line in $lines) {
        if ($line -match $pattern) {
            $updated = $true
            "$Key=$Value"
        } else {
            $line
        }
    }
    if (-not $updated) {
        $out += "$Key=$Value"
    }
    Set-Content -Path $envFile -Value $out -Encoding utf8
}

function Remove-EnvLine {
    param([string]$Key)
    $pattern = "^\s*$([regex]::Escape($Key))\s*="
    $out = Get-Content $envFile | Where-Object { $_ -notmatch $pattern }
    Set-Content -Path $envFile -Value $out -Encoding utf8
}

switch ($Channel) {
    "ntfy" {
        if (-not $NtfyTopic) {
            $suffix = (Get-Random -Minimum 100000 -Maximum 999999)
            $NtfyTopic = "creg-testnet-alerts-$suffix"
        }
        if (-not $projectId) { throw "GCP_PROJECT missing in hosting.env" }
        $secretId = if ($cfg.GCP_ALERT_NTFY_TOPIC_SECRET) { $cfg.GCP_ALERT_NTFY_TOPIC_SECRET } else { "creg-testnet-alert-ntfy-topic" }
        Set-GcpSecretValue -ProjectId $projectId -SecretId $secretId -Value $NtfyTopic
        Set-EnvLine -Key "GCP_ALERT_NTFY_TOPIC_SECRET" -Value $secretId
        Set-EnvLine -Key "GCP_ALERT_NTFY_SERVER" -Value "https://ntfy.sh"
        Remove-EnvLine -Key "GCP_ALERT_NTFY_TOPIC"
        Log "ntfy topic stored in Secret Manager ($secretId)"
        Log "Subscribe on phone: install ntfy app, add topic '$NtfyTopic'"
        Log "Web: https://ntfy.sh/$NtfyTopic"
    }
    "webhook" {
        if (-not $WebhookUrl) { throw "-WebhookUrl required for webhook channel" }
        if (-not $projectId) { throw "GCP_PROJECT missing in hosting.env" }
        $secretId = if ($cfg.GCP_ALERT_WEBHOOK_SECRET) { $cfg.GCP_ALERT_WEBHOOK_SECRET } else { "creg-testnet-alert-webhook" }
        Set-GcpSecretValue -ProjectId $projectId -SecretId $secretId -Value $WebhookUrl
        Set-EnvLine -Key "GCP_ALERT_WEBHOOK_SECRET" -Value $secretId
        Remove-EnvLine -Key "GCP_ALERT_WEBHOOK_URL"
        Log "Generic webhook stored in Secret Manager ($secretId)"
    }
    "slack" {
        if (-not $SlackWebhookUrl) { throw "-SlackWebhookUrl required for slack channel" }
        if (-not $projectId) { throw "GCP_PROJECT missing in hosting.env" }
        $secretId = if ($cfg.GCP_ALERT_SLACK_WEBHOOK_SECRET) { $cfg.GCP_ALERT_SLACK_WEBHOOK_SECRET } else { "creg-testnet-alert-slack-webhook" }
        Set-GcpSecretValue -ProjectId $projectId -SecretId $secretId -Value $SlackWebhookUrl
        Set-EnvLine -Key "GCP_ALERT_SLACK_WEBHOOK_SECRET" -Value $secretId
        Remove-EnvLine -Key "GCP_ALERT_SLACK_WEBHOOK_URL"
        Log "Slack webhook stored in Secret Manager ($secretId)"
    }
    "email" {
        Log "Email channel uses GCP_ALERT_EMAIL_TO and GCP_ALERT_SMTP_* in hosting.env"
        Log "Example: GCP_ALERT_EMAIL_TO=you@example.com, GCP_ALERT_SMTP_HOST=smtp.gmail.com:587"
    }
}

if (-not $SkipDeploy) {
    Log "Redeploying monitoring stack..."
    & (Join-Path $gcpDir "deploy-monitoring.ps1") -SkipSync
    if ($LASTEXITCODE -ne 0) { throw "deploy-monitoring failed" }
    & (Join-Path $gcpDir "verify-monitoring.ps1")
    if ($LASTEXITCODE -ne 0) { throw "verify-monitoring failed" }
}

Log "Done."

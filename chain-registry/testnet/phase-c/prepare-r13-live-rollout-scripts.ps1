$ErrorActionPreference = 'Stop'
$phase = $PSScriptRoot
$repo = (Resolve-Path (Join-Path $phase '../../..')).Path

function Write-Variant([string]$SourceName, [string]$TargetName, [object[]]$Replacements) {
    $sourcePath = Join-Path $phase $SourceName
    $targetPath = Join-Path $phase $TargetName
    if (Test-Path -LiteralPath $targetPath) { throw "Refusing to overwrite $targetPath" }
    $text = [IO.File]::ReadAllText($sourcePath)
    foreach ($entry in $Replacements) {
        if (-not $text.Contains($entry.From)) {
            throw "Expected source text is missing from ${SourceName}: $($entry.From)"
        }
        $text = $text.Replace($entry.From, $entry.To)
    }
    [IO.File]::WriteAllText($targetPath, $text, [Text.UTF8Encoding]::new($false))
}

$labels = @(
    @{ From = 'R12'; To = 'R13' },
    @{ From = 'r12'; To = 'r13' }
)
$helper = @($labels) + @(
    @{ From = 'old_scanner=$base/scanner-r11-osv-kleur; scanner=$base/scanner-r13-osv-escape-live'; To = 'old_scanner=$base/scanner-r12-osv-escape-live; scanner=$base/scanner-r13-osv-has-flag-live' },
    @{ From = '$container-r11-before-r13-$run_id'; To = '$container-r12-before-r13-$run_id' },
    @{ From = 'epoch=osv-2026-09-27-phase-c-escape-string-regexp-r13-live'; To = 'epoch=osv-2026-09-28-phase-c-has-flag-r13-live' },
    @{ From = 'old_epoch=osv-2026-09-27-phase-c-kleur-r11'; To = 'old_epoch=osv-2026-09-27-phase-c-escape-string-regexp-r12-live' },
    @{ From = 'capture_env=$base/node.env.r11-preserved-$run_id'; To = 'capture_env=$base/node.env.r12-preserved-$run_id' },
    @{ From = 'old_profile=f397fac29df29438f0ee15580547def5fe66a03af60153d22993f4a7af9fce8c'; To = 'old_profile=e067b8fc4aee823203a8a8204d2b57a4a097798d739d338bf8a45b4ff7b5ceb2' },
    @{ From = "`nprofile=e067b8fc4aee823203a8a8204d2b57a4a097798d739d338bf8a45b4ff7b5ceb2"; To = "`nprofile=8c7f011ab71a9a695ce1364e9958a2906f02645849c3567fc5691206a93c296e" },
    @{ From = '2ece0610b4b808ee5382704b702917dd6f2f3f75d6eeb2a8585aaa9e270507aa'; To = '4bd594d71b4fc8bc785c34b1e46291518328233c8aed616171efd9add073e260' },
    @{ From = '==14'; To = '==16' },
    @{ From = "'height':14"; To = "'height':16" },
    @{ From = 'signed_r7_h14'; To = 'signed_r7_h16' },
    @{ From = 'creg_scanner_osv_lookups 5'; To = 'creg_scanner_osv_lookups 6' },
    @{ From = "value('creg_scanner_osv_lookups')==6"; To = "value('creg_scanner_osv_lookups')==7" },
    @{ From = '981e3388bc2b79b71daebb63f0c467bd11e06ff481766e9b9f12615de7d77928'; To = '642c35a82a9296c36bfc5b79c152b3a78651d7c50d646206bc004620caa0925a' },
    @{ From = 'ad5720ece2376ebf3bace85456988a74463a599f3081b49c924da93d84cc0efa'; To = '510c237d49a709a57606427cdbbda6caa989864b8a8e0313b797eb29cd25eeab' },
    @{ From = '5b3aeb024b6e6bb0b82257aae81cb680b8d2aaff48b7753056493559e43861b1'; To = 'c97381b9df48722a1f591c8f81c87e4d9695c0b89877e6b609e883abf7e4a723' }
)
Write-Variant 'rollout-r12-scanner-linux-node.sh' 'rollout-r13-scanner-linux-node.sh' $helper

$fleet = @($labels) + @(
    @{ From = '$oldProfile=''f397fac29df29438f0ee15580547def5fe66a03af60153d22993f4a7af9fce8c'''; To = '$oldProfile=''e067b8fc4aee823203a8a8204d2b57a4a097798d739d338bf8a45b4ff7b5ceb2''' },
    @{ From = '$newProfile=''e067b8fc4aee823203a8a8204d2b57a4a097798d739d338bf8a45b4ff7b5ceb2'''; To = '$newProfile=''8c7f011ab71a9a695ce1364e9958a2906f02645849c3567fc5691206a93c296e''' },
    @{ From = 'osv-2026-09-27-phase-c-escape-string-regexp-r13-live'; To = 'osv-2026-09-28-phase-c-has-flag-r13-live' },
    @{ From = '981e3388bc2b79b71daebb63f0c467bd11e06ff481766e9b9f12615de7d77928'; To = '642c35a82a9296c36bfc5b79c152b3a78651d7c50d646206bc004620caa0925a' },
    @{ From = 'ad5720ece2376ebf3bace85456988a74463a599f3081b49c924da93d84cc0efa'; To = '510c237d49a709a57606427cdbbda6caa989864b8a8e0313b797eb29cd25eeab' },
    @{ From = '5b3aeb024b6e6bb0b82257aae81cb680b8d2aaff48b7753056493559e43861b1'; To = 'c97381b9df48722a1f591c8f81c87e4d9695c0b89877e6b609e883abf7e4a723' },
    @{ From = 'npm:escape-string-regexp@5.0.0'; To = 'npm:has-flag@5.0.1' },
    @{ From = '2ece0610b4b808ee5382704b702917dd6f2f3f75d6eeb2a8585aaa9e270507aa'; To = '4bd594d71b4fc8bc785c34b1e46291518328233c8aed616171efd9add073e260' },
    @{ From = 'function Get-Fleet([string]$Label,[string[]]$CaptureSlots=@(){'; To = "function Get-Fleet([string]`$Label,[string[]]`$CaptureSlots=@()){`n `$captureProfile=if(`$Label -eq 'complete'){`$newProfile}else{`$oldProfile}" },
    @{ From = '& $readiness -EvidencePath $path -ScannerProfileDigest $oldProfile'; To = '& $readiness -EvidencePath $path -ScannerProfileDigest $captureProfile' },
    @{ From = '$cap.nodes.Count -ne 8 -or -not $cap.all_eight_recovery_clear -or -not $cap.all_eight_pbft_clear'; To = '$cap.nodes.Count -ne 8 -or -not $cap.all_eight_recovery_clear -or -not $cap.all_eight_pbft_clear -or ($Label -eq ''complete'' -and -not $cap.all_eight_ready)' },
    @{ From = 'tip_height -ne 14'; To = 'tip_height -ne 16' },
    @{ From = 'finalized_height -ne 14'; To = 'finalized_height -ne 16' }
)
Write-Variant 'rollout-r12-scanner-fleet.ps1' 'rollout-r13-scanner-fleet.ps1' $fleet

$local = @($labels) + @(
    @{ From = '$oldScanner = ''C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r11-osv-kleur'''; To = '$oldScanner = ''C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r13-live-escape''' },
    @{ From = '$oldScanner = ''C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r13-live-escape'''; To = '$oldScanner = ''C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r12-live-escape''' },
    @{ From = '$newScanner = ''C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r13-live-escape'''; To = '$newScanner = ''C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r13-live-has-flag''' },
    @{ From = '$preserved = "$container-r11-before-r13-live-$runId"'; To = '$preserved = "$container-r12-before-r13-live-$runId"' },
    @{ From = '$envBackup = "$envPath.r11-preserved-$runId"'; To = '$envBackup = "$envPath.r12-preserved-for-r13-$runId"' },
    @{ From = '$oldEpoch = ''osv-2026-09-27-phase-c-kleur-r11'''; To = '$oldEpoch = ''osv-2026-09-27-phase-c-escape-string-regexp-r12-live''' },
    @{ From = '$newEpoch = ''osv-2026-09-27-phase-c-escape-string-regexp-r13-live'''; To = '$newEpoch = ''osv-2026-09-28-phase-c-has-flag-r13-live''' },
    @{ From = '$oldProfile = ''f397fac29df29438f0ee15580547def5fe66a03af60153d22993f4a7af9fce8c'''; To = '$oldProfile = ''e067b8fc4aee823203a8a8204d2b57a4a097798d739d338bf8a45b4ff7b5ceb2''' },
    @{ From = '$newProfile = ''e067b8fc4aee823203a8a8204d2b57a4a097798d739d338bf8a45b4ff7b5ceb2'''; To = '$newProfile = ''8c7f011ab71a9a695ce1364e9958a2906f02645849c3567fc5691206a93c296e''' },
    @{ From = '981e3388bc2b79b71daebb63f0c467bd11e06ff481766e9b9f12615de7d77928'; To = '642c35a82a9296c36bfc5b79c152b3a78651d7c50d646206bc004620caa0925a' },
    @{ From = 'ad5720ece2376ebf3bace85456988a74463a599f3081b49c924da93d84cc0efa'; To = '510c237d49a709a57606427cdbbda6caa989864b8a8e0313b797eb29cd25eeab' },
    @{ From = '5b3aeb024b6e6bb0b82257aae81cb680b8d2aaff48b7753056493559e43861b1'; To = 'c97381b9df48722a1f591c8f81c87e4d9695c0b89877e6b609e883abf7e4a723' },
    @{ From = '2ece0610b4b808ee5382704b702917dd6f2f3f75d6eeb2a8585aaa9e270507aa'; To = '4bd594d71b4fc8bc785c34b1e46291518328233c8aed616171efd9add073e260' },
    @{ From = 'npm:escape-string-regexp@5.0.0'; To = 'npm:has-flag@5.0.1' },
    @{ From = "creg_scanner_osv_lookups') -ne 5"; To = "creg_scanner_osv_lookups') -ne 6" },
    @{ From = "creg_scanner_osv_lookups') -eq 6"; To = "creg_scanner_osv_lookups') -eq 7" },
    @{ From = 'tip_height -ne 14'; To = 'tip_height -ne 16' },
    @{ From = 'finalized_height -ne 14'; To = 'finalized_height -ne 16' },
    @{ From = 'tip_height -eq 14'; To = 'tip_height -eq 16' },
    @{ From = 'finalized_height -eq 14'; To = 'finalized_height -eq 16' }
)
Write-Variant 'rollout-r12-live-local-v1.ps1' 'rollout-r13-live-local-v1.ps1' $local

Write-Output 'Created R13 local, remote fleet, and Linux node rollout scripts; R12 sources were left intact.'

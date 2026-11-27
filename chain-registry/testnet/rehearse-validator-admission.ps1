# Read R7 admission state, export wallet typed data, or simulate an approval bundle.
# All Sepolia operations in this script are reads (eth_call for simulation).
[CmdletBinding()]
param(
    [string]$Applicant,
    [string]$RpcUrl,
    [string]$TypedDataPath,
    [string]$SignaturesPath,
    [string[]]$SignerAddress = @(),
    [Alias('Rehearse')]
    [switch]$CheckOnly
)

$ErrorActionPreference = 'Stop'
$staking = '0x61c00EBe7C46f0bcE63942F7E15Bcc434DFC9B7C'
$chainId = 11155111
$addressPattern = '^0x[0-9a-fA-F]{40}$'
$cast = Join-Path $PSScriptRoot '.tools/foundry/cast.exe'
if (-not (Test-Path -LiteralPath $cast -PathType Leaf)) {
    $cast = (Get-Command cast -ErrorAction SilentlyContinue).Source
}
if (-not $cast) { throw 'Install Foundry cast or use the bundled testnet executable.' }
if (($TypedDataPath -or $SignaturesPath) -and -not $Applicant) {
    throw 'Specify the pending applicant wallet with -Applicant.'
}
if ($Applicant -and $Applicant -cnotmatch $addressPattern) { throw 'Invalid applicant wallet address.' }
if ($CheckOnly -and $TypedDataPath) { throw 'Omit -CheckOnly to write the typed-data file.' }

function Invoke-Cast([string[]]$CastArgs) {
    # Windows PowerShell can treat harmless native stderr as a terminating error.
    $ErrorActionPreference = 'Continue'
    $PSNativeCommandUseErrorActionPreference = $false
    $output = & $cast @CastArgs 2>$null
    if ($LASTEXITCODE -ne 0) { throw "Foundry $($CastArgs[0]) failed." }
    return ($output -join "`n").Trim()
}

function Read-Staking([string]$Signature, [string[]]$Values = @(), [string]$AtBlock = $blockNumber) {
    $value = Invoke-Cast (@('call', $staking, $Signature) + $Values + @('--block', $AtBlock))
    if ($value -cnotmatch '^0x(?:[0-9a-fA-F]{64})*$') {
        throw "Unexpected staking response for $Signature."
    }
    return $value.Substring(2).ToLowerInvariant()
}

function Read-Word([string]$Hex, [int]$Index = 0) {
    if ($Hex.Length -lt (($Index + 1) * 64)) { throw 'Incomplete staking contract response.' }
    return [bigint]::Parse(('0' + $Hex.Substring($Index * 64, 64)), [Globalization.NumberStyles]::AllowHexSpecifier)
}

$oldRpc = $env:ETH_RPC_URL
if (-not $RpcUrl) { $RpcUrl = $env:SEPOLIA_RPC_URL }
if (-not $RpcUrl) { $RpcUrl = $oldRpc }
if (-not $RpcUrl) { $RpcUrl = 'https://sepolia.gateway.tenderly.co' }
$env:ETH_RPC_URL = $RpcUrl
try {
    if ((Invoke-Cast @('chain-id')) -cne "$chainId") { throw 'The RPC must serve Ethereum Sepolia.' }
    $block = Invoke-Cast @('block', 'finalized', '--json') | ConvertFrom-Json
    $blockNumber = [Convert]::ToUInt64($block.number.Substring(2), 16).ToString()
    $active = Read-Word (Read-Staking 'activeValidatorCount()')
    if ($active -eq 0) { throw 'The R7 staking contract reports zero active validators.' }
    $required = [int][Math]::Ceiling(([double]$active * 2) / 3)
    $epoch = Read-Word (Read-Staking 'validatorSetEpoch()')
    $ruleVersion = Read-Word (Read-Staking 'RULE_SET_VERSION()')
    $report = [ordered]@{
        network = 'R7 Sepolia'
        staking_contract = $staking
        finalized_sepolia_block = $blockNumber
        active_validators = [int]$active
        required_approvals = $required
        validator_set_epoch = [long]$epoch
        rule_set_version = "$ruleVersion"
    }
    if ($SignerAddress.Count -gt 0) {
        $report.signer_checks = @(
            foreach ($address in $SignerAddress) {
                if ($address -cnotmatch $addressPattern) { throw 'Invalid signer wallet address.' }
                [ordered]@{
                    address = $address.ToLowerInvariant()
                    active = (Read-Word (Read-Staking 'isActiveValidator(address)' @($address))) -eq 1
                }
            }
        )
    }
    if (-not $Applicant) {
        $report | ConvertTo-Json
        return
    }

    $Applicant = $Applicant.ToLowerInvariant()
    $entry = Read-Staking 'validators(address)' @($Applicant)
    $stake = Read-Word $entry
    $state = Read-Word $entry 1
    $nonce = Read-Word $entry 5
    $minimum = Read-Word (Read-Staking 'minValidatorStake()')
    $timeout = Read-Word (Read-Staking 'APPLICATION_TIMEOUT()')
    $expires = [DateTimeOffset]::FromUnixTimeSeconds([long]($nonce + $timeout))
    if ($state -ne 1 -or $stake -lt $minimum -or [DateTimeOffset]::UtcNow -ge $expires) {
        throw 'The applicant must have a pending, funded application within its approval window.'
    }
    $digest = Read-Staking 'consensusMessageHash(address,uint256,uint256)' @($Applicant, "$stake", "$nonce")
    $report.applicant = $Applicant
    $report.application_nonce = [long]$nonce
    $report.stake_wei = "$stake"
    $report.expires_utc = $expires.ToString('o')
    $report.digest = "0x$digest"

    if ($TypedDataPath) {
        $typedData = @{
            types = @{
                EIP712Domain = @(
                    @{name='name'; type='string'}
                    @{name='version'; type='string'}
                    @{name='chainId'; type='uint256'}
                    @{name='verifyingContract'; type='address'}
                )
                ValidatorAdmission = @(
                    @{name='applicant'; type='address'}
                    @{name='stake'; type='uint256'}
                    @{name='nonce'; type='uint256'}
                    @{name='ruleSetVersion'; type='uint256'}
                    @{name='validatorSetEpoch'; type='uint256'}
                )
            }
            primaryType = 'ValidatorAdmission'
            domain = @{name='Chain Registry Validator Admission'; version='1'; chainId=$chainId; verifyingContract=$staking}
            message = @{applicant=$Applicant; stake="$stake"; nonce="$nonce"; ruleSetVersion="$ruleVersion"; validatorSetEpoch="$epoch"}
        }
        $path = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($TypedDataPath)
        $file = [IO.File]::Open($path, [IO.FileMode]::CreateNew)
        try {
            $json = $typedData | ConvertTo-Json -Depth 7
            $bytes = [Text.Encoding]::UTF8.GetBytes($json)
            $file.Write($bytes, 0, $bytes.Length)
        } finally {
            $file.Dispose()
        }
        $report.typed_data_file = $path
    }

    if ($SignaturesPath) {
        $entries = @(Get-Content -Raw -LiteralPath $SignaturesPath | ConvertFrom-Json)
        $signatures = @{}
        foreach ($item in $entries) {
            if ($item.signer -cnotmatch $addressPattern -or $item.signature -cnotmatch '^0x[0-9a-fA-F]{130}$') {
                throw 'Each approval must contain a signer address and a 65-byte hex signature.'
            }
            $signer = $item.signer.ToLowerInvariant()
            if ($signatures.ContainsKey($signer)) { throw "Duplicate approval from $signer." }
            $signatures[$signer] = $item.signature
        }
        if ($signatures.Count -lt $required) {
            throw "The bundle contains $($signatures.Count) distinct approvals. R7 requires $required."
        }
        $signers = @($signatures.Keys | Sort-Object)
        $values = @($signers | ForEach-Object { $signatures[$_] })
        # The deployed contract checks EIP-712 recovery, exact epoch, active
        # membership, nonce, and quorum. Use latest state for this final simulation.
        $calldata = Invoke-Cast (@('calldata', 'approveByConsensus(address,uint256,address[],bytes[])') + @(
            $Applicant, "$nonce", ('[' + ($signers -join ',') + ']'), ('[' + ($values -join ',') + ']')
        ))
        try {
            $null = Invoke-Cast @('call', $staking, '--data', $calldata, '--block', 'latest')
        } catch {
            throw 'Sepolia approval simulation failed. Check the signatures and current validator-set epoch.'
        }
        $report.simulation = 'passed'
        $report.signers = $signers
        $report.approval_calldata = $calldata
    }

    $report | ConvertTo-Json -Depth 5
} finally {
    $env:ETH_RPC_URL = $oldRpc
}

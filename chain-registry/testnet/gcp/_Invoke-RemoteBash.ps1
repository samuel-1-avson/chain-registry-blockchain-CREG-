# Run a bash script on a GCP VM over IAP without passing shell syntax through
# gcloud's --command. On Windows, gcloud is a .cmd wrapper, so cmd.exe re-parses
# --command and mangles quotes, pipes and redirections. The script is copied
# to the VM as a file instead, and only plain arguments (digests, addresses,
# numbers) go on the command line.
#
# Dot-source this file, then:
#   $lines = Invoke-RemoteBash -ProjectId p -Vm vm -Zone z -Script $bash -Arguments @('a', 'b')

function Invoke-RemoteBash {
    param(
        [Parameter(Mandatory)] [string]$ProjectId,
        [Parameter(Mandatory)] [string]$Vm,
        [Parameter(Mandatory)] [string]$Zone,
        [Parameter(Mandatory)] [string]$Script,
        [string[]]$Arguments = @()
    )
    foreach ($argument in $Arguments) {
        if ($argument -notmatch '^[A-Za-z0-9@:/._=,+-]+$') {
            throw "Remote argument '$argument' contains characters that cmd.exe could reinterpret."
        }
    }
    $name = 'creg-remote-' + [Guid]::NewGuid().ToString('N').Substring(0, 12) + '.sh'
    $local = Join-Path ([IO.Path]::GetTempPath()) $name
    # The script removes itself on exit, including when it fails early.
    $body = "trap 'rm -f -- `"`$0`"' EXIT`n" + ($Script -replace "`r`n", "`n") + "`n"
    [IO.File]::WriteAllText($local, $body, (New-Object Text.UTF8Encoding($false)))
    $common = @("--project=$ProjectId", "--zone=$Zone", '--tunnel-through-iap', '--strict-host-key-checking=no', '--quiet')
    try {
        $ErrorActionPreference = 'Continue'
        $copy = & gcloud compute scp $local "${Vm}:/tmp/$name" @common 2>&1
        if ($LASTEXITCODE -ne 0) { throw "Copying the remote script to $Vm failed: $(($copy | Select-Object -Last 2) -join ' ')" }
        $command = (@('bash', "/tmp/$name") + $Arguments) -join ' '
        $out = & gcloud compute ssh $Vm @common "--command=$command" 2>&1
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = 'Stop'
        Remove-Item -LiteralPath $local -ErrorAction SilentlyContinue
    }
    $lines = @($out | ForEach-Object { $_.ToString() })
    if ($code -ne 0) {
        throw "Remote script on $Vm failed (exit $code): $(($lines | Select-Object -Last 4) -join ' | ')"
    }
    return $lines
}

$json1 = Get-Content playground/block1.json | ConvertFrom-Json
$json2 = Get-Content playground/block2.json | ConvertFrom-Json

Write-Host "Block 1 Transactions:"
foreach ($tx in $json1.transactions) {
    if ($tx.Publish) {
        Write-Host ("- PUBLISH: " + $tx.Publish.id.ecosystem + ":" + $tx.Publish.id.name + "@" + $tx.Publish.id.version)
    }
    if ($tx.Revoke) {
        Write-Host ("- REVOKE:  " + $tx.Revoke.package_canonical + " (reason: " + $tx.Revoke.reason + ")")
    }
}

Write-Host "Block 2 Transactions:"
foreach ($tx in $json2.transactions) {
    if ($tx.Publish) {
        Write-Host ("- PUBLISH: " + $tx.Publish.id.ecosystem + ":" + $tx.Publish.id.name + "@" + $tx.Publish.id.version)
    }
    if ($tx.Revoke) {
        Write-Host ("- REVOKE:  " + $tx.Revoke.package_canonical + " (reason: " + $tx.Revoke.reason + ")")
    }
}

$env:IPFS_URL="http://127.0.0.1:5001"
$key = Get-Content playground/publisher.key -Raw
$key = $key.Trim()

$nodes = @("http://127.0.0.1:8080", "http://127.0.0.1:8082", "http://127.0.0.1:8083")

function Multi-Publish($path, $manifest) {
    foreach ($node in $nodes) {
        Write-Host "Publishing to $node..."
        cargo run -q --bin creg -- publish $path -k $key -m $manifest --node-url $node > $null 2>&1
    }
}

Write-Host "--- Publishing Good v1 to all nodes ---"
Multi-Publish "playground/good-pkg-v1/good-pkg-v1.tgz" "playground/good-pkg-v1/manifest.json"

Write-Host "--- Publishing Good v2 to all nodes ---"
Multi-Publish "playground/good-pkg-v2/good-pkg-v2.tgz" "playground/good-pkg-v2/manifest.json"

Write-Host "--- Publishing Evil v1 to all nodes ---"
Multi-Publish "playground/evil-pkg-v1/evil-pkg-v1.tgz" "playground/evil-pkg-v1/manifest.json"

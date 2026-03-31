$env:IPFS_URL="http://127.0.0.1:5001"
$env:CREG_NODE_URL="http://127.0.0.1:8080"
$key = Get-Content playground/publisher.key -Raw
$key = $key.Trim()

Write-Host "--- Publishing Good v1 ---"
cargo run -q --bin creg -- publish playground/good-pkg-v1/good-pkg-v1.tgz -k $key -m playground/good-pkg-v1/manifest.json > playground/good-pkg-v1.log 2>&1

Write-Host "--- Publishing Good v2 ---"
cargo run -q --bin creg -- publish playground/good-pkg-v2/good-pkg-v2.tgz -k $key -m playground/good-pkg-v2/manifest.json > playground/good-pkg-v2.log 2>&1

Write-Host "--- Publishing Evil v1 ---"
cargo run -q --bin creg -- publish playground/evil-pkg-v1/evil-pkg-v1.tgz -k $key -m playground/evil-pkg-v1/manifest.json > playground/evil-pkg-v1.log 2>&1

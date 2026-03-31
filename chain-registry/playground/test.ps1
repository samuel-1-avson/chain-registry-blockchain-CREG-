$env:IPFS_URL="http://127.0.0.1:5001"
$env:CREG_NODE_URL="http://127.0.0.1:8080"
$key = Get-Content playground/publisher.key
cargo run --bin creg -- publish playground/dummy-pkg/dummy-pkg.tgz -k $key -m playground/manifest.json > playground/publish-test.log 2>&1
Get-Content playground/publish-test.log

$env:IPFS_URL="http://127.0.0.1:5001"
$env:CREG_NODE_URL="http://127.0.0.1:8080"

Write-Host "--- Testing: Config Show ---"
cargo run -q --bin creg -- config show

Write-Host "`n--- Testing: Advanced ML-Verify ---"
cargo run -q --bin creg -- advanced ml-verify playground/dummy-pkg/dummy-pkg.tgz -e npm

Write-Host "`n--- Testing: Cache Status ---"
cargo run -q --bin creg -- cache

Write-Host "`n--- Testing: Lockfile Status ---"
cargo run -q --bin creg -- lockfile -d playground

Write-Host "`n--- Testing: Status for arbitrary package ---"
cargo run -q --bin creg -- status colors@1.4.0 -e npm

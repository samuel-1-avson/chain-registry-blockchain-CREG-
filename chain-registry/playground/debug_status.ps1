$env:CREG_NODE_URL = "http://127.0.0.1:8080"
Write-Host "Checking good-pkg@1.0.0..."
cargo run -q --bin creg -- status npm:good-pkg@1.0.0
Write-Host "`nChecking good-pkg@2.0.0..."
cargo run -q --bin creg -- status npm:good-pkg@2.0.0
Write-Host "`nChecking evil-pkg@1.0.0..."
cargo run -q --bin creg -- status npm:evil-pkg@1.0.0

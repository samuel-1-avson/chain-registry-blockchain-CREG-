// crates/cli/src/doctor.rs
// `creg doctor` — checks all system prerequisites and reports their status.

use anyhow::Result;
use colored::Colorize;
use std::time::{Duration, Instant};

pub async fn run(node_url: Option<&str>) -> Result<()> {
    let node = node_url.map(String::from).unwrap_or_else(|| {
        std::env::var("CREG_NODE_URL").unwrap_or_else(|_| "http://localhost:8080".into())
    });

    let ipfs = std::env::var("CREG_IPFS_URL").unwrap_or_else(|_| "http://127.0.0.1:5001".into());

    println!("{}", "creg doctor — system health check".bold());
    println!("{}", "─".repeat(52).dimmed());

    let mut all_ok = true;

    // 1. Node connectivity
    print_check("Chain node", "");
    let (node_ok, node_msg) = check_node(&node).await;
    print_result(node_ok, &node_msg);
    all_ok &= node_ok;

    // 2. Node sync status
    print_check("Chain sync", "");
    let (sync_ok, sync_msg) = check_chain_sync(&node).await;
    print_result(sync_ok, &sync_msg);
    // not blocking — just informational
    let _ = sync_ok;

    // 3. IPFS daemon
    print_check("IPFS daemon", "");
    let (ipfs_ok, ipfs_msg) = check_ipfs(&ipfs).await;
    print_result(ipfs_ok, &ipfs_msg);
    all_ok &= ipfs_ok;

    // 4. Publisher key
    print_check("Publisher key", "");
    let (key_ok, key_msg) = check_publisher_key();
    print_result(key_ok, &key_msg);

    // 5. nsjail (optional)
    print_check("nsjail sandbox", "(optional)");
    let (nsjail_ok, nsjail_msg) = check_nsjail();
    print_result(nsjail_ok, &nsjail_msg);

    // 6. gpg (optional)
    print_check("GnuPG", "(optional)");
    let (gpg_ok, gpg_msg) = check_gpg();
    print_result(gpg_ok, &gpg_msg);

    // 7. creg config file
    print_check("Config file", "");
    let (cfg_ok, cfg_msg) = check_config_file();
    print_result(cfg_ok, &cfg_msg);

    // 8. Dev sandbox override
    print_check("Dev sandbox bypass", "");
    let dev_sandbox = std::env::var("CREG_DEV_SANDBOX").as_deref() == Ok("true");
    if dev_sandbox {
        print_result(false, "CREG_DEV_SANDBOX=true — nsjail bypassed (dev only!)");
    } else {
        print_result(true, "not set (production mode)");
    }

    println!("{}", "─".repeat(52).dimmed());
    if all_ok {
        println!("{} All checks passed.", "✓".green().bold());
    } else {
        println!(
            "{} Some checks failed. See above for details.",
            "⚠".yellow().bold()
        );
        std::process::exit(1);
    }

    Ok(())
}

fn print_check(label: &str, note: &str) {
    if note.is_empty() {
        print!("  {:<24} ", label);
    } else {
        print!("  {:<24} {} ", label, note.dimmed());
    }
}

fn print_result(ok: bool, msg: &str) {
    if ok {
        println!("{} {}", "✓".green(), msg);
    } else {
        println!("{} {}", "✗".red(), msg.red());
    }
}

async fn check_node(node: &str) -> (bool, String) {
    let url = format!("{}/v1/health", node.trim_end_matches('/'));
    let start = Instant::now();

    match reqwest::Client::new()
        .get(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            let latency = start.elapsed().as_millis();
            (true, format!("{} — {}ms", node, latency))
        }
        Ok(r) => (false, format!("{} returned HTTP {}", node, r.status())),
        Err(e) => (false, format!("Cannot reach {} — {}", node, e)),
    }
}

async fn check_chain_sync(node: &str) -> (bool, String) {
    let url = format!("{}/v1/chain/stats", node.trim_end_matches('/'));
    match reqwest::Client::new()
        .get(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await {
            Ok(v) => {
                let height = v.get("tip_height").and_then(|h| h.as_u64()).unwrap_or(0);
                let pkgs = v.get("package_count").and_then(|p| p.as_u64()).unwrap_or(0);
                (true, format!("height={} packages={}", height, pkgs))
            }
            Err(_) => (false, "Could not parse chain stats response".into()),
        },
        _ => (false, "Could not reach chain stats endpoint".into()),
    }
}

async fn check_ipfs(ipfs: &str) -> (bool, String) {
    let url = format!("{}/api/v0/id", ipfs.trim_end_matches('/'));
    match reqwest::Client::new()
        .post(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await {
            Ok(v) => {
                let id = v.get("ID").and_then(|i| i.as_str()).unwrap_or("unknown");
                (true, format!("{} — peer {}", ipfs, &id[..id.len().min(12)]))
            }
            Err(_) => (true, format!("{} — reachable", ipfs)),
        },
        Ok(r) => (false, format!("{} returned HTTP {}", ipfs, r.status())),
        Err(e) => (
            false,
            format!(
                "IPFS daemon not running at {} — start with 'ipfs daemon'. Error: {}",
                ipfs, e
            ),
        ),
    }
}

fn check_publisher_key() -> (bool, String) {
    // Check env var first, then config file default location.
    if let Ok(path) = std::env::var("CREG_PUBLISHER_KEY") {
        let p = std::path::Path::new(&path);
        if p.exists() {
            return (true, format!("found at {}", path));
        }
        return (
            false,
            format!("CREG_PUBLISHER_KEY set but file not found: {}", path),
        );
    }

    // Config default: ~/.creg/publisher.key
    let default = dirs::home_dir()
        .unwrap_or_default()
        .join(".creg")
        .join("publisher.key");

    if default.exists() {
        return (true, format!("found at {}", default.display()));
    }

    (false, "No publisher key found. Run: creg keygen".into())
}

fn check_nsjail() -> (bool, String) {
    match which::which("nsjail") {
        Ok(p) => (true, format!("found at {}", p.display())),
        Err(_) => (
            false,
            "nsjail not in PATH — sandbox will use WASM fallback".into(),
        ),
    }
}

fn check_gpg() -> (bool, String) {
    match which::which("gpg") {
        Ok(p) => {
            // Get gpg version
            let ver = std::process::Command::new("gpg")
                .arg("--version")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| s.lines().next().map(String::from))
                .unwrap_or_else(|| "unknown version".into());
            (true, format!("{} — {}", p.display(), ver.trim()))
        }
        Err(_) => (false, "gpg not in PATH — PGP signing unavailable".into()),
    }
}

fn check_config_file() -> (bool, String) {
    let cfg_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".creg")
        .join("config.toml");

    if cfg_path.exists() {
        (true, format!("found at {}", cfg_path.display()))
    } else {
        (
            false,
            format!(
                "not found at {} — run: creg config init",
                cfg_path.display()
            ),
        )
    }
}

// crates/cli/src/observer.rs
// Observer node pool management, health probing, and HA failover verification

use anyhow::{bail, Result};
use colored::Colorize;
use serde_json::Value;
use std::time::Instant;

/// Probe the Observer Load Balancer and individual observer replicas
pub async fn status(lb_url: &str, replicas: &[String]) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .build()?;

    println!(
        "\n{}",
        "================================================================================".cyan()
    );
    println!(
        "  {}",
        "CREG OBSERVER HIGH-AVAILABILITY CLUSTER STATUS"
            .bold()
            .cyan()
    );
    println!(
        "{}\n",
        "================================================================================".cyan()
    );

    // 1. Probe Load Balancer
    println!("{}", "1. Load Balancer Routing Probe".bold().yellow());
    println!("  Endpoint: {}", lb_url.cyan());

    let start_lb = Instant::now();
    match client.get(format!("{}/lb-status", lb_url)).send().await {
        Ok(resp) if resp.status().is_success() => {
            let elapsed = start_lb.elapsed().as_millis();
            let json: Value = resp.json().await.unwrap_or_default();
            let component = json["component"].as_str().unwrap_or("load-balancer");
            println!(
                "  Status:   {} (component: {}, latency: {}ms)",
                "ONLINE".green().bold(),
                component.dimmed(),
                elapsed.to_string().cyan()
            );
        }
        Ok(resp) => {
            println!(
                "  Status:   {} (HTTP {})",
                "DEGRADED".yellow().bold(),
                resp.status().as_u16()
            );
        }
        Err(e) => {
            println!("  Status:   {} ({})", "UNREACHABLE".red().bold(), e);
        }
    }

    // Probe public API through load balancer
    match client.get(format!("{}/v1/health", lb_url)).send().await {
        Ok(resp) if resp.status().is_success() => {
            let upstream = resp
                .headers()
                .get("x-proxy-upstream")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("active-pool")
                .to_string();
            let body: Value = resp.json().await.unwrap_or_default();
            let node_id = body["node_id"].as_str().unwrap_or("unknown");
            println!(
                "  Public API: {} (routed to node: {}, upstream: {})",
                "200 OK".green().bold(),
                node_id.cyan(),
                upstream.dimmed()
            );
        }
        Ok(resp) => {
            println!("  Public API: {} (HTTP {})", "ERROR".red(), resp.status());
        }
        Err(e) => {
            println!("  Public API: {} ({})", "FAIL".red(), e);
        }
    }

    // 2. Probe Individual Replicas
    println!("\n{}", "2. Observer Replicas Pool".bold().yellow());
    if replicas.is_empty() {
        println!("  No direct replica endpoints specified. Checking default local ports (28181, 28182)...");
    }

    let default_replicas = vec![
        "http://localhost:28181".to_string(),
        "http://localhost:28182".to_string(),
    ];
    let targets = if replicas.is_empty() {
        &default_replicas
    } else {
        replicas
    };

    let mut healthy_count = 0;
    for (idx, replica_url) in targets.iter().enumerate() {
        let name = format!("Replica-{}", idx + 1);
        let start = Instant::now();
        match client
            .get(format!("{}/v1/health", replica_url))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let elapsed = start.elapsed().as_millis();
                let body: Value = resp.json().await.unwrap_or_default();
                let node_id = body["node_id"].as_str().unwrap_or("unknown");
                let is_validator = body["is_validator"].as_bool().unwrap_or(false);
                let role = if is_validator {
                    "Validator (unexpected)".yellow()
                } else {
                    "Observer (Read-Only)".green()
                };
                let sync_healthy = body["sync"]["healthy"].as_bool().unwrap_or(true);
                let lag = body["sync"]["lag_blocks"].as_u64().unwrap_or(0);

                println!(
                    "  {} {} [{}]:",
                    "●".green(),
                    name.bold(),
                    replica_url.dimmed()
                );
                println!("      Node ID:      {}", node_id.cyan());
                println!("      Role:         {}", role);
                println!(
                    "      Sync:         {} (lag: {} blocks)",
                    if sync_healthy {
                        "HEALTHY".green()
                    } else {
                        "LAGGING".yellow()
                    },
                    lag.to_string().cyan()
                );
                println!("      Latency:      {}ms", elapsed.to_string().cyan());
                healthy_count += 1;
            }
            Ok(resp) => {
                println!(
                    "  {} {} [{}]: HTTP {}",
                    "●".yellow(),
                    name.bold(),
                    replica_url.dimmed(),
                    resp.status()
                );
            }
            Err(e) => {
                println!(
                    "  {} {} [{}]: {}",
                    "●".red(),
                    name.bold(),
                    replica_url.dimmed(),
                    e
                );
            }
        }
    }

    println!("\n{}", "Summary:".bold());
    println!(
        "  Active Pool Health: {}/{} replicas operational",
        healthy_count.to_string().green().bold(),
        targets.len()
    );

    if healthy_count == targets.len() {
        println!(
            "  HA Status:          {}",
            "FULLY REDUNDANT (Zero-Downtime Ready)".green().bold()
        );
    } else if healthy_count > 0 {
        println!(
            "  HA Status:          {}",
            "DEGRADED (Single Point of Failure)".yellow().bold()
        );
    } else {
        println!(
            "  HA Status:          {}",
            "OUTAGE (All Observers Down)".red().bold()
        );
    }

    println!();
    Ok(())
}

/// Run an in-flight probe drill against the load balancer to test continuous routing
pub async fn probe_drill(lb_url: &str, count: usize) -> Result<()> {
    let requests = count.max(1);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()?;

    println!(
        "\n{}",
        "================================================================================".cyan()
    );
    println!(
        "  {}",
        format!("OBSERVER LOAD BALANCER BURST PROBE ({} REQUESTS)", requests)
            .bold()
            .cyan()
    );
    println!(
        "{}\n",
        "================================================================================".cyan()
    );

    println!("Target: {}", lb_url.cyan());
    println!("Dispatching queries...\n");

    let mut successes = 0;
    let mut failures = 0;
    let mut latencies = Vec::new();
    let mut node_hits = std::collections::HashMap::new();

    for i in 1..=requests {
        let start = Instant::now();
        match client.get(format!("{}/v1/health", lb_url)).send().await {
            Ok(resp) if resp.status().is_success() => {
                let elapsed = start.elapsed().as_millis();
                latencies.push(elapsed);
                successes += 1;
                let body: Value = resp.json().await.unwrap_or_default();
                let node_id = body["node_id"].as_str().unwrap_or("unknown").to_string();
                *node_hits.entry(node_id).or_insert(0) += 1;
                print!(".");
            }
            _ => {
                failures += 1;
                print!("x");
            }
        }
        if i % 50 == 0 {
            println!();
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    println!("\n");

    let total = successes + failures;
    let success_rate = if total > 0 {
        (successes as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    println!("{}", "Results:".bold());
    println!("  Total Queries:      {}", total);
    println!("  Successes (200 OK): {}", successes.to_string().green());
    println!("  Failures:           {}", failures.to_string().red());
    println!("  Survival Rate:      {:.2}%", success_rate);

    if !latencies.is_empty() {
        let sum: u128 = latencies.iter().sum();
        let avg = sum / latencies.len() as u128;
        let min = latencies.iter().min().unwrap();
        let max = latencies.iter().max().unwrap();
        println!(
            "  Latency:            min: {}ms | avg: {}ms | max: {}ms",
            min.to_string().cyan(),
            avg.to_string().cyan(),
            max.to_string().cyan()
        );
    }

    println!("\n{}", "Traffic Distribution across Replicas:".bold());
    for (node, count) in &node_hits {
        let pct = (*count as f64 / successes as f64) * 100.0;
        println!("  - {}: {} hits ({:.1}%)", node.cyan(), count, pct);
    }

    if success_rate >= 99.9 {
        println!(
            "\n{} High-Availability routing confirmed uninterrupted!",
            "✓".green().bold()
        );
    } else {
        bail!(
            "Probe drill detected dropped requests (survival rate: {:.2}%)",
            success_rate
        );
    }

    Ok(())
}

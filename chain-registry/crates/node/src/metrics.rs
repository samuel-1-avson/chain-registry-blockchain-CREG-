// crates/node/src/metrics.rs
// Lightweight Prometheus-compatible metrics endpoint.
// Exposed at GET /metrics — scrape with any Prometheus-compatible system.
//
// Metrics exposed:
//   creg_chain_height            — current chain tip height (gauge)
//   creg_package_count           — total verified packages (gauge)
//   creg_pending_pool_size       — packages awaiting consensus (gauge)
//   creg_publisher_count         — unique publishers seen (gauge)
//   creg_blocks_produced_total   — blocks produced by this node (counter)
//   creg_votes_cast_total        — PBFT votes cast by this node (counter)
//   creg_packages_verified_total — packages this node helped verify (counter)
//   creg_packages_rejected_total — packages this node rejected (counter)

use std::sync::Arc;
use tokio::sync::RwLock;
use crate::NodeState;

/// Build a Prometheus text-format metrics response.
pub async fn render(state: Arc<RwLock<NodeState>>) -> String {
    let s = state.read().await;
    let stats = s.chain.stats();

    let mut out = String::with_capacity(1024);

    metric(&mut out, "creg_chain_height",
        "Current chain tip height", "gauge",
        stats.tip_height as f64);

    metric(&mut out, "creg_package_count",
        "Total verified packages on chain", "gauge",
        stats.package_count as f64);

    metric(&mut out, "creg_pending_pool_size",
        "Packages currently awaiting consensus", "gauge",
        s.pending_pool.len() as f64);

    metric(&mut out, "creg_block_count",
        "Total blocks in the chain", "gauge",
        stats.block_count as f64);

    metric(&mut out, "creg_publisher_count",
        "Unique publishers tracked", "gauge",
        s.publisher_index.publisher_count() as f64);

    // Node identity label.
    let node_id = &s.config.node_id;
    labeled_metric(&mut out, "creg_node_info",
        "Static node information", "gauge",
        &[("node_id", node_id.as_str()), ("version", env!("CARGO_PKG_VERSION"))],
        1.0);

    out
}

fn metric(buf: &mut String, name: &str, help: &str, kind: &str, value: f64) {
    buf.push_str(&format!("# HELP {} {}\n", name, help));
    buf.push_str(&format!("# TYPE {} {}\n", name, kind));
    buf.push_str(&format!("{} {}\n\n", name, value));
}

fn labeled_metric(
    buf: &mut String,
    name: &str,
    help: &str,
    kind: &str,
    labels: &[(&str, &str)],
    value: f64,
) {
    buf.push_str(&format!("# HELP {} {}\n", name, help));
    buf.push_str(&format!("# TYPE {} {}\n", name, kind));
    let label_str = labels.iter()
        .map(|(k, v)| format!("{}=\"{}\"", k, v))
        .collect::<Vec<_>>()
        .join(",");
    buf.push_str(&format!("{}{{{}}} {}\n\n", name, label_str, value));
}

// crates/cli/src/watch.rs
// `creg watch` — streams live registry events from the SSE endpoint
// and renders them as formatted terminal output.
//
// Usage:
//   creg watch                      — all events
//   creg watch --filter verified    — only verified packages
//   creg watch --filter pkg:express — only events for a specific package

use anyhow::Result;
use colored::Colorize;

pub async fn run(filter: Option<&str>, node_url: Option<&str>, ci_mode: bool) -> Result<()> {
    let url = format!(
        "{}/v1/public/events",
        node_url
            .map(String::from)
            .unwrap_or_else(|| {
                std::env::var("CREG_NODE_URL").unwrap_or_else(|_| "http://localhost:8080".into())
            })
            .trim_end_matches('/')
    );

    if ci_mode {
        println!(
            "{} CI mode — watching for Critical security events (exits 1 on Critical)",
            "⚠".yellow().bold()
        );
    }
    println!(
        "{} Connecting to event stream at {}",
        "→".cyan(),
        url.dimmed()
    );
    if let Some(f) = filter {
        println!("{} Filter: {}", "→".cyan(), f.yellow());
    }
    println!("{}", "─".repeat(60).dimmed());

    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()?;

    let mut retry_count = 0u32;
    let max_retries = 10u32;

    'reconnect: loop {
        let mut response = match client
            .get(&url)
            .header("Accept", "text/event-stream")
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => {
                retry_count = 0;
                r
            }
            Ok(r) => {
                eprintln!("{} Server returned {}", "✗".red(), r.status());
                if retry_count >= max_retries {
                    break;
                }
                let delay = std::cmp::min(1u64 << retry_count, 30);
                retry_count += 1;
                eprintln!(
                    "{} Reconnecting in {}s... ({}/{})",
                    "↻".yellow(),
                    delay,
                    retry_count,
                    max_retries
                );
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                continue 'reconnect;
            }
            Err(e) => {
                eprintln!("{} Connection failed: {}", "✗".red(), e);
                if retry_count >= max_retries {
                    break;
                }
                let delay = std::cmp::min(1u64 << retry_count, 30);
                retry_count += 1;
                eprintln!(
                    "{} Reconnecting in {}s... ({}/{})",
                    "↻".yellow(),
                    delay,
                    retry_count,
                    max_retries
                );
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                continue 'reconnect;
            }
        };

        let mut buffer = Vec::new();

        loop {
            // Read chunks from the SSE stream.
            let chunk = match response.chunk().await {
                Ok(Some(c)) => c,
                Ok(None) => {
                    eprintln!("{} Stream closed by server.", "⚠".yellow());
                    if retry_count >= max_retries {
                        break 'reconnect;
                    }
                    let delay = std::cmp::min(1u64 << retry_count, 30);
                    retry_count += 1;
                    eprintln!(
                        "{} Reconnecting in {}s... ({}/{})",
                        "↻".yellow(),
                        delay,
                        retry_count,
                        max_retries
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                    continue 'reconnect;
                }
                Err(e) => {
                    eprintln!("{} Stream error: {}", "✗".red(), e);
                    if retry_count >= max_retries {
                        break 'reconnect;
                    }
                    let delay = std::cmp::min(1u64 << retry_count, 30);
                    retry_count += 1;
                    eprintln!(
                        "{} Reconnecting in {}s... ({}/{})",
                        "↻".yellow(),
                        delay,
                        retry_count,
                        max_retries
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                    continue 'reconnect;
                }
            };

            buffer.extend_from_slice(&chunk);

            // Keep partial UTF-8 bytes and support standard SSE line endings.
            while let Some((end, separator_len)) = sse_frame_end(&buffer) {
                let frame: Vec<u8> = buffer.drain(..end + separator_len).collect();
                let Ok(message) = std::str::from_utf8(&frame[..end]) else {
                    continue;
                };

                if let Some(event) = parse_sse(message) {
                    if should_display(&event, filter) {
                        render_event(&event);
                        if ci_mode && is_critical_event(&event) {
                            eprintln!(
                                "{} Critical security event detected — exiting with code 1 (CI mode)",
                                "✗".red().bold()
                            );
                            std::process::exit(1);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn sse_frame_end(buffer: &[u8]) -> Option<(usize, usize)> {
    [b"\n\n".as_slice(), b"\r\n\r\n".as_slice(), b"\r\r".as_slice()]
        .into_iter()
        .filter_map(|separator| {
            buffer
                .windows(separator.len())
                .position(|window| window == separator)
                .map(|end| (end, separator.len()))
        })
        .min_by_key(|(end, _)| *end)
}

#[derive(Debug)]
struct SseEvent {
    kind: String,
    data: serde_json::Value,
}

fn parse_sse(raw: &str) -> Option<SseEvent> {
    let mut event_name = String::new();
    let mut data = String::new();

    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            event_name = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        } else if line.starts_with(':') {
            // SSE comment / heartbeat — skip.
        }
    }

    if data.is_empty() {
        return None;
    }

    let envelope: serde_json::Value = serde_json::from_str(&data).ok()?;
    let kind = envelope
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .filter(|kind| !kind.is_empty())
        .unwrap_or(&event_name)
        .to_ascii_lowercase();
    if kind.is_empty() {
        return None;
    }

    // The node sends { kind, ts, payload }, while the terminal renderer and
    // package filter consume the event fields directly. Flatten the envelope
    // and keep its timestamp so named events reach the same presentation path.
    let event_data = match envelope.get("payload") {
        Some(serde_json::Value::Object(payload)) => {
            let mut flattened = payload.clone();
            if !flattened.contains_key("ts") {
                if let Some(timestamp) = envelope.get("ts") {
                    flattened.insert("ts".to_string(), timestamp.clone());
                }
            }
            serde_json::Value::Object(flattened)
        }
        _ => envelope,
    };

    Some(SseEvent {
        kind,
        data: event_data,
    })
}

fn is_critical_event(event: &SseEvent) -> bool {
    // In CI mode, any rejected or revoked package event is critical.
    matches!(event.kind.as_str(), "package_rejected" | "package_revoked")
}

fn should_display(event: &SseEvent, filter: Option<&str>) -> bool {
    let Some(f) = filter else { return true };
    let canonical = event
        .data
        .get("canonical")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if f.starts_with("pkg:") {
        return canonical.contains(&f[4..]);
    }
    match f.to_ascii_lowercase().as_str() {
        "" | "all" => return true,
        "packages" => return event.kind.starts_with("package_"),
        "blocks" => return event.kind == "block_produced",
        "votes" => return event.kind == "validator_voted",
        _ => {}
    }
    // Filter by event kind.
    event.kind.to_lowercase().contains(&f.to_lowercase())
}

fn render_event(event: &SseEvent) {
    let ts = event
        .data
        .get("ts")
        .and_then(|v| v.as_str())
        .and_then(|s| s.get(11..19)) // HH:MM:SS
        .unwrap_or("?");

    let canonical = event
        .data
        .get("canonical")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    match event.kind.as_str() {
        "package_verified" => {
            let block = event
                .data
                .get("block_hash")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let validators = event
                .data
                .get("validator_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            println!(
                "{} {} {} {} {} {} {}",
                ts.dimmed(),
                "✓ VERIFIED ".green().bold(),
                canonical.white().bold(),
                "block".dimmed(),
                block.dimmed(),
                "validators".dimmed(),
                validators.to_string().dimmed(),
            );
        }

        "package_submitted" => {
            let publisher = event
                .data
                .get("publisher_pubkey")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!(
                "{} {} {} {}",
                ts.dimmed(),
                "↑ SUBMITTED".cyan(),
                canonical.white(),
                format!("by {}...", publisher).dimmed(),
            );
        }

        "package_rejected" => {
            let reason = event
                .data
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown reason");
            println!(
                "{} {} {} {}",
                ts.dimmed(),
                "✗ REJECTED ".red().bold(),
                canonical.white(),
                format!("({})", reason).red().dimmed(),
            );
        }

        "package_revoked" => {
            let reason = event
                .data
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!(
                "{} {} {} {}",
                ts.dimmed(),
                "⊘ REVOKED  ".red().bold(),
                canonical.white().bold(),
                format!("— {}", reason).red(),
            );
        }

        "block_produced" => {
            let height = event
                .data
                .get("height")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let hash = event
                .data
                .get("hash")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let tx_count = event
                .data
                .get("tx_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            println!(
                "{} {} {} {} {}",
                ts.dimmed(),
                "▣ BLOCK    ".purple(),
                format!("#{}", height).white(),
                hash.dimmed(),
                format!("({} tx)", tx_count).dimmed(),
            );
        }

        "validator_voted" => {
            let vid = event
                .data
                .get("validator_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let approved = event
                .data
                .get("approved")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let icon = if approved { "✓".green() } else { "✗".red() };
            println!(
                "{} {} {} {} {}",
                ts.dimmed(),
                "◈ VOTE     ".yellow(),
                icon,
                vid.dimmed(),
                format!("on {}", canonical).dimmed(),
            );
        }

        other => {
            println!("{} {} {:?}", ts.dimmed(), other.dimmed(), event.data);
        }
    }
}

// crates/cli/src/watch.rs
// `creg watch` — streams live registry events from SSE v2 (`/v1/public/v4/events`,
// falling back to `/v1/public/events` on 404) with `Last-Event-ID` resume and renders
// them with `DisplayText` sanitization (`ADR-019` §4.2, `ECA-18`, `ECA-17`).

use anyhow::Result;
use colored::Colorize;
use common::DisplayText;
use registry_view::SseEnvelopeV2;

/// ECA-18 / eco-adv-08: the most bytes of one not-yet-terminated SSE frame we
/// hold. A stream that exceeds it without a frame separator is dropped.
pub const MAX_PENDING_SSE_BYTES: usize = 256 * 1024;

/// Longest rendering of one untrusted field (ECA-17).
const MAX_FIELD_CHARS: usize = 160;

#[derive(Debug, PartialEq, Eq)]
struct FrameOverflow {
    pending: usize,
}

/// Append `chunk` to `buffer` and return every complete frame (without its
/// separator). Frames that are not UTF-8 are skipped. Errors when the
/// remaining partial frame exceeds `MAX_PENDING_SSE_BYTES`.
fn push_chunk(buffer: &mut Vec<u8>, chunk: &[u8]) -> Result<Vec<String>, FrameOverflow> {
    buffer.extend_from_slice(chunk);
    let mut frames = Vec::new();
    // Keep partial UTF-8 bytes and support standard SSE line endings.
    while let Some((end, separator_len)) = sse_frame_end(buffer) {
        let frame: Vec<u8> = buffer.drain(..end + separator_len).collect();
        if let Ok(message) = std::str::from_utf8(&frame[..end]) {
            frames.push(message.to_string());
        }
    }
    if buffer.len() > MAX_PENDING_SSE_BYTES {
        let pending = buffer.len();
        buffer.clear();
        return Err(FrameOverflow { pending });
    }
    Ok(frames)
}

pub async fn run(filter: Option<&str>, node_url: Option<&str>, ci_mode: bool) -> Result<()> {
    let base = node_url
        .map(String::from)
        .unwrap_or_else(|| {
            std::env::var("CREG_NODE_URL").unwrap_or_else(|_| "http://localhost:8080".into())
        })
        .trim_end_matches('/')
        .to_string();
    let v4_url = format!("{base}/v1/public/v4/events");
    let legacy_url = format!("{base}/v1/public/events");
    let mut active_url = v4_url.clone();

    if ci_mode {
        println!(
            "{} CI mode — watching for Critical security events (exits 1 on Critical)",
            "⚠".yellow().bold()
        );
    }
    println!(
        "{} Connecting to event stream at {}",
        "→".cyan(),
        active_url.dimmed()
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
    let mut last_event_id: Option<String> = None;

    'reconnect: loop {
        let mut req = client
            .get(&active_url)
            .header("Accept", "text/event-stream");
        if let Some(ref event_id) = last_event_id {
            req = req.header("Last-Event-ID", event_id);
        }

        let mut response = match req.send().await {
            Ok(r) if r.status().is_success() => {
                retry_count = 0;
                r
            }
            Ok(r) if r.status() == reqwest::StatusCode::NOT_FOUND && active_url == v4_url => {
                active_url = legacy_url.clone();
                eprintln!(
                    "{} SSE v2 endpoint returned 404; falling back to legacy stream {}",
                    "↻".yellow(),
                    active_url.dimmed()
                );
                continue 'reconnect;
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

            let frames = match push_chunk(&mut buffer, &chunk) {
                Ok(frames) => frames,
                Err(FrameOverflow { pending }) => {
                    eprintln!(
                        "{} Dropping stream: {} bytes without an SSE frame separator (cap {} bytes).",
                        "✗".red(),
                        pending,
                        MAX_PENDING_SSE_BYTES
                    );
                    if retry_count >= max_retries {
                        break 'reconnect;
                    }
                    let delay = std::cmp::min(1u64 << retry_count, 30);
                    retry_count += 1;
                    tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                    continue 'reconnect;
                }
            };

            for message in &frames {
                if let Some(event) = parse_sse(message) {
                    if let Some(ref eid) = event.id {
                        last_event_id = Some(eid.clone());
                    }
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
    [
        b"\n\n".as_slice(),
        b"\r\n\r\n".as_slice(),
        b"\r\r".as_slice(),
    ]
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
    id: Option<String>,
    kind: String,
    data: serde_json::Value,
}

fn parse_sse(raw: &str) -> Option<SseEvent> {
    let mut event_id: Option<String> = None;
    let mut event_name = String::new();
    let mut data = String::new();

    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("id:") {
            let trimmed = rest.trim();
            if !trimmed.is_empty() {
                event_id = Some(trimmed.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("event:") {
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

    // Prefer typed SSE v2 envelopes when the payload decodes as SseEnvelopeV2.
    if let Ok(v2) = serde_json::from_str::<SseEnvelopeV2>(&data) {
        let mut merged = match v2.data {
            serde_json::Value::Object(map) => map,
            other => {
                let mut map = serde_json::Map::new();
                map.insert("value".to_string(), other);
                map
            }
        };
        merged.insert("height".to_string(), serde_json::json!(v2.height));
        merged.insert("tx_index".to_string(), serde_json::json!(v2.tx_index));
        if let Some(canonical) = v2.canonical {
            merged.insert(
                "canonical".to_string(),
                serde_json::Value::String(canonical.into_string()),
            );
        }
        return Some(SseEvent {
            id: event_id.or(Some(v2.id)),
            kind: v2.kind.as_str().to_string(),
            data: serde_json::Value::Object(merged),
        });
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

    let id = event_id.or_else(|| {
        envelope
            .get("id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    });

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
        id,
        kind,
        data: event_data,
    })
}

fn is_critical_event(event: &SseEvent) -> bool {
    // In CI mode, any rejection, revocation (v1 or v2), quarantine, quarantine
    // expiry, or committed/verified sandbox witness is critical (ECA-18).
    let kind = event.kind.as_str();
    kind.starts_with("package_revoked")
        || matches!(
            kind,
            "package_rejected"
                | "package_quarantined"
                | "quarantine_expired"
                | "witness_verified"
                | "witness_committed"
        )
}

/// String field from the event payload, sanitised for the terminal.
fn text_field(event: &SseEvent, key: &str, default: &str) -> String {
    let raw = event
        .data
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or(default);
    DisplayText::bounded(raw, MAX_FIELD_CHARS).into_string()
}

fn should_display(event: &SseEvent, filter: Option<&str>) -> bool {
    let Some(f) = filter else { return true };
    let canonical = event
        .data
        .get("canonical")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if let Some(pkg_query) = f.strip_prefix("pkg:") {
        return canonical.contains(pkg_query);
    }
    match f.to_ascii_lowercase().as_str() {
        "" | "all" => return true,
        "packages" => {
            return event.kind.starts_with("package_") || event.kind.starts_with("quarantine_")
        }
        "blocks" => return matches!(event.kind.as_str(), "block_produced" | "block_committed"),
        "votes" => return event.kind == "validator_voted",
        _ => {}
    }
    // Filter by event kind.
    event.kind.to_lowercase().contains(&f.to_lowercase())
}

fn render_event(event: &SseEvent) {
    let height_badge = event
        .data
        .get("height")
        .and_then(|v| v.as_u64())
        .map(|h| format!("#{h}"));
    let ts_raw = event
        .data
        .get("ts")
        .and_then(|v| v.as_str())
        .and_then(|s| s.get(11..19));
    let prefix =
        DisplayText::bounded(ts_raw.or(height_badge.as_deref()).unwrap_or("?"), 16).into_string();
    let ts = prefix.as_str();

    let canonical = text_field(event, "canonical", "");
    let canonical = canonical.as_str();

    match event.kind.as_str() {
        "package_verified" => {
            let block = text_field(event, "block_hash", "?");
            let block = block.as_str();
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
            let publisher = text_field(event, "publisher_pubkey", "?");
            println!(
                "{} {} {} {}",
                ts.dimmed(),
                "↑ SUBMITTED".cyan(),
                canonical.white(),
                format!("by {}...", publisher).dimmed(),
            );
        }

        "package_rejected" => {
            let reason = text_field(event, "reason", "unknown reason");
            println!(
                "{} {} {} {}",
                ts.dimmed(),
                "✗ REJECTED ".red().bold(),
                canonical.white(),
                format!("({})", reason).red().dimmed(),
            );
        }

        kind if kind.starts_with("package_revoked") => {
            let reason = text_field(event, "reason", "?");
            println!(
                "{} {} {} {}",
                ts.dimmed(),
                "⊘ REVOKED  ".red().bold(),
                canonical.white().bold(),
                format!("— {}", reason).red(),
            );
        }

        "package_quarantined" | "quarantine_expired" => {
            let reason = text_field(event, "reason", "committee hold");
            let label = if event.kind == "quarantine_expired" {
                "⊘ EXPIRED→REVOKED".red().bold()
            } else {
                "⧗ QUARANTINE".magenta().bold()
            };
            println!(
                "{} {} {} {}",
                ts.dimmed(),
                label,
                canonical.white().bold(),
                format!("— {}", reason).magenta(),
            );
        }

        "quarantine_resolved" => {
            let outcome = text_field(event, "outcome", "resolved");
            println!(
                "{} {} {} {}",
                ts.dimmed(),
                "✓ RESOLVED ".green(),
                canonical.white().bold(),
                format!("({outcome})").dimmed(),
            );
        }

        "witness_committed" | "witness_verified" => {
            let digest = text_field(event, "witness_digest", "?");
            println!(
                "{} {} {} {}",
                ts.dimmed(),
                "⚡ WITNESS  ".red().bold(),
                canonical.white().bold(),
                format!("digest={digest}").dimmed(),
            );
        }

        "bounty_awarded" => {
            let amount = event
                .data
                .get("amount")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            println!(
                "{} {} {} {}",
                ts.dimmed(),
                "★ BOUNTY   ".yellow(),
                canonical.white(),
                format!("amount={amount}").dimmed(),
            );
        }

        "block_produced" | "block_committed" => {
            let height = event
                .data
                .get("height")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let hash = text_field(event, "hash", "?");
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
            let vid = text_field(event, "validator_id", "?");
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
            let kind = DisplayText::bounded(other, 48);
            let data = DisplayText::bounded(&format!("{:?}", event.data), 512);
            println!("{} {} {}", ts.dimmed(), kind.to_string().dimmed(), data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use registry_view::SseEventKindV2;

    #[test]
    fn eco_adv_08_unterminated_stream_is_dropped_at_cap() {
        // 10 MiB with no frame separator, delivered in 64 KiB chunks.
        let chunk = vec![b'a'; 64 * 1024];
        let mut buffer = Vec::new();
        let mut delivered = 0usize;
        let mut overflow = None;
        while delivered < 10 * 1024 * 1024 {
            delivered += chunk.len();
            if let Err(err) = push_chunk(&mut buffer, &chunk) {
                overflow = Some(err);
                break;
            }
        }
        let overflow = overflow.expect("stream must be dropped");
        assert!(overflow.pending > MAX_PENDING_SSE_BYTES);
        assert!(delivered <= MAX_PENDING_SSE_BYTES + chunk.len());
        assert!(buffer.is_empty(), "buffer must be released on overflow");
    }

    #[test]
    fn complete_frames_are_split_and_partial_tail_kept() {
        let mut buffer = Vec::new();
        let frames = push_chunk(
            &mut buffer,
            b"data: {\"kind\":\"a\"}\n\ndata: {\"kind\":\"b\"}\r\n\r\ndata: {\"ki",
        )
        .unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(buffer, b"data: {\"ki");
        let frames = push_chunk(&mut buffer, b"nd\":\"c\"}\n\n").unwrap();
        assert_eq!(frames, vec!["data: {\"kind\":\"c\"}".to_string()]);
        assert!(buffer.is_empty());
    }

    fn event(kind: &str) -> SseEvent {
        SseEvent {
            id: None,
            kind: kind.into(),
            data: serde_json::json!({}),
        }
    }

    #[test]
    fn ci_mode_fails_on_witness_committed_and_v4_kinds() {
        for kind in [
            "package_rejected",
            "package_revoked",
            "package_revoked_v2",
            "package_quarantined",
            "quarantine_expired",
            "witness_verified",
            "witness_committed",
        ] {
            assert!(is_critical_event(&event(kind)), "{kind}");
        }
        for kind in [
            "package_verified",
            "quarantine_resolved",
            "block_produced",
            "block_committed",
            "bounty_awarded",
            "validator_voted",
        ] {
            assert!(!is_critical_event(&event(kind)), "{kind}");
        }
    }

    #[test]
    fn parse_sse_decodes_v2_envelope_and_last_event_id() {
        let envelope = SseEnvelopeV2::new(
            SseEventKindV2::WitnessCommitted,
            48_211,
            2,
            Some("npm:bad@1.0.0"),
            serde_json::json!({ "witness_digest": "abcd" }),
        );
        let frame = envelope.to_sse_frame();
        let parsed = parse_sse(frame.trim_end_matches('\n')).expect("sse v2 frame");
        assert_eq!(parsed.id.as_deref(), Some("48211.2"));
        assert_eq!(parsed.kind, "witness_committed");
        assert_eq!(
            parsed.data.get("canonical").and_then(|v| v.as_str()),
            Some("npm:bad@1.0.0")
        );
        assert_eq!(
            parsed.data.get("height").and_then(|v| v.as_u64()),
            Some(48_211)
        );
        assert!(is_critical_event(&parsed));
    }

    #[test]
    fn eco_adv_07_payload_strings_are_sanitised() {
        // Built with Rust escapes so the source holds no raw control or bidi
        // characters (rustc denies the latter in literals).
        let payload = serde_json::json!({
            "kind": "package_rejected",
            "payload": {
                "canonical": "npm:x\u{1b}]52;c;ZXZpbA==\u{07}@1",
                "reason": "bad\r OK \u{202e}",
            }
        });
        let raw = format!("data: {payload}");
        let event = parse_sse(&raw).expect("event");
        let canonical = text_field(&event, "canonical", "");
        let reason = text_field(&event, "reason", "");
        for text in [&canonical, &reason] {
            assert!(!text.contains('\u{1b}'), "{text:?}");
            assert!(!text.contains('\u{07}'), "{text:?}");
            assert!(!text.contains('\r'), "{text:?}");
            assert!(!text.contains('\u{202e}'), "{text:?}");
        }
        assert_eq!(canonical, "npm:x]52;c;ZXZpbA==@1");
    }
}

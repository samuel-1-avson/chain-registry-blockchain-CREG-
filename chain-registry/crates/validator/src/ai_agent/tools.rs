//! Eight bounded, strictly typed read-only & deterministic-replay tools exposed
//! to Layer 4's `AgentRunner` over `creg-unpack` + `PackageAstIndexV1` +
//! `BinaryInspectionReport` + `KernelBehaviorGraphV1` + WASM static transform +
//! Tier 1/2 replay (`ADR-018`).

use super::envelope::{detect_prompt_injection_patterns, SessionEnvelope};
use super::runner::TriggerWitnessDraftV1;
use crate::kernel_sandbox::{KernelBehaviorGraphV1, KernelViolationCode, TriggerSpecV1};
use crate::sandbox::SandboxResult;
use anyhow::{bail, Result};
use base64::Engine as _;
use common::{PackageId, PackageManifest};
use creg_unpack::{UnpackLimits, UnpackedArchive};
use ml_validator::ast_index::PackageAstIndexV1;
use ml_validator::binary_inspect::BinaryInspectionReport;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::pin::Pin;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use wasm_sandbox_crate::{CapabilitySet, ResourceLimits, SandboxConfig, SandboxInput, WasmSandbox};

pub const MAX_LIST_ENTRIES: usize = 200;
pub const MAX_READ_LINES: usize = 200;
pub const MAX_READ_BYTES: usize = 16 * 1024;
pub const MAX_AST_QUERY_HITS: usize = 100;
pub const MAX_BINARY_REPORT_BYTES: usize = 64 * 1024;
pub const MAX_DEOBF_SLICE_BYTES: usize = 16 * 1024;
pub const MAX_DEOBF_OUTPUT_BYTES: usize = 8 * 1024;
pub const MAX_DIFF_HUNKS: usize = 40;
pub const MAX_REPLAY_TIMEOUT_SECS: u64 = 10;

/// Valid 37-byte WASM module `(func (export "main") (result i32) (i32.const 0))`
/// used to gate every static deobfuscation run under fuel-metered `wasmtime 40`
/// with empty capabilities (`CapabilitySet::empty()`).
const DEOBF_GATE_WASM: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7f, 0x03,
    0x02, 0x01, 0x00, 0x07, 0x08, 0x01, 0x04, b'm', b'a', b'i', b'n', 0x00, 0x00, 0x0a, 0x06, 0x01,
    0x04, 0x00, 0x41, 0x00, 0x0b,
];

static DEOBF_SANDBOX: LazyLock<Option<WasmSandbox>> = LazyLock::new(|| {
    let limits = ResourceLimits::strict()
        .with_memory(1024 * 1024)
        .with_fuel(1_000_000)
        .with_time_ms(500);
    let cfg = SandboxConfig::default()
        .with_resource_limits(limits)
        .with_capabilities(CapabilitySet::empty());
    WasmSandbox::new(cfg).ok()
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelTraceSection {
    Processes,
    Syscalls,
    Canary,
    NetworkSinkhole,
    FsDiff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeobfuscationMode {
    Base64HexXor,
    JsAstFold,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListFilesArgs {
    #[serde(default)]
    pub glob: Option<String>,
    #[serde(default)]
    pub max_entries: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadFileSliceArgs {
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryAstSinksArgs {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectKernelTraceArgs {
    pub section: KernelTraceSection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectBinaryArgs {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeobfuscateArgs {
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub mode: DeobfuscationMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiffPreviousArgs {
    #[serde(default)]
    pub max_hunks: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestReplayArgs {
    pub trigger_spec: TriggerSpecV1,
    #[serde(default)]
    pub timeout_secs: Option<u8>,
}

/// Strict allowlist of the 8 Layer 4 agent tools (`INV-06`). Any unlisted tool
/// name fails serde deserialization and triggers a prompt-injection warning.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "tool", content = "args", rename_all = "snake_case")]
pub enum ToolCallRequest {
    ListFiles(ListFilesArgs),
    ReadFileSlice(ReadFileSliceArgs),
    QueryAstSinks(QueryAstSinksArgs),
    InspectKernelTrace(InspectKernelTraceArgs),
    InspectBinaryElfWasm(InspectBinaryArgs),
    DeobfuscateInWasm(DeobfuscateArgs),
    DiffPreviousVersion(DiffPreviousArgs),
    RequestSandboxReplay(RequestReplayArgs),
}

impl ToolCallRequest {
    pub fn name(&self) -> &'static str {
        match self {
            Self::ListFiles(_) => "list_files",
            Self::ReadFileSlice(_) => "read_file_slice",
            Self::QueryAstSinks(_) => "query_ast_sinks",
            Self::InspectKernelTrace(_) => "inspect_kernel_trace",
            Self::InspectBinaryElfWasm(_) => "inspect_binary_elf_wasm",
            Self::DeobfuscateInWasm(_) => "deobfuscate_in_wasm",
            Self::DiffPreviousVersion(_) => "diff_previous_version",
            Self::RequestSandboxReplay(_) => "request_sandbox_replay",
        }
    }
}

/// Pluggable kernel replay backend so tests can exercise deterministic replay
/// and witness generation without Linux KVM/nsjail binaries.
pub trait SandboxReplayExecutor: Send + Sync {
    fn replay<'a>(
        &'a self,
        pkg_id: &'a PackageId,
        tarball_bytes: &'a [u8],
        manifest: &'a PackageManifest,
        trigger_spec: &'a TriggerSpecV1,
        timeout_secs: u64,
    ) -> Pin<Box<dyn Future<Output = Result<SandboxResult>> + Send + 'a>>;
}

pub struct DefaultSandboxReplayExecutor;

impl SandboxReplayExecutor for DefaultSandboxReplayExecutor {
    fn replay<'a>(
        &'a self,
        pkg_id: &'a PackageId,
        tarball_bytes: &'a [u8],
        manifest: &'a PackageManifest,
        trigger_spec: &'a TriggerSpecV1,
        timeout_secs: u64,
    ) -> Pin<Box<dyn Future<Output = Result<SandboxResult>> + Send + 'a>> {
        Box::pin(async move {
            let capped = timeout_secs.clamp(1, MAX_REPLAY_TIMEOUT_SECS);
            tokio::time::timeout(
                Duration::from_secs(capped.saturating_add(2)),
                crate::sandbox::run_with_trigger(pkg_id, tarball_bytes, manifest, trigger_spec),
            )
            .await
            .map_err(|_| anyhow::anyhow!("sandbox replay timed out after {capped}s"))?
        })
    }
}

#[derive(Debug, Clone)]
pub struct ToolExecutionOutput {
    pub ok: bool,
    pub tool_name: &'static str,
    pub payload_json: String,
    pub injection_hits: Vec<&'static str>,
    pub witness: Option<TriggerWitnessDraftV1>,
}

pub struct ToolContext<'a> {
    pub pkg_id: &'a PackageId,
    pub content_hash: &'a str,
    pub tarball: &'a [u8],
    pub archive: &'a UnpackedArchive,
    pub manifest: &'a PackageManifest,
    pub policy_bundle_id: &'a str,
    pub ast_index: Option<&'a PackageAstIndexV1>,
    pub binary_report: Option<&'a BinaryInspectionReport>,
    pub latest_kernel_graph: Mutex<Option<KernelBehaviorGraphV1>>,
    pub prev_manifest: Option<&'a PackageManifest>,
    pub prev_sandbox: Option<&'a SandboxResult>,
    pub envelope: &'a SessionEnvelope,
    pub max_sandbox_replays: usize,
    pub replays_used: Mutex<usize>,
    pub replay_executor: &'a dyn SandboxReplayExecutor,
}

impl<'a> ToolContext<'a> {
    pub fn replays_used_count(&self) -> usize {
        *self.replays_used.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn latest_graph_snapshot(&self) -> Option<KernelBehaviorGraphV1> {
        self.latest_kernel_graph
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

fn validate_relative_pkg_path(raw: &str) -> Result<String> {
    let norm = raw.trim().replace('\\', "/");
    if norm.is_empty()
        || norm.starts_with('/')
        || norm.contains('\0')
        || norm
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        bail!("invalid relative package path '{}'", raw);
    }
    Ok(norm)
}

fn magic_tag(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x7fELF") {
        "elf"
    } else if bytes.starts_with(b"\x00asm") {
        "wasm"
    } else if bytes.starts_with(b"MZ") {
        "pe"
    } else if matches!(
        bytes.get(0..4),
        Some([0xfe, 0xed, 0xfa, 0xce])
            | Some([0xfe, 0xed, 0xfa, 0xcf])
            | Some([0xcf, 0xfa, 0xed, 0xfe])
            | Some([0xca, 0xfe, 0xba, 0xbe])
    ) {
        "macho"
    } else {
        "data_or_text"
    }
}

fn shannon_entropy(bytes: &[u8]) -> f64 {
    if bytes.is_empty() {
        return 0.0;
    }
    let mut freq = [0u64; 256];
    for &b in bytes {
        freq[b as usize] += 1;
    }
    let n = bytes.len() as f64;
    freq.iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum()
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p = pattern.trim().to_ascii_lowercase();
    if p.is_empty() || p == "*" || p == "**" {
        return true;
    }
    if let Some(suffix) = p.strip_prefix('*') {
        return text.to_ascii_lowercase().ends_with(suffix);
    }
    text.to_ascii_lowercase().contains(&p)
}

fn canonical_manifest_digest(manifest: &PackageManifest) -> String {
    let encoded = serde_json::to_vec(manifest).unwrap_or_default();
    hex::encode(Sha256::digest(&encoded))
}

fn extract_bounded_lines(
    raw: &[u8],
    start_line: u32,
    end_line: u32,
    max_bytes: usize,
) -> (String, bool, usize, usize) {
    let start = start_line.max(1) as usize;
    let max_end = start.saturating_add(MAX_READ_LINES - 1);
    let end = (end_line.max(start_line) as usize).min(max_end);
    let has_invalid_utf8 = std::str::from_utf8(raw).is_err();
    let lossy = String::from_utf8_lossy(raw);
    let mut selected = String::new();
    let mut byte_start = 0usize;
    let mut byte_end = 0usize;
    let mut cur_offset = 0usize;

    for (idx, line) in lossy.split_inclusive('\n').enumerate() {
        let lineno = idx + 1;
        if lineno < start {
            cur_offset += line.len();
            continue;
        }
        if lineno == start {
            byte_start = cur_offset;
        }
        if lineno > end || selected.len() + line.len() > max_bytes {
            break;
        }
        selected.push_str(line);
        cur_offset += line.len();
        byte_end = cur_offset;
    }
    (selected, has_invalid_utf8, byte_start, byte_end)
}

fn decode_base64_candidates(text: &str) -> Vec<String> {
    let mut decoded = Vec::new();
    let mut tok = String::new();
    let flush = |cand: &str, out: &mut Vec<String>| {
        if cand.len() >= 12 {
            if let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(cand) {
                let ascii_ok = raw
                    .iter()
                    .filter(|&&b| b.is_ascii_graphic() || b.is_ascii_whitespace())
                    .count();
                if !raw.is_empty() && ascii_ok * 4 >= raw.len() * 3 {
                    out.push(format!(
                        "b64({}) -> {}",
                        &cand[..cand.len().min(24)],
                        String::from_utf8_lossy(&raw)
                    ));
                }
            }
        }
    };
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '+' | '/' | '=' | '-' | '_') {
            tok.push(ch);
        } else {
            flush(&tok, &mut decoded);
            tok.clear();
        }
    }
    flush(&tok, &mut decoded);
    decoded
}

fn decode_hex_and_charcode(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    // 1. \xNN sequences
    let bytes = text.as_bytes();
    let mut ascii_buf = Vec::new();
    let mut i = 0;
    while i + 3 < bytes.len() {
        if bytes[i] == b'\\' && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X') {
            if let Ok(val) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 2..i + 4]).unwrap_or(""), 16)
            {
                ascii_buf.push(val);
                i += 4;
                continue;
            }
        }
        if ascii_buf.len() >= 4 {
            out.push(format!(
                "hex_esc -> {}",
                String::from_utf8_lossy(&ascii_buf)
            ));
        }
        ascii_buf.clear();
        i += 1;
    }
    if ascii_buf.len() >= 4 {
        out.push(format!(
            "hex_esc -> {}",
            String::from_utf8_lossy(&ascii_buf)
        ));
    }

    // 2. String.fromCharCode(65,66,...)
    if let Some(pos) = text.find("fromCharCode(") {
        let rest = &text[pos + "fromCharCode(".len()..];
        if let Some(end) = rest.find(')') {
            let nums: Vec<u8> = rest[..end]
                .split(',')
                .filter_map(|s| s.trim().parse::<u16>().ok())
                .filter(|&n| n <= 127)
                .map(|n| n as u8)
                .collect();
            if nums.len() >= 3 {
                out.push(format!(
                    "fromCharCode -> {}",
                    String::from_utf8_lossy(&nums)
                ));
            }
        }
    }
    out
}

fn fold_js_concat_literals(text: &str) -> String {
    let mut s = text.to_string();
    for sep in ["' + '", "'+'", "\" + \"", "\"+\""] {
        s = s.replace(sep, "");
    }
    s
}

pub async fn execute_tool(ctx: &ToolContext<'_>, call: ToolCallRequest) -> ToolExecutionOutput {
    let tool_name = call.name();
    match execute_tool_inner(ctx, call).await {
        Ok(out) => out,
        Err(err) => ToolExecutionOutput {
            ok: false,
            tool_name,
            payload_json: serde_json::json!({
                "ok": false,
                "error": err.to_string()
            })
            .to_string(),
            injection_hits: Vec::new(),
            witness: None,
        },
    }
}

async fn execute_tool_inner(
    ctx: &ToolContext<'_>,
    call: ToolCallRequest,
) -> Result<ToolExecutionOutput> {
    match call {
        ToolCallRequest::ListFiles(args) => {
            let cap = usize::from(args.max_entries.unwrap_or(200)).min(MAX_LIST_ENTRIES);
            let glob = args.glob.as_deref().unwrap_or("*");
            let mut rows = Vec::new();
            for f in &ctx.archive.files {
                if !wildcard_match(glob, &f.path) {
                    continue;
                }
                let bytes = f.raw_bytes.as_ref();
                rows.push(serde_json::json!({
                    "path": f.path,
                    "bytes": bytes.len(),
                    "valid_utf8": std::str::from_utf8(bytes).is_ok(),
                    "entropy": ((shannon_entropy(bytes) * 100.0).round() / 100.0),
                    "magic": magic_tag(bytes),
                }));
                if rows.len() >= cap {
                    break;
                }
            }
            Ok(ToolExecutionOutput {
                ok: true,
                tool_name: "list_files",
                payload_json: serde_json::json!({
                    "total_files": ctx.archive.files.len(),
                    "returned": rows.len(),
                    "files": rows,
                })
                .to_string(),
                injection_hits: Vec::new(),
                witness: None,
            })
        }
        ToolCallRequest::ReadFileSlice(args) => {
            let rel = validate_relative_pkg_path(&args.path)?;
            let file = ctx
                .archive
                .files
                .iter()
                .find(|f| f.path == rel)
                .ok_or_else(|| anyhow::anyhow!("file '{}' not found in package archive", rel))?;
            let (slice_text, non_utf8, byte_start, byte_end) = extract_bounded_lines(
                file.raw_bytes.as_ref(),
                args.start_line,
                args.end_line,
                MAX_READ_BYTES,
            );
            let hits = detect_prompt_injection_patterns(&slice_text);
            let wrapped = ctx.envelope.wrap_untrusted(&rel, &slice_text);
            let slice_sha256 = if byte_end <= file.raw_bytes.len() && byte_start <= byte_end {
                hex::encode(Sha256::digest(&file.raw_bytes[byte_start..byte_end]))
            } else {
                hex::encode(Sha256::digest(slice_text.as_bytes()))
            };
            Ok(ToolExecutionOutput {
                ok: true,
                tool_name: "read_file_slice",
                payload_json: serde_json::json!({
                    "path": rel,
                    "byte_start": byte_start,
                    "byte_end": byte_end,
                    "slice_sha256": slice_sha256,
                    "non_utf8_bytes": non_utf8,
                    "prompt_injection_signals": hits,
                    "content": wrapped,
                })
                .to_string(),
                injection_hits: hits,
                witness: None,
            })
        }
        ToolCallRequest::QueryAstSinks(args) => {
            let Some(ast) = ctx.ast_index else {
                return Ok(ToolExecutionOutput {
                    ok: true,
                    tool_name: "query_ast_sinks",
                    payload_json: serde_json::json!({
                        "available": false,
                        "sinks": [],
                        "reachability_edges": [],
                        "obfuscation_signals": []
                    })
                    .to_string(),
                    injection_hits: Vec::new(),
                    witness: None,
                });
            };
            let kind_filter = args.kind.as_deref().map(|s| s.to_ascii_lowercase());
            let file_filter = args.file.as_deref().map(|s| s.to_ascii_lowercase());

            let sinks: Vec<_> =
                ast.sinks
                    .iter()
                    .filter(|s| {
                        kind_filter.as_ref().is_none_or(|k| {
                            format!("{:?}", s.kind).to_ascii_lowercase().contains(k)
                        }) && file_filter
                            .as_ref()
                            .is_none_or(|f| s.file.to_ascii_lowercase().contains(f))
                    })
                    .take(MAX_AST_QUERY_HITS)
                    .collect();

            Ok(ToolExecutionOutput {
                ok: true,
                tool_name: "query_ast_sinks",
                payload_json: serde_json::json!({
                    "available": true,
                    "files_parsed": ast.files_parsed,
                    "sinks": sinks,
                    "sensitive_reads": &ast.sensitive_reads[..ast.sensitive_reads.len().min(MAX_AST_QUERY_HITS)],
                    "obfuscation_signals": &ast.obfuscation_signals[..ast.obfuscation_signals.len().min(MAX_AST_QUERY_HITS)],
                    "reachability_edges": &ast.reachability_edges[..ast.reachability_edges.len().min(MAX_AST_QUERY_HITS)],
                })
                .to_string(),
                injection_hits: Vec::new(),
                witness: None,
            })
        }
        ToolCallRequest::InspectKernelTrace(args) => {
            let graph = ctx.latest_graph_snapshot();
            let payload = match graph {
                None => serde_json::json!({
                    "available": false,
                    "note": "No Tier 1/2 KernelBehaviorGraphV1 baseline trace recorded"
                }),
                Some(ref g) => match args.section {
                    KernelTraceSection::Processes => {
                        serde_json::json!({"tier": g.tier, "processes": g.processes})
                    }
                    KernelTraceSection::Syscalls => serde_json::json!({
                        "tier": g.tier,
                        "memory_anomalies": g.memory_anomalies,
                        "kernel_escapes": g.kernel_escapes,
                        "violations": g.violations,
                    }),
                    KernelTraceSection::Canary => {
                        serde_json::json!({"tier": g.tier, "canary_exfiltrations": g.canary_exfiltrations})
                    }
                    KernelTraceSection::NetworkSinkhole => {
                        serde_json::json!({"tier": g.tier, "network_events": g.network_events})
                    }
                    KernelTraceSection::FsDiff => serde_json::json!({
                        "tier": g.tier,
                        "files_written": g.files_written,
                        "overlay_added": g.overlay_added,
                        "overlay_modified": g.overlay_modified,
                    }),
                },
            };
            Ok(ToolExecutionOutput {
                ok: true,
                tool_name: "inspect_kernel_trace",
                payload_json: payload.to_string(),
                injection_hits: Vec::new(),
                witness: None,
            })
        }
        ToolCallRequest::InspectBinaryElfWasm(args) => {
            let rel = validate_relative_pkg_path(&args.path)?;
            let matched = ctx
                .binary_report
                .and_then(|r| r.binaries.iter().find(|b| b.path == rel));
            let mut serialized = serde_json::json!({
                "path": rel,
                "binary_inspection": matched,
            })
            .to_string();
            if serialized.len() > MAX_BINARY_REPORT_BYTES {
                serialized.truncate(MAX_BINARY_REPORT_BYTES);
            }
            Ok(ToolExecutionOutput {
                ok: true,
                tool_name: "inspect_binary_elf_wasm",
                payload_json: serialized,
                injection_hits: Vec::new(),
                witness: None,
            })
        }
        ToolCallRequest::DeobfuscateInWasm(args) => {
            let rel = validate_relative_pkg_path(&args.path)?;
            let file = ctx
                .archive
                .files
                .iter()
                .find(|f| f.path == rel)
                .ok_or_else(|| anyhow::anyhow!("file '{}' not found", rel))?;
            let (slice_text, _, _, _) = extract_bounded_lines(
                file.raw_bytes.as_ref(),
                args.start_line,
                args.end_line,
                MAX_DEOBF_SLICE_BYTES,
            );

            // Gate computation in a zero-capability, fuel-bounded wasmtime 40 sandbox.
            let wasm_verified = if let Some(ref sb) = *DEOBF_SANDBOX {
                let res = sb
                    .run(
                        DEOBF_GATE_WASM,
                        &SandboxInput::new(
                            &ctx.pkg_id.name,
                            &ctx.pkg_id.version,
                            &ctx.pkg_id.ecosystem,
                        ),
                    )
                    .await?;
                res.exit_code == 0
            } else {
                false
            };

            let mut outputs = match args.mode {
                DeobfuscationMode::Base64HexXor => {
                    let mut v = decode_base64_candidates(&slice_text);
                    v.extend(decode_hex_and_charcode(&slice_text));
                    v
                }
                DeobfuscationMode::JsAstFold => {
                    let folded = fold_js_concat_literals(&slice_text);
                    let mut v = vec![format!("folded_js -> {folded}")];
                    v.extend(decode_base64_candidates(&folded));
                    v.extend(decode_hex_and_charcode(&folded));
                    v
                }
            };

            let mut joined = outputs.join("\n");
            if joined.len() > MAX_DEOBF_OUTPUT_BYTES {
                joined.truncate(MAX_DEOBF_OUTPUT_BYTES);
            }
            outputs.clear();
            let hits = detect_prompt_injection_patterns(&joined);
            let wrapped = ctx
                .envelope
                .wrap_untrusted(&format!("deobf:{rel}"), &joined);

            Ok(ToolExecutionOutput {
                ok: true,
                tool_name: "deobfuscate_in_wasm",
                payload_json: serde_json::json!({
                    "wasm_sandbox_isolated": wasm_verified,
                    "mode": args.mode,
                    "decoded": wrapped,
                    "prompt_injection_signals": hits,
                })
                .to_string(),
                injection_hits: hits,
                witness: None,
            })
        }
        ToolCallRequest::DiffPreviousVersion(args) => {
            let max_hunks = usize::from(args.max_hunks.unwrap_or(20)).min(MAX_DIFF_HUNKS);
            let diff_summary = crate::diff::analyze(
                ctx.manifest,
                &ctx.prev_sandbox.cloned().unwrap_or_else(|| SandboxResult {
                    findings: vec![],
                    observed_network_hosts: vec![],
                    observed_fs_writes: vec![],
                    observed_process_spawns: vec![],
                    kernel_graph: None,
                    metrics: crate::sandbox::SandboxMetrics::default(),
                }),
                ctx.prev_manifest,
                ctx.prev_sandbox,
            );
            let hunks: Vec<_> = diff_summary.findings.into_iter().take(max_hunks).collect();
            Ok(ToolExecutionOutput {
                ok: true,
                tool_name: "diff_previous_version",
                payload_json: serde_json::json!({
                    "has_previous_manifest": ctx.prev_manifest.is_some(),
                    "has_previous_sandbox": ctx.prev_sandbox.is_some(),
                    "hunk_count": hunks.len(),
                    "hunks": hunks,
                })
                .to_string(),
                injection_hits: Vec::new(),
                witness: None,
            })
        }
        ToolCallRequest::RequestSandboxReplay(args) => {
            args.trigger_spec.validate()?;
            {
                let mut used = ctx.replays_used.lock().unwrap_or_else(|e| e.into_inner());
                if *used >= ctx.max_sandbox_replays.min(3) {
                    bail!(
                        "sandbox replay budget exhausted (used {}/{})",
                        *used,
                        ctx.max_sandbox_replays.min(3)
                    );
                }
                *used += 1;
            }
            let timeout =
                u64::from(args.timeout_secs.unwrap_or(10)).clamp(1, MAX_REPLAY_TIMEOUT_SECS);
            let replay_res = ctx
                .replay_executor
                .replay(
                    ctx.pkg_id,
                    ctx.tarball,
                    ctx.manifest,
                    &args.trigger_spec,
                    timeout,
                )
                .await?;

            let mut witness = None;
            if let Some(ref graph) = replay_res.kernel_graph {
                *ctx.latest_kernel_graph
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(graph.clone());

                let mut violation_codes: Vec<KernelViolationCode> = graph
                    .violations
                    .iter()
                    .map(|v| v.code)
                    .filter(|c| *c != KernelViolationCode::KbRootfsDigestMismatch)
                    .collect();
                violation_codes.sort();
                violation_codes.dedup();

                if !violation_codes.is_empty() {
                    let draft = TriggerWitnessDraftV1 {
                        schema_version: 1,
                        package_canonical: ctx.pkg_id.canonical(),
                        content_hash: ctx.content_hash.to_string(),
                        manifest_sha256: canonical_manifest_digest(ctx.manifest),
                        sandbox_policy_bundle_id: ctx.policy_bundle_id.to_string(),
                        trigger_spec: args.trigger_spec.clone(),
                        observed_violations: violation_codes,
                        graph_canonical_digest: graph.canonical_digest(),
                    };
                    draft.validate_wire_budget()?;
                    witness = Some(draft);
                }
            }

            let wrapped_findings = ctx.envelope.wrap_untrusted(
                "sandbox_replay",
                &serde_json::to_string(&replay_res.findings).unwrap_or_default(),
            );
            Ok(ToolExecutionOutput {
                ok: true,
                tool_name: "request_sandbox_replay",
                payload_json: serde_json::json!({
                    "engine_used": replay_res.metrics.engine_used,
                    "exit_code": replay_res.metrics.exit_code,
                    "witness_sealed": witness.is_some(),
                    "witness": witness,
                    "findings": wrapped_findings,
                    "observed_network_hosts": replay_res.observed_network_hosts,
                    "observed_fs_writes": replay_res.observed_fs_writes,
                    "observed_process_spawns": replay_res.observed_process_spawns,
                })
                .to_string(),
                injection_hits: Vec::new(),
                witness,
            })
        }
    }
}

pub fn load_archive_bounded(tarball: &[u8]) -> Result<UnpackedArchive> {
    creg_unpack::unpack_tarball_with_limits(tarball, UnpackLimits::default())
        .map_err(|e| anyhow::anyhow!("bounded unpack failed: {e}"))
}

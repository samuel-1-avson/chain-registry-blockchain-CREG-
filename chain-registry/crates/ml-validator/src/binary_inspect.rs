//! Native binary (ELF, Mach-O, PE) & WebAssembly structural inspector (`VNA-11` / Layer 2).
//!
//! Scans every file inside [`creg_unpack::UnpackedArchive`] by magic header
//! regardless of file extension, surfacing disguised binaries (`assets/logo.png`),
//! writable+executable (`RWX`) segments, dangerous kernel/loader imports
//! (`ptrace`, `memfd_create`, `io_uring_setup`, `CreateRemoteThread`, WASI sockets),
//! packer entropy, and embedded IOC strings.

use common::{Finding, FindingSeverity};
use object::read::macho::{FatArch, MachOFatFile32};
use object::{
    Architecture, BinaryFormat, Object, ObjectSection, ObjectSegment, ObjectSymbol, SectionFlags,
    SegmentFlags,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use wasmparser::{Parser, Payload};

const HIGH_ENTROPY_PACKED_THRESHOLD: f64 = 7.2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinaryFormatKind {
    Elf,
    MachO,
    Pe,
    Wasm,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InspectedBinary {
    pub path: String,
    pub sha256: String,
    pub size_bytes: usize,
    pub format: BinaryFormatKind,
    pub architecture: String,
    pub is_disguised_extension: bool,
    pub has_rwx_segment: bool,
    pub is_packed: bool,
    pub entropy: f64,
    pub dangerous_imports: Vec<String>,
    pub wasi_sensitive_imports: Vec<String>,
    pub ioc_strings: Vec<String>,
    pub parse_error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BinaryInspectionReport {
    pub binaries: Vec<InspectedBinary>,
}

const DISGUISED_TEXT_OR_ASSET_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "svg", "ico", "webp", "bmp", "txt", "md", "json", "js", "mjs",
    "cjs", "ts", "mts", "cts", "py", "rs", "css", "html", "toml", "yml", "yaml", "csv", "lock",
];

const CRITICAL_IMPORT_SYMBOLS: &[&str] = &[
    "ptrace",
    "memfd_create",
    "process_vm_writev",
    "io_uring_setup",
    "init_module",
    "finit_module",
    "createremotethread",
    "writeprocessmemory",
    "virtualallocex",
    "ntallocatevirtualmemory",
];

const HIGH_IMPORT_SYMBOLS: &[&str] = &[
    "execve",
    "execveat",
    "system",
    "popen",
    "dlopen",
    "dlsym",
    "socket",
    "connect",
    "mprotect",
    "virtualalloc",
    "urldownloadtofile",
    "winexec",
    "shellexecute",
];

/// Inspects every file in `archive` for ELF, Mach-O, PE, or WebAssembly magic bytes.
pub fn inspect_archive(archive: &creg_unpack::UnpackedArchive) -> BinaryInspectionReport {
    let mut binaries = Vec::new();
    for file in &archive.files {
        let bytes = file.raw_bytes.as_ref();
        let disguised = file
            .extension_lower()
            .is_some_and(|ext| DISGUISED_TEXT_OR_ASSET_EXTS.contains(&ext.as_str()));

        if bytes.starts_with(b"\x7fELF") {
            binaries.push(inspect_native(
                file,
                BinaryFormatKind::Elf,
                disguised,
                bytes,
            ));
        } else if bytes.starts_with(b"\x00asm") {
            binaries.push(inspect_wasm(file, disguised, bytes));
        } else if bytes.starts_with(b"MZ") {
            if let Ok(obj) = object::File::parse(bytes) {
                if obj.format() == BinaryFormat::Pe {
                    binaries.push(inspect_native_parsed(
                        file,
                        BinaryFormatKind::Pe,
                        disguised,
                        bytes,
                        &obj,
                    ));
                }
            }
        } else if matches_thin_macho(bytes) {
            binaries.push(inspect_native(
                file,
                BinaryFormatKind::MachO,
                disguised,
                bytes,
            ));
        } else if bytes.starts_with(&[0xca, 0xfe, 0xba, 0xbe]) && is_valid_macho_fat32(bytes) {
            binaries.push(inspect_macho_fat(file, disguised, bytes));
        }
    }
    BinaryInspectionReport { binaries }
}

fn matches_thin_macho(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xfe, 0xed, 0xfa, 0xce])
        || bytes.starts_with(&[0xce, 0xfa, 0xed, 0xfe])
        || bytes.starts_with(&[0xfe, 0xed, 0xfa, 0xcf])
        || bytes.starts_with(&[0xcf, 0xfa, 0xed, 0xfe])
}

/// Validates Mach-O fat headers strictly so Java `.class` files (which share the
/// `0xCAFEBABE` 4-byte prefix) are never misclassified as Mach-O binaries.
fn is_valid_macho_fat32(bytes: &[u8]) -> bool {
    let Ok(fat) = MachOFatFile32::parse(bytes) else {
        return false;
    };
    let arches = fat.arches();
    if arches.is_empty() || arches.len() > 16 {
        return false;
    }
    arches.iter().all(|arch| {
        if arch.architecture() == Architecture::Unknown {
            return false;
        }
        let offset = arch.offset() as u64;
        let size = arch.size() as u64;
        size >= 28 && offset.saturating_add(size) <= bytes.len() as u64
    })
}

fn inspect_macho_fat(
    file: &creg_unpack::UnpackedFile,
    disguised: bool,
    bytes: &[u8],
) -> InspectedBinary {
    let mut entry = empty_entry(file, BinaryFormatKind::MachO, disguised, bytes);
    let Ok(fat) = MachOFatFile32::parse(bytes) else {
        entry.parse_error = Some("invalid fat Mach-O header".to_string());
        return entry;
    };
    let mut arches = Vec::new();
    let mut imports = BTreeSet::new();
    for arch in fat.arches() {
        arches.push(format!("{:?}", arch.architecture()));
        if let Ok(slice) = arch.data(bytes) {
            if let Ok(obj) = object::File::parse(slice) {
                if segment_or_section_is_rwx(&obj) {
                    entry.has_rwx_segment = true;
                }
                collect_dangerous_imports(&obj, &mut imports);
            }
        }
    }
    entry.architecture = arches.join(",");
    entry.dangerous_imports = imports.into_iter().collect();
    entry
}

fn empty_entry(
    file: &creg_unpack::UnpackedFile,
    format: BinaryFormatKind,
    disguised: bool,
    bytes: &[u8],
) -> InspectedBinary {
    let entropy = creg_unpack::shannon_entropy_bytes(bytes);
    let upx = bytes
        .windows(4)
        .any(|w| w == b"UPX0" || w == b"UPX1" || w == b"UPX!");
    InspectedBinary {
        path: file.path.clone(),
        sha256: file.sha256.clone(),
        size_bytes: bytes.len(),
        format,
        architecture: "Unknown".to_string(),
        is_disguised_extension: disguised,
        has_rwx_segment: false,
        is_packed: upx || entropy >= HIGH_ENTROPY_PACKED_THRESHOLD,
        entropy,
        dangerous_imports: Vec::new(),
        wasi_sensitive_imports: Vec::new(),
        ioc_strings: extract_ioc_strings(bytes),
        parse_error: None,
    }
}

fn inspect_native(
    file: &creg_unpack::UnpackedFile,
    format: BinaryFormatKind,
    disguised: bool,
    bytes: &[u8],
) -> InspectedBinary {
    match object::File::parse(bytes) {
        Ok(obj) => inspect_native_parsed(file, format, disguised, bytes, &obj),
        Err(err) => {
            let mut entry = empty_entry(file, format, disguised, bytes);
            entry.parse_error = Some(err.to_string());
            entry
        }
    }
}

fn inspect_native_parsed(
    file: &creg_unpack::UnpackedFile,
    format: BinaryFormatKind,
    disguised: bool,
    bytes: &[u8],
    obj: &object::File<'_>,
) -> InspectedBinary {
    let mut entry = empty_entry(file, format, disguised, bytes);
    entry.architecture = format!("{:?}", obj.architecture());
    entry.has_rwx_segment = segment_or_section_is_rwx(obj);
    let mut imports = BTreeSet::new();
    collect_dangerous_imports(obj, &mut imports);
    entry.dangerous_imports = imports.into_iter().collect();
    entry
}

fn segment_or_section_is_rwx(obj: &object::File<'_>) -> bool {
    for seg in obj.segments() {
        match seg.flags() {
            SegmentFlags::Elf { p_flags } => {
                let rwx = object::elf::PF_W | object::elf::PF_X;
                if (p_flags & rwx) == rwx {
                    return true;
                }
            }
            SegmentFlags::MachO {
                initprot, maxprot, ..
            } => {
                let rwx = object::macho::VM_PROT_WRITE | object::macho::VM_PROT_EXECUTE;
                if (initprot & rwx) == rwx || (maxprot & rwx) == rwx {
                    return true;
                }
            }
            SegmentFlags::Coff { characteristics } => {
                let rwx = object::pe::IMAGE_SCN_MEM_WRITE | object::pe::IMAGE_SCN_MEM_EXECUTE;
                if (characteristics & rwx) == rwx {
                    return true;
                }
            }
            _ => {}
        }
    }
    for sec in obj.sections() {
        match sec.flags() {
            SectionFlags::Elf { sh_flags } => {
                let rwx = u64::from(object::elf::SHF_WRITE | object::elf::SHF_EXECINSTR);
                if (sh_flags & rwx) == rwx {
                    return true;
                }
            }
            SectionFlags::Coff { characteristics } => {
                let rwx = object::pe::IMAGE_SCN_MEM_WRITE | object::pe::IMAGE_SCN_MEM_EXECUTE;
                if (characteristics & rwx) == rwx {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn collect_dangerous_imports(obj: &object::File<'_>, out: &mut BTreeSet<String>) {
    if let Ok(imports) = obj.imports() {
        for imp in imports {
            check_symbol_name(&String::from_utf8_lossy(imp.name()), out);
        }
    }
    for sym in obj.dynamic_symbols() {
        if let Ok(name) = sym.name() {
            check_symbol_name(name, out);
        }
    }
    for sym in obj.symbols() {
        if let Ok(name) = sym.name() {
            check_symbol_name(name, out);
        }
    }
}

fn check_symbol_name(raw_name: &str, out: &mut BTreeSet<String>) {
    let clean = raw_name.trim_start_matches('_').to_ascii_lowercase();
    for &pat in CRITICAL_IMPORT_SYMBOLS
        .iter()
        .chain(HIGH_IMPORT_SYMBOLS.iter())
    {
        if clean == pat
            || clean.starts_with(&format!("{pat}@"))
            || clean.starts_with(&format!("{pat}a")) && pat.ends_with("file")
        {
            out.insert(raw_name.to_string());
            break;
        }
        if clean == "shellexecutea"
            || clean == "shellexecutew"
            || clean == "urldownloadtofilea"
            || clean == "urldownloadtofilew"
        {
            out.insert(raw_name.to_string());
            break;
        }
    }
}

fn inspect_wasm(
    file: &creg_unpack::UnpackedFile,
    disguised: bool,
    bytes: &[u8],
) -> InspectedBinary {
    let mut entry = empty_entry(file, BinaryFormatKind::Wasm, disguised, bytes);
    entry.architecture = "wasm32".to_string();

    let mut wasi_sensitive = BTreeSet::new();
    for payload in Parser::new(0).parse_all(bytes) {
        match payload {
            Ok(Payload::ImportSection(reader)) => {
                for imp in reader {
                    match imp {
                        Ok(wasmparser::Imports::Single(_, import)) => {
                            record_wasm_import(import.module, import.name, &mut wasi_sensitive);
                        }
                        Ok(wasmparser::Imports::Compact1 { module, items }) => {
                            for item in items {
                                match item {
                                    Ok(item) => {
                                        record_wasm_import(module, item.name, &mut wasi_sensitive);
                                    }
                                    Err(e) => {
                                        entry.parse_error = Some(e.to_string());
                                        break;
                                    }
                                }
                            }
                        }
                        Ok(wasmparser::Imports::Compact2 { module, names, .. }) => {
                            for name in names {
                                match name {
                                    Ok(name) => {
                                        record_wasm_import(module, name, &mut wasi_sensitive);
                                    }
                                    Err(e) => {
                                        entry.parse_error = Some(e.to_string());
                                        break;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            entry.parse_error = Some(e.to_string());
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                entry.parse_error = Some(e.to_string());
                break;
            }
            _ => {}
        }
    }
    entry.wasi_sensitive_imports = wasi_sensitive.into_iter().collect();
    entry
}

fn record_wasm_import(module: &str, name: &str, out: &mut BTreeSet<String>) {
    let full = format!("{module}::{name}");
    let lower = full.to_ascii_lowercase();
    if is_sensitive_wasm_import(&lower) {
        out.insert(full);
    }
}

fn is_sensitive_wasm_import(full_lower: &str) -> bool {
    const WASI_SENSITIVE: &[&str] = &[
        "sock_accept",
        "sock_recv",
        "sock_send",
        "sock_shutdown",
        "proc_exec",
        "proc_raise",
        "path_open",
        "path_unlink",
        "path_rename",
        "environ_get",
        "::eval",
        "::exec",
        "::fetch",
        "::spawn",
        "command",
    ];
    WASI_SENSITIVE
        .iter()
        .any(|needle| full_lower.contains(needle))
}

fn extract_ioc_strings(bytes: &[u8]) -> Vec<String> {
    let mut out = BTreeSet::new();
    let mut run = Vec::new();
    for &b in bytes {
        if (0x20..=0x7e).contains(&b) {
            run.push(b);
        } else {
            maybe_push_ioc(&run, &mut out);
            run.clear();
            if out.len() >= 32 {
                break;
            }
        }
    }
    if out.len() < 32 {
        maybe_push_ioc(&run, &mut out);
    }
    out.into_iter().collect()
}

fn maybe_push_ioc(run: &[u8], out: &mut BTreeSet<String>) {
    if run.len() < 6 {
        return;
    }
    let s = String::from_utf8_lossy(run);
    let lower = s.to_ascii_lowercase();
    if lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("stratum+tcp://")
        || lower.contains("stratum+ssl://")
        || lower.contains(".ssh/id_")
        || lower.contains("/etc/shadow")
        || lower.contains("/dev/tcp/")
        || s.contains("BEGIN OPENSSH PRIVATE KEY")
        || s.contains("BEGIN RSA PRIVATE KEY")
    {
        out.insert(s.chars().take(160).collect());
    }
}

impl BinaryInspectionReport {
    /// Converts binary/WASM structural inspection results into consensus findings.
    pub fn to_findings(&self) -> Vec<Finding> {
        let mut findings = Vec::new();

        for bin in &self.binaries {
            if bin.is_disguised_extension {
                findings.push(Finding {
                    id: "ST-BIN-DISGUISED".into(),
                    title: "Binary payload disguised with non-binary extension".into(),
                    severity: FindingSeverity::Critical,
                    description: format!(
                        "File '{}' has {:?} ({}) magic header behind a non-binary extension.",
                        bin.path, bin.format, bin.architecture
                    ),
                    file: bin.path.clone(),
                    line: None,
                });
            }
            if bin.has_rwx_segment {
                findings.push(Finding {
                    id: "ST-BIN-RWX".into(),
                    title: "Executable binary maps writable+executable (RWX) memory".into(),
                    severity: FindingSeverity::Critical,
                    description: format!(
                        "{:?} binary '{}' ({}) contains RWX segment/section flags.",
                        bin.format, bin.path, bin.architecture
                    ),
                    file: bin.path.clone(),
                    line: None,
                });
            }
            if !bin.dangerous_imports.is_empty() {
                let has_critical = bin.dangerous_imports.iter().any(|sym| {
                    let lower = sym.trim_start_matches('_').to_ascii_lowercase();
                    CRITICAL_IMPORT_SYMBOLS.contains(&lower.as_str())
                });
                findings.push(Finding {
                    id: "ST-BIN-DANGEROUS-IMPORTS".into(),
                    title: "Native binary references dangerous system/loader symbols".into(),
                    severity: if has_critical {
                        FindingSeverity::Critical
                    } else {
                        FindingSeverity::High
                    },
                    description: format!(
                        "{:?} binary '{}' references dangerous symbols: {}",
                        bin.format,
                        bin.path,
                        bin.dangerous_imports.join(", ")
                    ),
                    file: bin.path.clone(),
                    line: None,
                });
            }
            if bin.is_packed {
                findings.push(Finding {
                    id: "ST-BIN-PACKED".into(),
                    title: "Packed or high-entropy native/WASM binary".into(),
                    severity: FindingSeverity::High,
                    description: format!(
                        "{:?} binary '{}' has Shannon entropy {:.2} bits/byte or UPX markers.",
                        bin.format, bin.path, bin.entropy
                    ),
                    file: bin.path.clone(),
                    line: None,
                });
            }
            if !bin.ioc_strings.is_empty() {
                findings.push(Finding {
                    id: "ST-BIN-IOC".into(),
                    title: "Suspicious network or credential IOC embedded in binary".into(),
                    severity: FindingSeverity::High,
                    description: format!(
                        "{:?} binary '{}' embeds IOC strings: {}",
                        bin.format,
                        bin.path,
                        bin.ioc_strings
                            .iter()
                            .take(5)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    file: bin.path.clone(),
                    line: None,
                });
            }
            if !bin.wasi_sensitive_imports.is_empty() {
                let high = bin.wasi_sensitive_imports.iter().any(|imp| {
                    let l = imp.to_ascii_lowercase();
                    l.contains("sock_")
                        || l.contains("proc_")
                        || l.contains("::eval")
                        || l.contains("::exec")
                        || l.contains("::fetch")
                });
                findings.push(Finding {
                    id: "ST-BIN-WASM-SENSITIVE-IMPORT".into(),
                    title: "WebAssembly module imports sensitive host capabilities".into(),
                    severity: if high {
                        FindingSeverity::High
                    } else {
                        FindingSeverity::Medium
                    },
                    description: format!(
                        "WASM module '{}' imports host capabilities: {}",
                        bin.path,
                        bin.wasi_sensitive_imports.join(", ")
                    ),
                    file: bin.path.clone(),
                    line: None,
                });
            }
            if bin.format == BinaryFormatKind::Wasm && bin.parse_error.is_some() {
                findings.push(Finding {
                    id: "ST-BIN-WASM-MALFORMED".into(),
                    title: "Malformed WebAssembly module".into(),
                    severity: FindingSeverity::High,
                    description: format!(
                        "WASM payload '{}' failed structural validation: {}",
                        bin.path,
                        bin.parse_error.as_deref().unwrap_or("parse error")
                    ),
                    file: bin.path.clone(),
                    line: None,
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

    fn minimal_elf64_rwx_le() -> Vec<u8> {
        let mut buf = vec![0u8; 64 + 56];
        // ELF64 magic, little-endian, version 1
        buf[0..16].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        buf[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
        buf[18..20].copy_from_slice(&62u16.to_le_bytes()); // EM_X86_64
        buf[20..24].copy_from_slice(&1u32.to_le_bytes()); // EV_CURRENT
        buf[32..40].copy_from_slice(&64u64.to_le_bytes()); // e_phoff = 64
        buf[52..54].copy_from_slice(&64u16.to_le_bytes()); // e_ehsize = 64
        buf[54..56].copy_from_slice(&56u16.to_le_bytes()); // e_phentsize = 56
        buf[56..58].copy_from_slice(&1u16.to_le_bytes()); // e_phnum = 1
                                                          // Program header 0: PT_LOAD with PF_R | PF_W | PF_X (7)
        buf[64..68].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
        buf[68..72].copy_from_slice(&7u32.to_le_bytes()); // p_flags = PF_R | PF_W | PF_X
        buf[96..104].copy_from_slice(&64u64.to_le_bytes()); // p_filesz
        buf[104..112].copy_from_slice(&64u64.to_le_bytes()); // p_memsz
        buf
    }

    fn minimal_wasm_with_sock_accept() -> Vec<u8> {
        // Minimal valid WASM module: type ()->(), import "wasi_snapshot_preview1"::"sock_accept" func 0
        let mut wasm = vec![0x00, b'a', b's', b'm', 0x01, 0x00, 0x00, 0x00];
        // Type section (id=1): 1 func type ()->()
        wasm.extend_from_slice(&[0x01, 0x04, 0x01, 0x60, 0x00, 0x00]);
        // Import section (id=2): module "wasi_snapshot_preview1" (22B), name "sock_accept" (11B), kind=0 (func), type=0
        let mut imp_body = vec![0x01]; // 1 import
        imp_body.push(22);
        imp_body.extend_from_slice(b"wasi_snapshot_preview1");
        imp_body.push(11);
        imp_body.extend_from_slice(b"sock_accept");
        imp_body.push(0x00); // Func import
        imp_body.push(0x00); // Type index 0
        wasm.push(0x02);
        wasm.push(imp_body.len() as u8);
        wasm.extend_from_slice(&imp_body);
        wasm
    }

    #[test]
    fn detects_disguised_rwx_elf_binary() {
        let elf = minimal_elf64_rwx_le();
        let archive = test_archive(&[("package/assets/logo.png", &elf)]);
        let report = inspect_archive(&archive);
        assert_eq!(report.binaries.len(), 1);
        assert_eq!(report.binaries[0].format, BinaryFormatKind::Elf);
        assert!(report.binaries[0].is_disguised_extension);
        assert!(report.binaries[0].has_rwx_segment);

        let findings = report.to_findings();
        assert!(findings.iter().any(|f| f.id == "ST-BIN-DISGUISED"));
        assert!(findings.iter().any(|f| f.id == "ST-BIN-RWX"));
    }

    #[test]
    fn detects_wasm_sensitive_import_and_malformed_wasm_and_ignores_java_class() {
        let wasm = minimal_wasm_with_sock_accept();
        let bad_wasm = b"\x00asm\xff\xff\xff\xff";
        let java_class = b"\xca\xfe\xba\xbe\x00\x00\x00\x3d\x00\x10";
        let archive = test_archive(&[
            ("package/icon.png", &wasm),
            ("package/corrupt.wasm", bad_wasm),
            ("classes/Hello.class", java_class),
        ]);
        let report = inspect_archive(&archive);
        // Only the 2 WASM files are inspected; Java .class must NOT trigger Mach-O.
        assert_eq!(report.binaries.len(), 2);
        let findings = report.to_findings();
        assert!(findings
            .iter()
            .any(|f| f.id == "ST-BIN-DISGUISED" && f.file == "package/icon.png"));
        assert!(findings.iter().any(|f| {
            f.id == "ST-BIN-WASM-SENSITIVE-IMPORT" && f.description.contains("sock_accept")
        }));
        assert!(findings
            .iter()
            .any(|f| f.id == "ST-BIN-WASM-MALFORMED" && f.file == "package/corrupt.wasm"));
    }
}

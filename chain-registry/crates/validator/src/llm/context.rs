use common::Finding;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub(super) struct SelectedFile {
    pub path: String,
    pub content: String,
    pub entropy: f64,
}

#[derive(Debug, Clone)]
pub(super) struct PackageEvidencePacket {
    pub file_count: usize,
    pub entropy_alerts: Vec<super::EntropyAlert>,
    pub selected_files: Vec<SelectedFile>,
}

pub(super) struct EvidencePacketBuilder;

impl EvidencePacketBuilder {
    pub(super) fn build(tarball: &[u8], prior_findings: &[Finding]) -> PackageEvidencePacket {
        let Ok(unpacked) = creg_unpack::unpack_tarball(tarball) else {
            return PackageEvidencePacket {
                file_count: 0,
                entropy_alerts: Vec::new(),
                selected_files: Vec::new(),
            };
        };

        let files: HashMap<String, Vec<u8>> = unpacked
            .files
            .into_iter()
            .map(|file| (file.path, file.raw_bytes.to_vec()))
            .collect();

        let entropy_alerts = super::scan_entropy(&files);
        let selected_files =
            super::select_files_for_analysis(&files, prior_findings, super::max_files_to_analyse())
                .into_iter()
                .filter_map(|(path, data, entropy)| {
                    // Use lossy UTF-8 projection so invalid bytes cannot blind LLM triage (VNA-09).
                    let content_lossy = String::from_utf8_lossy(&data);
                    let content = super::sanitize_content(&content_lossy, super::max_file_chars());
                    if content.trim().is_empty() {
                        None
                    } else {
                        Some(SelectedFile {
                            path,
                            content,
                            entropy,
                        })
                    }
                })
                .collect();

        PackageEvidencePacket {
            file_count: files.len(),
            entropy_alerts,
            selected_files,
        }
    }
}

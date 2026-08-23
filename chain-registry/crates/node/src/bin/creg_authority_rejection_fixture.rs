//! Test fixture sidecar for the node's private-temp launch contract.
//!
//! This binary is not a production namespace-authority verifier. It only
//! confirms that a cleared environment received the v2 protocol pin and one
//! absolute private scratch directory through TEMP/TMP/TMPDIR, then rejects.

use std::io::{Read, Write};
use std::path::Path;

const REQUIRED_ENV: [&str; 4] = ["CREG_AUTHORITY_ADAPTER_PROTOCOL", "TEMP", "TMP", "TMPDIR"];
const FORBIDDEN_INHERITED_ENV: [&str; 6] = [
    "PATH",
    "HOME",
    "USERPROFILE",
    "USERNAME",
    "APPDATA",
    "LOCALAPPDATA",
];

fn main() {
    // Drain stdin so the parent write task is not killed by a broken pipe.
    let mut stdin = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut stdin);

    if !launch_contract_holds() {
        print!("ENV_CONTRACT_FAILED");
        return;
    }

    let mut stderr = std::io::stderr();
    let _ = stderr.write_all(b"pypi authority verification rejected: launch-fixture");
    std::process::exit(1);
}

fn launch_contract_holds() -> bool {
    let protocol = std::env::var("CREG_AUTHORITY_ADAPTER_PROTOCOL").ok();
    let temp = std::env::var("TEMP").ok();
    let tmp = std::env::var("TMP").ok();
    let tmpdir = std::env::var("TMPDIR").ok();
    let Some(temp) = temp else {
        return false;
    };
    if protocol.as_deref() != Some("2")
        || tmp.as_deref() != Some(temp.as_str())
        || tmpdir.as_deref() != Some(temp.as_str())
    {
        return false;
    }
    let temp_path = Path::new(&temp);
    if !temp_path.is_absolute() || !temp_path.is_dir() {
        return false;
    }
    if !temp_path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("creg-authority-"))
    {
        return false;
    }
    if cfg!(windows) && temp.starts_with(r"\\?\") {
        return false;
    }
    let keys: Vec<String> = std::env::vars().map(|(key, _)| key).collect();
    if !REQUIRED_ENV
        .iter()
        .all(|key| keys.iter().any(|present| present.eq_ignore_ascii_case(key)))
    {
        return false;
    }
    !keys.iter().any(|key| {
        FORBIDDEN_INHERITED_ENV
            .iter()
            .any(|forbidden| key.eq_ignore_ascii_case(forbidden))
    })
}

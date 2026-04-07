// crates/cli/src/intercept.rs
// Manages the PATH shims that transparently intercept package manager calls.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

const SHIM_TARGETS: &[(&str, &str)] = &[
    ("npm", "npm"),
    ("pip", "pip"),
    ("pip3", "pip"),
    ("cargo", "cargo-shim"),
    ("gem", "gem"),
    ("mvn", "mvn"),
];

/// Install shim binaries into `shim_dir` (defaults to ~/.local/bin).
/// The shims are copies of the current `creg` binary, each named after
/// the package manager they intercept. When called as `npm`, the binary
/// reads argv[0] to know which ecosystem to route to.
pub fn setup_shims(shim_dir: Option<&Path>) -> Result<()> {
    let dir = shim_dir.map(PathBuf::from).unwrap_or_else(default_shim_dir);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Cannot create shim dir: {}", dir.display()))?;

    let current_exe = std::env::current_exe()?;
    let exe_dir = current_exe.parent().unwrap_or(Path::new("."));

    for (shim_name, bin_name) in SHIM_TARGETS {
        // Look for the compiled shim binary next to the creg binary
        let shim_binary = exe_dir.join(bin_name);
        let source = if shim_binary.exists() {
            &shim_binary
        } else {
            // Fallback: use the creg binary itself (it dispatches on argv[0])
            &current_exe
        };
        let dest = dir.join(shim_name);
        std::fs::copy(source, &dest)
            .with_context(|| format!("Failed to copy shim to {}", dest.display()))?;

        // Mark executable on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&dest)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&dest, perms)?;
        }

        println!("  ✓ Installed shim: {}", dest.display());
    }

    println!(
        "\nMake sure {} is at the start of your PATH:",
        dir.display()
    );
    println!("  export PATH=\"{}:$PATH\"", dir.display());
    Ok(())
}

/// Remove shims by deleting the named files from the shim directory.
pub fn remove_shims() -> Result<()> {
    let dir = default_shim_dir();
    for (shim_name, _) in SHIM_TARGETS {
        let path = dir.join(shim_name);
        if path.exists() {
            std::fs::remove_file(&path)?;
            println!("  ✓ Removed shim: {}", path.display());
        }
    }
    Ok(())
}

fn default_shim_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".local")
        .join("bin")
}

fn main() {
    // Release workflows inject the tag through CREG_RELEASE_VERSION. Local
    // builds fall back to the Cargo package version so the binary always has
    // a truthful, self-describing version string.
    let version = std::env::var("CREG_RELEASE_VERSION")
        .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string());
    println!(
        "cargo:rustc-env=CREG_EFFECTIVE_VERSION={}",
        version.trim_start_matches('v')
    );
    println!("cargo:rerun-if-env-changed=CREG_RELEASE_VERSION");
}

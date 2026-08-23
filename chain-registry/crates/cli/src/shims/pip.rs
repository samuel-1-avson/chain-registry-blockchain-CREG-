// crates/cli/src/shims/pip.rs
// PATH shim for `pip install` — same fail-closed gate as `creg install`.

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let exit_code = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime")
        .block_on(async {
            let sub = args.get(1).map(String::as_str).unwrap_or("");
            if !matches!(sub, "install" | "download") {
                return run_real("pip", &args[1..]).await;
            }
            run_verified_install("pypi", &args).await
        });

    std::process::exit(exit_code);
}

async fn run_verified_install(ecosystem: &str, args: &[String]) -> i32 {
    let allow = resolver::install_gate::shim_allows_unverified(args);
    if resolver::install_gate::requirement_file_requested(args) && !allow {
        eprintln!(
            "\x1b[31m✗ BLOCKED\x1b[0m  pip requirement files are not chain-gated. \
             Install named packages with `creg install` or set {}=true only as break-glass.",
            resolver::install_gate::ALLOW_UNVERIFIED_ENV
        );
        return 1;
    }

    let packages: Vec<&str> = args[2..]
        .iter()
        .filter(|a| !a.starts_with('-') && !a.ends_with(".txt") && !a.ends_with(".cfg"))
        .map(String::as_str)
        .collect();

    for pkg in &packages {
        match resolver::install_gate::gate_shim_package(pkg, ecosystem, allow, None).await {
            Ok(resolver::install_gate::InstallDecision::Allow) => {
                eprintln!("\x1b[32m✓ VERIFIED\x1b[0m {pkg}");
            }
            Ok(resolver::install_gate::InstallDecision::AllowUnverified) => {
                eprintln!(
                    "\x1b[33m⚠ BREAK-GLASS\x1b[0m  {pkg} — installing unverified ({})",
                    resolver::install_gate::ALLOW_UNVERIFIED_ENV
                );
            }
            Ok(resolver::install_gate::InstallDecision::Block { message, .. }) => {
                eprintln!("\x1b[31m✗ BLOCKED\x1b[0m  {message}");
                return 1;
            }
            Err(err) => {
                eprintln!("\x1b[31m✗ BLOCKED\x1b[0m  {err}");
                return 1;
            }
        }
    }

    let forwarded = resolver::install_gate::strip_unverified_flag(args);
    run_real("pip", &forwarded[1..]).await
}

async fn run_real(pm: &str, args: &[String]) -> i32 {
    let args_str: Vec<&str> = args.iter().map(String::as_str).collect();
    let real = match which::which_all(pm).ok().and_then(|mut it| {
        it.next();
        it.next()
    }) {
        Some(p) => p,
        None => {
            eprintln!("chain-registry: real {pm} not found");
            return 127;
        }
    };

    std::process::Command::new(real)
        .args(&args_str)
        .status()
        .map(|s| s.code().unwrap_or(1))
        .unwrap_or(1)
}

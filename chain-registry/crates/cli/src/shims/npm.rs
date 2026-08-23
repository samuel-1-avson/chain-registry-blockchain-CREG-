// crates/cli/src/shims/npm.rs
// This binary is placed in ~/.local/bin/npm (or %LOCALAPPDATA%\creg\bin\npm.cmd).
// Named-package installs use the same fail-closed gate as `creg install`.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let passthrough = should_passthrough(&args);

    let exit_code = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime")
        .block_on(async {
            if passthrough {
                run_real_pm("npm", &args[1..]).await
            } else {
                run_verified_install("npm", &args).await
            }
        });

    std::process::exit(exit_code);
}

fn should_passthrough(args: &[String]) -> bool {
    let sub = args.get(1).map(String::as_str).unwrap_or("");
    !matches!(sub, "install" | "i" | "ci" | "add")
}

async fn run_verified_install(ecosystem: &str, args: &[String]) -> i32 {
    let allow = resolver::install_gate::shim_allows_unverified(args);
    let packages: Vec<&str> = args[2..]
        .iter()
        .filter(|a| !a.starts_with('-') && !a.starts_with("--"))
        .map(String::as_str)
        .collect();

    if packages.is_empty() {
        eprintln!(
            "\x1b[33m⚠\x1b[0m chain-registry: `{ecosystem} {}` with no package arguments is not chain-gated. Use `creg install` for named packages.",
            args.get(1).map(String::as_str).unwrap_or("install")
        );
        let forwarded = resolver::install_gate::strip_unverified_flag(args);
        return run_real_pm(ecosystem, &forwarded[1..]).await;
    }

    if let Err(code) = gate_packages(ecosystem, &packages, allow).await {
        return code;
    }

    let forwarded = resolver::install_gate::strip_unverified_flag(args);
    run_real_pm(ecosystem, &forwarded[1..]).await
}

async fn gate_packages(ecosystem: &str, packages: &[&str], allow: bool) -> Result<(), i32> {
    for pkg in packages {
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
                return Err(1);
            }
            Err(err) => {
                eprintln!("\x1b[31m✗ BLOCKED\x1b[0m  {err}");
                return Err(1);
            }
        }
    }
    Ok(())
}

async fn run_real_pm(pm: &str, args: &[String]) -> i32 {
    let real = match which::which_all(pm)
        .map(|mut it| {
            it.next();
            it.next()
        })
        .ok()
        .flatten()
    {
        Some(p) => p,
        None => {
            eprintln!("chain-registry: could not find real {pm}");
            return 127;
        }
    };

    std::process::Command::new(real)
        .args(args)
        .status()
        .map(|s| s.code().unwrap_or(1))
        .unwrap_or(1)
}

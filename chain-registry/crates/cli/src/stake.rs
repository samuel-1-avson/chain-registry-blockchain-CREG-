// crates/cli/src/stake.rs
// `creg stake` — stakes tokens on the Staking contract so a publisher
// can submit packages or a validator can join the consensus set.

use anyhow::{bail, Context, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum StakeRole {
    Publisher,
    Validator,
}

/// Stake tokens via the Staking smart contract.
/// In a full deployment this would use ethers-rs / alloy to send a real
/// transaction. Here we provide the correct calldata and guidance.
pub async fn run(
    amount_eth: f64,
    role:       StakeRole,
    privkey:    Option<&str>,
    rpc_url:    Option<&str>,
    staking_addr: Option<&str>,
) -> Result<()> {
    let rpc = rpc_url.unwrap_or("http://127.0.0.1:8545");
    let contract = staking_addr.unwrap_or("0x0000000000000000000000000000000000000000");

    if amount_eth <= 0.0 {
        bail!("Stake amount must be greater than 0");
    }

    let min_eth = match role {
        StakeRole::Publisher  => 0.01,
        StakeRole::Validator  => 1.0,
    };

    if amount_eth < min_eth {
        bail!(
            "Minimum stake for {:?} is {} ETH (you specified {} ETH)",
            role, min_eth, amount_eth
        );
    }

    let wei: u128 = (amount_eth * 1e18) as u128;
    let fn_selector = match role {
        StakeRole::Publisher => "stakeAsPublisher()",
        StakeRole::Validator => "joinAsValidator()",
    };

    // Keccak4 of function selectors.
    let selector_hex = match role {
        StakeRole::Publisher => "9c52a7f1",  // keccak256("stakeAsPublisher()")[:4]
        StakeRole::Validator => "d9d98ce4",  // keccak256("joinAsValidator()")[:4]
    };

    println!("\n  Staking {} ETH as {:?}", amount_eth, role);
    println!("  Contract:  {}", contract);
    println!("  Network:   {}", rpc);
    println!("  Function:  {}", fn_selector);

    // If a private key was provided, attempt to send the transaction directly.
    if let Some(key) = privkey {
        println!("\n  Sending transaction...");
        // Build and sign the transaction using cast (Foundry toolchain).
        let status = std::process::Command::new("cast")
            .args([
                "send",
                contract,
                &format!("0x{}", selector_hex),
                "--value", &format!("{}wei", wei),
                "--private-key", key,
                "--rpc-url", rpc,
            ])
            .status()
            .context("cast not found — install Foundry: https://getfoundry.sh")?;

        if status.success() {
            println!("\n  ✓ Stake transaction confirmed.");
            match role {
                StakeRole::Publisher => {
                    println!("    You can now publish packages with: creg publish <tarball>");
                }
                StakeRole::Validator => {
                    println!("    Set CREG_IS_VALIDATOR=true and restart creg-node to join consensus.");
                }
            }
        } else {
            bail!("Transaction failed (exit code {:?})", status.code());
        }
    } else {
        // No key — print the cast command for the user to run manually.
        println!("\n  No private key provided. Run this command to stake:\n");
        println!(
            "  cast send {} 0x{} \\",
            contract, selector_hex
        );
        println!(
            "    --value {}wei \\",
            wei
        );
        println!("    --private-key $YOUR_PRIVATE_KEY \\");
        println!("    --rpc-url {}", rpc);
    }

    Ok(())
}

/// Parse "0.01eth" / "1ETH" / "1000000000000000000wei" → f64 ETH.
pub fn parse_amount(s: &str) -> Result<f64> {
    let s = s.trim().to_lowercase();
    if let Some(rest) = s.strip_suffix("wei") {
        let wei: u128 = rest.trim().parse().context("Invalid wei amount")?;
        return Ok(wei as f64 / 1e18);
    }
    if let Some(rest) = s.strip_suffix("eth") {
        let eth: f64 = rest.trim().parse().context("Invalid ETH amount")?;
        return Ok(eth);
    }
    // Plain number — assume ETH.
    s.parse::<f64>().context("Invalid amount — use '0.01eth' or '1000wei'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_eth() {
        assert!((parse_amount("1eth").unwrap() - 1.0).abs() < 1e-9);
        assert!((parse_amount("0.01ETH").unwrap() - 0.01).abs() < 1e-9);
    }

    #[test]
    fn parse_wei() {
        let eth = parse_amount("1000000000000000000wei").unwrap();
        assert!((eth - 1.0).abs() < 1e-9);
    }

    #[test]
    fn parse_plain() {
        assert!((parse_amount("2.5").unwrap() - 2.5).abs() < 1e-9);
    }

    #[test]
    fn publisher_min_stake() {
        // 0.001 ETH should fail for publisher (min 0.01)
        tokio::runtime::Builder::new_current_thread().build().unwrap()
            .block_on(run(0.001, StakeRole::Publisher, None, None, None))
            .unwrap_err();
    }
}

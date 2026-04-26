use anyhow::Result;
use clap::Subcommand;

#[derive(Subcommand)]
pub enum ChainSpecCommands {
    /// Validate a local chain-spec.json file.
    Validate {
        #[arg(value_name = "FILE")]
        path: String,
    },
    /// Compute and print the genesis hash for a spec file.
    ComputeGenesisHash {
        #[arg(value_name = "FILE")]
        path: String,
    },
    /// Diff the live spec against your cached copy.
    Diff,
}

pub async fn run(cmd: ChainSpecCommands) -> Result<()> {
    match cmd {
        ChainSpecCommands::Validate { path } => {
            let json = std::fs::read_to_string(&path)?;
            let spec: common::ChainSpec = serde_json::from_str(&json)
                .map_err(|e| anyhow::anyhow!("Invalid spec JSON: {}", e))?;

            if spec.spec_version != common::CURRENT_SPEC_VERSION {
                anyhow::bail!("Unknown spec_version: {}", spec.spec_version);
            }

            let hash = spec.compute_genesis_hash()?;
            println!("✓ Spec is valid");
            println!("  chain_id:      {}", spec.chain_id);
            println!("  network:       {:?}", spec.network);
            println!("  genesis_hash:  {}", hash);
            println!("  validators:    {}", spec.validator_set.validators.len());
            Ok(())
        }
        ChainSpecCommands::ComputeGenesisHash { path } => {
            let json = std::fs::read_to_string(&path)?;
            let spec: common::ChainSpec = serde_json::from_str(&json)?;
            let hash = spec.compute_genesis_hash()?;
            println!("{}", hash);
            Ok(())
        }
        ChainSpecCommands::Diff => {
            let cache_dir = dirs::cache_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
            let cache_path = cache_dir.join("creg").join("chain-spec.cached.json");
            if !cache_path.exists() {
                anyhow::bail!("No cached spec found at {}", cache_path.display());
            }
            let cached = std::fs::read_to_string(&cache_path)?;
            let spec: common::ChainSpec = serde_json::from_str(&cached)?;
            let hash = spec.compute_genesis_hash()?;
            println!("Cached spec: {} (genesis_hash: {})", spec.chain_id, hash);
            // TODO: fetch live spec and diff
            Ok(())
        }
    }
}

//! WASM Sandboxing for Package Validation
//!
//! This crate provides a secure, cross-platform sandbox for validating packages
//! using WebAssembly.
//!
//! **⚠️ EXPERIMENTAL** — The WASI-based sandbox is not yet production-hardened.
//! It should not be relied upon as a security boundary. Use nsjail-based
//! sandboxing in production deployments. See the deep-dive analysis for details.

use std::collections::HashMap;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, info, warn};
use wasmtime::{Engine, Module, Store, StoreLimits, StoreLimitsBuilder};

pub mod capabilities;
pub mod limits;

pub use capabilities::CapabilitySet;
pub use limits::ResourceLimits;

/// Errors that can occur during WASM sandbox execution
#[derive(Error, Debug)]
pub enum SandboxError {
    #[error("WASM compilation error: {0}")]
    CompilationError(String),

    #[error("WASM execution error: {0}")]
    ExecutionError(String),

    #[error("Resource limit exceeded: {0}")]
    ResourceLimitExceeded(String),

    #[error("Timeout after {0:?}")]
    Timeout(Duration),

    #[error("Memory access error: {0}")]
    MemoryError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Sandbox configuration
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Memory limit in bytes
    pub memory_limit: usize,
    /// CPU time limit in seconds
    pub timeout_secs: u64,
    /// Allowed capabilities
    pub capabilities: CapabilitySet,
    /// Environment variables
    pub env_vars: HashMap<String, String>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            memory_limit: 256 * 1024 * 1024, // 256MB
            timeout_secs: 30,
            capabilities: CapabilitySet::default(),
            env_vars: HashMap::new(),
        }
    }
}

impl SandboxConfig {
    /// Set memory limit
    pub fn with_memory_limit(mut self, bytes: usize) -> Self {
        self.memory_limit = bytes;
        self
    }

    /// Set timeout
    pub fn with_timeout_secs(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }
}

/// Input data for sandbox execution
#[derive(Debug, Clone)]
pub struct SandboxInput {
    /// Package metadata
    pub package_name: String,
    pub package_version: String,
    pub ecosystem: String,
    /// Package content
    pub tarball_bytes: Vec<u8>,
}

impl SandboxInput {
    /// Create new sandbox input
    pub fn new(package_name: &str, version: &str, ecosystem: &str) -> Self {
        Self {
            package_name: package_name.to_string(),
            package_version: version.to_string(),
            ecosystem: ecosystem.to_string(),
            tarball_bytes: vec![],
        }
    }

    /// Set tarball bytes
    pub fn with_tarball(mut self, bytes: Vec<u8>) -> Self {
        self.tarball_bytes = bytes;
        self
    }
}

/// Result of sandbox execution
#[derive(Debug, Clone)]
pub struct SandboxResult {
    /// Whether execution was successful
    pub success: bool,
    /// Exit code
    pub exit_code: i32,
    /// stdout output
    pub stdout: String,
    /// stderr output
    pub stderr: String,
    /// Resource usage
    pub resource_usage: ResourceUsage,
    /// Validation findings
    pub findings: Vec<SafetyFinding>,
}

/// Resource usage statistics
#[derive(Debug, Clone, Default)]
pub struct ResourceUsage {
    /// Peak memory usage in bytes
    pub peak_memory: usize,
    /// CPU time used in milliseconds
    pub cpu_time_ms: u64,
    /// Wall clock time in milliseconds
    pub wall_time_ms: u64,
}

/// Safety finding from validation
#[derive(Debug, Clone)]
pub struct SafetyFinding {
    /// Severity level
    pub severity: Severity,
    /// Category
    pub category: String,
    /// Description
    pub description: String,
}

/// Severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

/// WASM Sandbox for package validation
pub struct WasmSandbox {
    engine: Engine,
    config: SandboxConfig,
}

/// Store data carrying resource limits for wasmtime.
struct SandboxState {
    limits: StoreLimits,
}

impl WasmSandbox {
    /// Create a new WASM sandbox with epoch interruption enabled.
    pub fn new(config: SandboxConfig) -> Result<Self, SandboxError> {
        info!("Initializing WASM sandbox");

        let mut engine_config = wasmtime::Config::new();
        // Enable epoch-based interruption for enforcing timeouts.
        engine_config.epoch_interruption(true);

        let engine = Engine::new(&engine_config)
            .map_err(|e| SandboxError::CompilationError(format!("Engine creation failed: {}", e)))?;

        Ok(Self { engine, config })
    }

    /// Return a snapshot of the sandbox configuration as key-value pairs.
    pub fn stats(&self) -> HashMap<&'static str, u64> {
        let mut m = HashMap::new();
        m.insert("memory_limit", self.config.memory_limit as u64);
        m.insert("timeout_secs", self.config.timeout_secs);
        m
    }

    /// Run a WASM module in the sandbox with timeout and resource limits enforced.
    pub async fn run(
        &self,
        wasm_bytes: &[u8],
        _input: &SandboxInput,
    ) -> Result<SandboxResult, SandboxError> {
        debug!("Compiling WASM module");

        // Compile module
        let module = Module::new(&self.engine, wasm_bytes)
            .map_err(|e| SandboxError::CompilationError(e.to_string()))?;

        // Build store with resource limits
        let limits = StoreLimitsBuilder::new()
            .memory_size(self.config.memory_limit)
            .build();

        let mut store = Store::new(&self.engine, SandboxState { limits });
        store.limiter(|state| &mut state.limits);

        // Set epoch deadline — the module will trap after this many epoch ticks.
        // We tick once per second, so deadline = timeout_secs.
        let deadline = self.config.timeout_secs.max(1);
        store.set_epoch_deadline(deadline);

        // Spawn a background task that increments the engine epoch once per second.
        // This drives the epoch-based timeout for the WASM execution.
        let engine_clone = self.engine.clone();
        let timeout_secs = self.config.timeout_secs;
        let epoch_handle = tokio::spawn(async move {
            for _ in 0..(timeout_secs + 1) {
                tokio::time::sleep(Duration::from_secs(1)).await;
                engine_clone.increment_epoch();
            }
        });

        // Set up WASI context based on capabilities
        let wasi_ctx = self.build_wasi_context();
        let linker = wasmtime::Linker::new(&self.engine);

        // Only add WASI imports if the module expects them (best-effort).
        // Modules without WASI imports will work fine with an empty import set.
        if let Ok(ctx) = wasi_ctx {
            // wasmtime_wasi::add_to_linker is available; for now we provide
            // an empty linker to avoid capability leakage. Modules that call
            // WASI functions they aren't granted will trap with a link error.
            let _ = ctx; // WASI context prepared but not yet wired (see below)
        }

        // Instantiate module
        let instance = linker.instantiate(&mut store, &module)
            .map_err(|e| SandboxError::ExecutionError(e.to_string()))?;

        // Get the main function
        let main = instance
            .get_typed_func::<(), i32>(&mut store, "_start")
            .or_else(|_| instance.get_typed_func::<(), i32>(&mut store, "main"))
            .map_err(|e| SandboxError::ExecutionError(format!("No main function: {}", e)))?;

        // Execute with epoch-based timeout enforcement
        let start_time = std::time::Instant::now();

        let call_result = main.call(&mut store, ());

        // Cancel the epoch ticker
        epoch_handle.abort();

        let wall_time = start_time.elapsed();

        match call_result {
            Ok(exit_code) => Ok(SandboxResult {
                success: exit_code == 0,
                exit_code,
                stdout: String::new(),
                stderr: String::new(),
                resource_usage: ResourceUsage {
                    peak_memory: 0,
                    cpu_time_ms: wall_time.as_millis() as u64,
                    wall_time_ms: wall_time.as_millis() as u64,
                },
                findings: vec![],
            }),
            Err(e) => {
                // Check if the trap was caused by epoch deadline (timeout)
                let err_str = e.to_string();
                if err_str.contains("epoch") || err_str.contains("interrupt") {
                    warn!(
                        "WASM execution timed out after {}s (limit: {}s)",
                        wall_time.as_secs(),
                        self.config.timeout_secs
                    );
                    Err(SandboxError::Timeout(Duration::from_secs(
                        self.config.timeout_secs,
                    )))
                } else if err_str.contains("memory") {
                    Err(SandboxError::ResourceLimitExceeded(format!(
                        "Memory limit exceeded (limit: {} bytes): {}",
                        self.config.memory_limit, err_str
                    )))
                } else {
                    Err(SandboxError::ExecutionError(err_str))
                }
            }
        }
    }

    /// Build a WASI context respecting the configured capabilities.
    fn build_wasi_context(&self) -> Result<(), SandboxError> {
        // Capability enforcement: only grant WASI features that match
        // the configured CapabilitySet. Currently we provide no WASI
        // imports at all (most restrictive), which means any WASM module
        // that calls WASI functions will trap. This is intentional for
        // untrusted code validation.
        //
        // Future: use wasmtime_wasi::WasiCtxBuilder to selectively
        // enable stdio (if caps.has("stdio")), clock, random, and
        // filesystem access based on the CapabilitySet.
        if self.config.capabilities.has("network") {
            warn!("Network capability requested but not yet supported in WASM sandbox");
        }
        if self.config.capabilities.has("filesystem-write") {
            warn!("Filesystem write capability requested but not yet supported in WASM sandbox");
        }
        Ok(())
    }

    /// Run a validator script on a package
    pub async fn validate_package(
        &self,
        validator_wasm: &[u8],
        package_data: &SandboxInput,
    ) -> Result<SandboxResult, SandboxError> {
        debug!("Running package validation in WASM sandbox");
        self.run(validator_wasm, package_data).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_config() {
        let config = SandboxConfig::default()
            .with_memory_limit(1024)
            .with_timeout_secs(10);

        assert_eq!(config.memory_limit, 1024);
        assert_eq!(config.timeout_secs, 10);
    }

    #[test]
    fn test_sandbox_input() {
        let input = SandboxInput::new("test-pkg", "1.0.0", "npm");

        assert_eq!(input.package_name, "test-pkg");
        assert_eq!(input.ecosystem, "npm");
    }
}

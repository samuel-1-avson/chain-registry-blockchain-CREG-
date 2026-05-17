//! WASM Sandboxing for Package Validation
//!
//! This crate provides a secure, cross-platform sandbox for validating packages
//! using WebAssembly.
//!
//! The sandbox enforces memory limits via `StoreLimitsBuilder`, CPU limits via
//! wasmtime epoch-based interruption, and provides no WASI imports by default
//! (modules that call WASI functions will trap). This makes it suitable as a
//! fallback when nsjail/gVisor/Docker are unavailable, though production
//! deployments should still prefer nsjail for the strongest isolation.

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

/// Store data carrying resource limits and execution state for wasmtime.
struct SandboxState {
    limits: StoreLimits,
    /// Exit code communicated by `proc_exit` before the WASM trap fires.
    /// None if the module terminated via a trap rather than a clean exit.
    exit_code: Option<i32>,
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

        // Build store with resource limits.
        let limits = StoreLimitsBuilder::new()
            .memory_size(self.config.memory_limit)
            .build();

        let mut store = Store::new(&self.engine, SandboxState { limits, exit_code: None });
        store.limiter(|state| &mut state.limits);

        // Epoch-based CPU timeout: the engine epoch is incremented by a
        // background task once per second; the store traps after `deadline`
        // ticks have passed.
        let deadline = self.config.timeout_secs.max(1);
        store.set_epoch_deadline(deadline);

        let engine_clone = self.engine.clone();
        let timeout_secs = self.config.timeout_secs;
        let epoch_handle = tokio::spawn(async move {
            for _ in 0..(timeout_secs + 1) {
                tokio::time::sleep(Duration::from_secs(1)).await;
                engine_clone.increment_epoch();
            }
        });

        // ── WASI stub linker ──────────────────────────────────────────────────
        //
        // We provide stub host functions for every WASI snapshot_preview1
        // import a compiled WASM binary might reference. This prevents
        // link-time "unknown import" errors while granting zero real capability:
        //
        //  • I/O stubs (fd_write, fd_read, fd_close, fd_seek, fd_fdstat_get,
        //    fd_filestat_get) return EBADF (8) — all file descriptors are invalid.
        //  • Filesystem stubs (fd_prestat_get, path_open) return EBADF.
        //  • Environment / args stubs report 0 entries.
        //  • clock_time_get returns ENOSYS (52) — no real clock access.
        //  • random_get returns ENOSYS (52) — no entropy.
        //  • sched_yield returns 0 (no-op).
        //  • poll_oneoff returns ENOSYS (52).
        //
        // CRITICAL: proc_exit must NOT call std::process::exit — that would
        // terminate the entire node process. Instead, we record the exit code
        // in the store and bail with a tagged error so the caller can detect a
        // clean WASM exit (code 0) vs a real trap.
        let mut linker: wasmtime::Linker<SandboxState> = wasmtime::Linker::new(&self.engine);

        // fd_write(fd, iovs, iovs_len, nwritten) -> errno
        let _ = linker.func_wrap(
            "wasi_snapshot_preview1", "fd_write",
            |_: wasmtime::Caller<'_, SandboxState>,
             _fd: i32, _iovs: i32, _iovs_len: i32, _nwritten: i32| -> i32 { 8 },
        );
        // fd_read(fd, iovs, iovs_len, nread) -> errno
        let _ = linker.func_wrap(
            "wasi_snapshot_preview1", "fd_read",
            |_: wasmtime::Caller<'_, SandboxState>,
             _fd: i32, _iovs: i32, _iovs_len: i32, _nread: i32| -> i32 { 8 },
        );
        // fd_close(fd) -> errno
        let _ = linker.func_wrap(
            "wasi_snapshot_preview1", "fd_close",
            |_: wasmtime::Caller<'_, SandboxState>, _fd: i32| -> i32 { 8 },
        );
        // fd_seek(fd, offset, whence, newoffset) -> errno
        let _ = linker.func_wrap(
            "wasi_snapshot_preview1", "fd_seek",
            |_: wasmtime::Caller<'_, SandboxState>,
             _fd: i32, _offset: i64, _whence: i32, _newoffset: i32| -> i32 { 8 },
        );
        // fd_fdstat_get(fd, stat) -> errno
        let _ = linker.func_wrap(
            "wasi_snapshot_preview1", "fd_fdstat_get",
            |_: wasmtime::Caller<'_, SandboxState>, _fd: i32, _stat: i32| -> i32 { 8 },
        );
        // fd_fdstat_set_flags(fd, flags) -> errno
        let _ = linker.func_wrap(
            "wasi_snapshot_preview1", "fd_fdstat_set_flags",
            |_: wasmtime::Caller<'_, SandboxState>, _fd: i32, _flags: i32| -> i32 { 8 },
        );
        // fd_filestat_get(fd, stat) -> errno
        let _ = linker.func_wrap(
            "wasi_snapshot_preview1", "fd_filestat_get",
            |_: wasmtime::Caller<'_, SandboxState>, _fd: i32, _stat: i32| -> i32 { 8 },
        );
        // fd_prestat_get(fd, prestat) -> errno
        let _ = linker.func_wrap(
            "wasi_snapshot_preview1", "fd_prestat_get",
            |_: wasmtime::Caller<'_, SandboxState>, _fd: i32, _prestat: i32| -> i32 { 8 },
        );
        // fd_prestat_dir_name(fd, path, path_len) -> errno
        let _ = linker.func_wrap(
            "wasi_snapshot_preview1", "fd_prestat_dir_name",
            |_: wasmtime::Caller<'_, SandboxState>,
             _fd: i32, _path: i32, _len: i32| -> i32 { 8 },
        );
        // path_open(dirfd, dirflags, path, path_len, oflags, fs_rights_base,
        //           fs_rights_inheriting, fdflags, fd) -> errno
        let _ = linker.func_wrap(
            "wasi_snapshot_preview1", "path_open",
            |_: wasmtime::Caller<'_, SandboxState>,
             _: i32, _: i32, _: i32, _: i32, _: i32,
             _: i64, _: i64, _: i32, _: i32| -> i32 { 8 },
        );
        // environ_sizes_get(count_ptr, size_ptr) -> errno
        let _ = linker.func_wrap(
            "wasi_snapshot_preview1", "environ_sizes_get",
            |_: wasmtime::Caller<'_, SandboxState>, _count: i32, _size: i32| -> i32 { 0 },
        );
        // environ_get(environ, buf) -> errno
        let _ = linker.func_wrap(
            "wasi_snapshot_preview1", "environ_get",
            |_: wasmtime::Caller<'_, SandboxState>, _environ: i32, _buf: i32| -> i32 { 0 },
        );
        // args_sizes_get(argc_ptr, argv_buf_size_ptr) -> errno
        let _ = linker.func_wrap(
            "wasi_snapshot_preview1", "args_sizes_get",
            |_: wasmtime::Caller<'_, SandboxState>, _argc: i32, _size: i32| -> i32 { 0 },
        );
        // args_get(argv, argv_buf) -> errno
        let _ = linker.func_wrap(
            "wasi_snapshot_preview1", "args_get",
            |_: wasmtime::Caller<'_, SandboxState>, _argv: i32, _buf: i32| -> i32 { 0 },
        );
        // clock_time_get(id, precision, time_ptr) -> errno  [ENOSYS — no clock]
        let _ = linker.func_wrap(
            "wasi_snapshot_preview1", "clock_time_get",
            |_: wasmtime::Caller<'_, SandboxState>,
             _id: i32, _prec: i64, _time: i32| -> i32 { 52 /* ENOSYS */ },
        );
        // clock_res_get(id, res_ptr) -> errno
        let _ = linker.func_wrap(
            "wasi_snapshot_preview1", "clock_res_get",
            |_: wasmtime::Caller<'_, SandboxState>, _id: i32, _res: i32| -> i32 { 52 },
        );
        // random_get(buf, len) -> errno  [ENOSYS — no entropy]
        let _ = linker.func_wrap(
            "wasi_snapshot_preview1", "random_get",
            |_: wasmtime::Caller<'_, SandboxState>, _buf: i32, _len: i32| -> i32 { 52 },
        );
        // sched_yield() -> errno  [no-op]
        let _ = linker.func_wrap(
            "wasi_snapshot_preview1", "sched_yield",
            |_: wasmtime::Caller<'_, SandboxState>| -> i32 { 0 },
        );
        // poll_oneoff(in, out, nsubs, nevents) -> errno
        let _ = linker.func_wrap(
            "wasi_snapshot_preview1", "poll_oneoff",
            |_: wasmtime::Caller<'_, SandboxState>,
             _in: i32, _out: i32, _nsubs: i32, _nevents: i32| -> i32 { 52 },
        );

        // proc_exit: MUST NOT call std::process::exit — that would kill the node.
        // Instead, record the exit code in the store and bail with a sentinel
        // error so the run() match arm below can detect a clean WASM exit.
        let _ = linker.func_wrap(
            "wasi_snapshot_preview1", "proc_exit",
            |mut caller: wasmtime::Caller<'_, SandboxState>, code: i32| -> Result<(), anyhow::Error> {
                caller.data_mut().exit_code = Some(code);
                anyhow::bail!("wasm-sandbox-proc-exit:{}", code)
            },
        );

        // ── Instantiate and run ───────────────────────────────────────────────
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| SandboxError::ExecutionError(e.to_string()))?;

        // Try _start (WASI convention) first, then fall back to main.
        // Both are treated as () -> i32; proc_exit is the canonical way to
        // return an exit code from a WASI module.
        let call_result = instance
            .get_typed_func::<(), i32>(&mut store, "_start")
            .or_else(|_| instance.get_typed_func::<(), i32>(&mut store, "main"))
            .map_err(|e| SandboxError::ExecutionError(format!("No entry point (_start/main): {}", e)))
            .and_then(|f| f.call(&mut store, ()).map_err(|e| {
                // If proc_exit was called, the sentinel error is surfaced here.
                SandboxError::ExecutionError(e.to_string())
            }));

        epoch_handle.abort();

        // Measure peak memory: collect Memory handles first (ends the mutable
        // borrow of `store`), then query sizes with an immutable borrow.
        let memories: Vec<wasmtime::Memory> = instance
            .exports(&mut store)
            .filter_map(|exp| exp.into_memory())
            .collect();
        let peak_memory: usize = memories.iter().map(|mem| mem.data_size(&store)).sum();

        match call_result {
            Ok(exit_code) => Ok(SandboxResult {
                success: exit_code == 0,
                exit_code,
                stdout: String::new(),
                stderr: String::new(),
                resource_usage: ResourceUsage {
                    peak_memory,
                    cpu_time_ms: 0,
                    wall_time_ms: 0,
                },
                findings: vec![],
            }),
            Err(SandboxError::ExecutionError(ref msg))
                if msg.contains("wasm-sandbox-proc-exit:") =>
            {
                // Clean proc_exit — extract the recorded exit code.
                let code = store.data().exit_code.unwrap_or(0);
                Ok(SandboxResult {
                    success: code == 0,
                    exit_code: code,
                    stdout: String::new(),
                    stderr: String::new(),
                    resource_usage: ResourceUsage {
                        peak_memory,
                        cpu_time_ms: 0,
                        wall_time_ms: 0,
                    },
                    findings: vec![],
                })
            }
            Err(SandboxError::ExecutionError(ref msg))
                if msg.contains("epoch") || msg.contains("interrupt") =>
            {
                warn!("WASM execution timed out (limit: {}s)", self.config.timeout_secs);
                Err(SandboxError::Timeout(Duration::from_secs(self.config.timeout_secs)))
            }
            Err(SandboxError::ExecutionError(ref msg)) if msg.contains("memory") => {
                Err(SandboxError::ResourceLimitExceeded(format!(
                    "Memory limit exceeded (limit: {} bytes): {}",
                    self.config.memory_limit, msg
                )))
            }
            Err(e) => Err(e),
        }
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

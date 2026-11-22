//! WASM Sandboxing for Package Validation
//!
//! This crate provides a secure, cross-platform sandbox for validating packages
//! using WebAssembly.
//!
//! The sandbox enforces memory limits via `StoreLimitsBuilder`, wall-clock
//! timeout via `tokio::time::timeout`, a WASI syscall budget, and capability
//! gates on `wasi_snapshot_preview1` host stubs (`stdio`, `clock`, `random`,
//! filesystem). Production deployments should still prefer nsjail for the
//! strongest isolation.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing::{debug, info, warn};
use wasmtime::{Engine, Module, Store, StoreLimits, StoreLimitsBuilder};

pub mod capabilities;
pub mod limits;

pub use capabilities::CapabilitySet;
pub use limits::{ResourceLimits, MAX_WASM_STDIO_BYTES};

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
    /// Resource limits enforced on the wasmtime store and WASI stubs
    pub limits: ResourceLimits,
    /// Environment variables
    pub env_vars: HashMap<String, String>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        let limits = ResourceLimits::default();
        Self {
            memory_limit: limits.max_memory,
            timeout_secs: secs_from_ms(limits.max_time_ms),
            capabilities: CapabilitySet::default(),
            limits,
            env_vars: HashMap::new(),
        }
    }
}

fn secs_from_ms(ms: u64) -> u64 {
    (ms / 1000).max(1)
}

impl SandboxConfig {
    /// Set memory limit
    pub fn with_memory_limit(mut self, bytes: usize) -> Self {
        self.memory_limit = bytes;
        self.limits.max_memory = bytes;
        self
    }

    /// Set timeout
    pub fn with_timeout_secs(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self.limits.max_time_ms = secs.saturating_mul(1000);
        self
    }

    /// Replace the capability set used by WASI host stubs.
    pub fn with_capabilities(mut self, capabilities: CapabilitySet) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// Replace resource limits and keep memory/timeout fields in sync.
    pub fn with_resource_limits(mut self, limits: ResourceLimits) -> Self {
        self.memory_limit = limits.max_memory;
        self.timeout_secs = secs_from_ms(limits.max_time_ms);
        self.limits = limits;
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
    /// stdout output (capped at `ResourceLimits::max_stdio_bytes`, default 64 KiB)
    pub stdout: String,
    /// stderr output (capped at `ResourceLimits::max_stdio_bytes`, default 64 KiB)
    pub stderr: String,
    /// True if stdout exceeded `max_stdio_bytes` and was truncated
    pub stdout_truncated: bool,
    /// True if stderr exceeded `max_stdio_bytes` and was truncated
    pub stderr_truncated: bool,
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
    capabilities: CapabilitySet,
    stdout: String,
    stderr: String,
    stdout_truncated: bool,
    stderr_truncated: bool,
    max_stdio_bytes: usize,
    syscalls: u64,
    max_syscalls: u64,
}

const WASI_ESUCCESS: i32 = 0;
const WASI_EBADF: i32 = 8;
const WASI_EFAULT: i32 = 21;
const WASI_EINVAL: i32 = 28;
const WASI_NOSYS: i32 = 52;
const WASI_NOTCAPABLE: i32 = 63;
const MAX_FD_WRITE_IOVS: usize = 1024;

struct WatchdogFinishGuard(Arc<AtomicBool>);

impl Drop for WatchdogFinishGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

fn push_bounded_utf8(dest: &mut String, bytes: &[u8], cap: usize, truncated: &mut bool) {
    if bytes.is_empty() {
        return;
    }
    let remaining = cap.saturating_sub(dest.len());
    if remaining == 0 {
        *truncated = true;
        return;
    }
    let slice = &bytes[..bytes.len().min(remaining)];
    if slice.len() < bytes.len() {
        *truncated = true;
    }
    let text = String::from_utf8_lossy(slice);
    if text.len() <= remaining {
        dest.push_str(&text);
    } else {
        // Multi-byte U+FFFD replacement expanded past `remaining`; clamp safely at char boundary.
        *truncated = true;
        let mut end = remaining;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        dest.push_str(&text[..end]);
    }
}

fn take_syscall(state: &mut SandboxState) -> i32 {
    state.syscalls = state.syscalls.saturating_add(1);
    if state.syscalls > state.max_syscalls {
        WASI_NOSYS
    } else {
        WASI_ESUCCESS
    }
}

fn require_cap(state: &SandboxState, cap: &str) -> i32 {
    if state.capabilities.has(cap) {
        WASI_ESUCCESS
    } else {
        WASI_NOTCAPABLE
    }
}

fn guest_memory(caller: &mut wasmtime::Caller<'_, SandboxState>) -> Option<wasmtime::Memory> {
    caller.get_export("memory")?.into_memory()
}

fn wasi_fd_write(
    mut caller: wasmtime::Caller<'_, SandboxState>,
    fd: i32,
    iovs: i32,
    iovs_len: i32,
    nwritten_ptr: i32,
) -> i32 {
    {
        let state = caller.data_mut();
        let err = take_syscall(state);
        if err != WASI_ESUCCESS {
            return err;
        }
        let err = require_cap(state, "stdio");
        if err != WASI_ESUCCESS {
            return err;
        }
    }
    if fd != 1 && fd != 2 {
        return WASI_EBADF;
    }
    if iovs_len < 0 || (iovs_len as usize) > MAX_FD_WRITE_IOVS {
        return WASI_EINVAL;
    }
    let Some(memory) = guest_memory(&mut caller) else {
        return WASI_EBADF;
    };
    let (current_len, max_stdio_bytes) = {
        let state = caller.data();
        let cur = if fd == 1 {
            state.stdout.len()
        } else {
            state.stderr.len()
        };
        (cur, state.max_stdio_bytes)
    };
    let mut remaining = max_stdio_bytes.saturating_sub(current_len);
    let data = memory.data(&caller);
    let iovs_base = (iovs as u32) as usize;
    let iovs_count = iovs_len as usize;
    let mut collected = Vec::with_capacity(remaining.min(4096));
    let mut total_requested: usize = 0;
    let mut overflowed = false;

    for i in 0..iovs_count {
        let base = iovs_base.saturating_add(i.saturating_mul(8));
        if base.saturating_add(8) > data.len() {
            return WASI_EFAULT;
        }
        let ptr = u32::from_le_bytes(data[base..base + 4].try_into().unwrap_or([0; 4])) as usize;
        let len =
            u32::from_le_bytes(data[base + 4..base + 8].try_into().unwrap_or([0; 4])) as usize;
        if ptr.saturating_add(len) > data.len() {
            return WASI_EFAULT;
        }
        total_requested = total_requested.saturating_add(len);
        if remaining > 0 {
            let take = len.min(remaining);
            collected.extend_from_slice(&data[ptr..ptr + take]);
            remaining -= take;
            if take < len {
                overflowed = true;
            }
        } else if len > 0 {
            overflowed = true;
        }
    }
    {
        let state = caller.data_mut();
        if fd == 1 {
            push_bounded_utf8(
                &mut state.stdout,
                &collected,
                state.max_stdio_bytes,
                &mut state.stdout_truncated,
            );
            if overflowed {
                state.stdout_truncated = true;
            }
        } else {
            push_bounded_utf8(
                &mut state.stderr,
                &collected,
                state.max_stdio_bytes,
                &mut state.stderr_truncated,
            );
            if overflowed {
                state.stderr_truncated = true;
            }
        }
    }
    let total = total_requested.min(u32::MAX as usize) as u32;
    let nptr = (nwritten_ptr as u32) as usize;
    let data = memory.data_mut(&mut caller);
    if nptr.saturating_add(4) > data.len() {
        return WASI_EFAULT;
    }
    data[nptr..nptr + 4].copy_from_slice(&total.to_le_bytes());
    WASI_ESUCCESS
}

impl WasmSandbox {
    /// Create a new WASM sandbox with cross-platform fuel metering and
    /// epoch-based timeout interruption (`wasmtime >= 40`, VNA-16).
    pub fn new(config: SandboxConfig) -> Result<Self, SandboxError> {
        info!("Initializing WASM sandbox");

        let mut engine_config = wasmtime::Config::new();
        engine_config.consume_fuel(true);
        engine_config.epoch_interruption(true);
        engine_config.max_wasm_stack(config.limits.max_stack_size);

        let engine = Engine::new(&engine_config).map_err(|e| {
            SandboxError::CompilationError(format!("Engine creation failed: {}", e))
        })?;

        Ok(Self { engine, config })
    }

    /// Return a snapshot of the sandbox configuration as key-value pairs.
    pub fn stats(&self) -> HashMap<&'static str, u64> {
        let mut m = HashMap::new();
        m.insert("memory_limit", self.config.memory_limit as u64);
        m.insert("timeout_secs", self.config.timeout_secs);
        m.insert("max_syscalls", self.config.limits.max_syscalls);
        m.insert("max_fuel", self.config.limits.max_fuel);
        m.insert(
            "max_open_files",
            u64::from(self.config.limits.max_open_files),
        );
        m.insert("max_stdio_bytes", self.config.limits.max_stdio_bytes as u64);
        m
    }

    /// Run a WASM module in the sandbox with fuel, bounded stdio, and
    /// epoch-based timeout interruption enforced cleanly on all platforms.
    pub async fn run(
        &self,
        wasm_bytes: &[u8],
        _input: &SandboxInput,
    ) -> Result<SandboxResult, SandboxError> {
        debug!("Compiling WASM module");
        let started = Instant::now();

        // Compile module (can be done on async thread — it's fast)
        let module = Module::new(&self.engine, wasm_bytes)
            .map_err(|e| SandboxError::CompilationError(e.to_string()))?;

        // Clone what we need for the blocking closure and epoch watchdog
        let engine = self.engine.clone();
        let memory_limit = self.config.limits.max_memory.max(self.config.memory_limit);
        let max_fuel = self.config.limits.max_fuel;
        let max_stdio_bytes = self.config.limits.max_stdio_bytes;
        let timeout_duration = Duration::from_millis(
            self.config
                .limits
                .max_time_ms
                .max(self.config.timeout_secs.saturating_mul(1000)),
        );
        let capabilities = self.config.capabilities.clone();
        let max_syscalls = self.config.limits.max_syscalls;

        let watchdog_done = Arc::new(AtomicBool::new(false));
        let watchdog_engine = engine.clone();
        let watchdog_done_task = watchdog_done.clone();
        let watchdog = tokio::spawn(async move {
            tokio::time::sleep(timeout_duration).await;
            while !watchdog_done_task.load(Ordering::Acquire) {
                watchdog_engine.increment_epoch();
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        let blocking_done = watchdog_done.clone();

        // Run instantiation + execution on a blocking thread with both epoch
        // interruption and fuel limits active.
        let handle = tokio::task::spawn_blocking(move || {
            let _finish_guard = WatchdogFinishGuard(blocking_done);

            // Build store with resource limits.
            let limits = StoreLimitsBuilder::new().memory_size(memory_limit).build();

            let mut store = Store::new(
                &engine,
                SandboxState {
                    limits,
                    exit_code: None,
                    capabilities,
                    stdout: String::new(),
                    stderr: String::new(),
                    stdout_truncated: false,
                    stderr_truncated: false,
                    max_stdio_bytes,
                    syscalls: 0,
                    max_syscalls,
                },
            );
            if let Err(e) = store.set_fuel(max_fuel) {
                return Err(SandboxError::ExecutionError(format!(
                    "Failed to configure fuel budget: {}",
                    e
                )));
            }
            store.set_epoch_deadline(1);
            store.limiter(|state| &mut state.limits);

            // ── WASI stub linker (ISSUE-015: capability + resource gated) ─
            let mut linker: wasmtime::Linker<SandboxState> = wasmtime::Linker::new(&engine);

            let _ = linker.func_wrap("wasi_snapshot_preview1", "fd_write", wasi_fd_write);
            let _ = linker.func_wrap(
                "wasi_snapshot_preview1",
                "fd_read",
                |mut caller: wasmtime::Caller<'_, SandboxState>,
                 _fd: i32,
                 _iovs: i32,
                 _iovs_len: i32,
                 _nread: i32|
                 -> i32 {
                    let state = caller.data_mut();
                    let err = take_syscall(state);
                    if err != WASI_ESUCCESS {
                        return err;
                    }
                    let err = require_cap(state, "stdio");
                    if err != WASI_ESUCCESS {
                        return err;
                    }
                    WASI_EBADF
                },
            );
            let denied_fd = |mut caller: wasmtime::Caller<'_, SandboxState>, _fd: i32| -> i32 {
                let state = caller.data_mut();
                let err = take_syscall(state);
                if err != WASI_ESUCCESS {
                    return err;
                }
                let err = require_cap(state, "stdio");
                if err != WASI_ESUCCESS {
                    return err;
                }
                WASI_EBADF
            };
            let _ = linker.func_wrap("wasi_snapshot_preview1", "fd_close", denied_fd);
            let _ = linker.func_wrap(
                "wasi_snapshot_preview1",
                "fd_seek",
                |mut caller: wasmtime::Caller<'_, SandboxState>,
                 _fd: i32,
                 _offset: i64,
                 _whence: i32,
                 _newoffset: i32|
                 -> i32 {
                    let state = caller.data_mut();
                    let err = take_syscall(state);
                    if err != WASI_ESUCCESS {
                        return err;
                    }
                    let err = require_cap(state, "stdio");
                    if err != WASI_ESUCCESS {
                        return err;
                    }
                    WASI_EBADF
                },
            );
            let _ = linker.func_wrap(
                "wasi_snapshot_preview1",
                "fd_fdstat_get",
                |mut caller: wasmtime::Caller<'_, SandboxState>, _fd: i32, _stat: i32| -> i32 {
                    let state = caller.data_mut();
                    let err = take_syscall(state);
                    if err != WASI_ESUCCESS {
                        return err;
                    }
                    let err = require_cap(state, "stdio");
                    if err != WASI_ESUCCESS {
                        return err;
                    }
                    WASI_EBADF
                },
            );
            let _ = linker.func_wrap(
                "wasi_snapshot_preview1",
                "fd_fdstat_set_flags",
                |mut caller: wasmtime::Caller<'_, SandboxState>, _fd: i32, _flags: i32| -> i32 {
                    let state = caller.data_mut();
                    let err = take_syscall(state);
                    if err != WASI_ESUCCESS {
                        return err;
                    }
                    let err = require_cap(state, "stdio");
                    if err != WASI_ESUCCESS {
                        return err;
                    }
                    WASI_EBADF
                },
            );
            let _ = linker.func_wrap(
                "wasi_snapshot_preview1",
                "fd_filestat_get",
                |mut caller: wasmtime::Caller<'_, SandboxState>, _fd: i32, _stat: i32| -> i32 {
                    let state = caller.data_mut();
                    let err = take_syscall(state);
                    if err != WASI_ESUCCESS {
                        return err;
                    }
                    let err = require_cap(state, "stdio");
                    if err != WASI_ESUCCESS {
                        return err;
                    }
                    WASI_EBADF
                },
            );
            let _ = linker.func_wrap(
                "wasi_snapshot_preview1",
                "fd_prestat_get",
                |mut caller: wasmtime::Caller<'_, SandboxState>, _fd: i32, _prestat: i32| -> i32 {
                    let state = caller.data_mut();
                    let err = take_syscall(state);
                    if err != WASI_ESUCCESS {
                        return err;
                    }
                    WASI_NOTCAPABLE
                },
            );
            let _ = linker.func_wrap(
                "wasi_snapshot_preview1",
                "fd_prestat_dir_name",
                |mut caller: wasmtime::Caller<'_, SandboxState>,
                 _fd: i32,
                 _path: i32,
                 _len: i32|
                 -> i32 {
                    let state = caller.data_mut();
                    let err = take_syscall(state);
                    if err != WASI_ESUCCESS {
                        return err;
                    }
                    WASI_NOTCAPABLE
                },
            );
            let _ = linker.func_wrap(
                "wasi_snapshot_preview1",
                "path_open",
                |mut caller: wasmtime::Caller<'_, SandboxState>,
                 _: i32,
                 _: i32,
                 _: i32,
                 _: i32,
                 _: i32,
                 _: i64,
                 _: i64,
                 _: i32,
                 _: i32|
                 -> i32 {
                    let state = caller.data_mut();
                    let err = take_syscall(state);
                    if err != WASI_ESUCCESS {
                        return err;
                    }
                    if state.capabilities.has("filesystem-read")
                        || state.capabilities.has("filesystem-write")
                    {
                        return WASI_NOSYS;
                    }
                    WASI_NOTCAPABLE
                },
            );
            let _ = linker.func_wrap(
                "wasi_snapshot_preview1",
                "environ_sizes_get",
                |mut caller: wasmtime::Caller<'_, SandboxState>, _count: i32, _size: i32| -> i32 {
                    let err = take_syscall(caller.data_mut());
                    if err != WASI_ESUCCESS {
                        return err;
                    }
                    WASI_ESUCCESS
                },
            );
            let _ = linker.func_wrap(
                "wasi_snapshot_preview1",
                "environ_get",
                |mut caller: wasmtime::Caller<'_, SandboxState>, _environ: i32, _buf: i32| -> i32 {
                    let err = take_syscall(caller.data_mut());
                    if err != WASI_ESUCCESS {
                        return err;
                    }
                    WASI_ESUCCESS
                },
            );
            let _ = linker.func_wrap(
                "wasi_snapshot_preview1",
                "args_sizes_get",
                |mut caller: wasmtime::Caller<'_, SandboxState>, _argc: i32, _size: i32| -> i32 {
                    let err = take_syscall(caller.data_mut());
                    if err != WASI_ESUCCESS {
                        return err;
                    }
                    WASI_ESUCCESS
                },
            );
            let _ = linker.func_wrap(
                "wasi_snapshot_preview1",
                "args_get",
                |mut caller: wasmtime::Caller<'_, SandboxState>, _argv: i32, _buf: i32| -> i32 {
                    let err = take_syscall(caller.data_mut());
                    if err != WASI_ESUCCESS {
                        return err;
                    }
                    WASI_ESUCCESS
                },
            );
            let _ = linker.func_wrap(
                "wasi_snapshot_preview1",
                "clock_time_get",
                |mut caller: wasmtime::Caller<'_, SandboxState>,
                 _id: i32,
                 _prec: i64,
                 time_ptr: i32|
                 -> i32 {
                    {
                        let state = caller.data_mut();
                        let err = take_syscall(state);
                        if err != WASI_ESUCCESS {
                            return err;
                        }
                        let err = require_cap(state, "clock");
                        if err != WASI_ESUCCESS {
                            return err;
                        }
                    }
                    let Some(memory) = guest_memory(&mut caller) else {
                        return WASI_EBADF;
                    };
                    let nanos = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos() as u64)
                        .unwrap_or(0);
                    let ptr = time_ptr as usize;
                    let data = memory.data_mut(&mut caller);
                    if ptr.saturating_add(8) > data.len() {
                        return WASI_EFAULT;
                    }
                    data[ptr..ptr + 8].copy_from_slice(&nanos.to_le_bytes());
                    WASI_ESUCCESS
                },
            );
            let _ = linker.func_wrap(
                "wasi_snapshot_preview1",
                "clock_res_get",
                |mut caller: wasmtime::Caller<'_, SandboxState>, _id: i32, _res: i32| -> i32 {
                    let state = caller.data_mut();
                    let err = take_syscall(state);
                    if err != WASI_ESUCCESS {
                        return err;
                    }
                    require_cap(state, "clock")
                },
            );
            let _ = linker.func_wrap(
                "wasi_snapshot_preview1",
                "random_get",
                |mut caller: wasmtime::Caller<'_, SandboxState>, buf: i32, len: i32| -> i32 {
                    {
                        let state = caller.data_mut();
                        let err = take_syscall(state);
                        if err != WASI_ESUCCESS {
                            return err;
                        }
                        let err = require_cap(state, "random");
                        if err != WASI_ESUCCESS {
                            return err;
                        }
                    }
                    let Some(memory) = guest_memory(&mut caller) else {
                        return WASI_EBADF;
                    };
                    let ptr = buf as usize;
                    let n = (len.max(0)) as usize;
                    let data = memory.data_mut(&mut caller);
                    if ptr.saturating_add(n) > data.len() {
                        return WASI_EFAULT;
                    }
                    for b in data[ptr..ptr + n].iter_mut() {
                        *b = 0xA5;
                    }
                    WASI_ESUCCESS
                },
            );
            let _ = linker.func_wrap(
                "wasi_snapshot_preview1",
                "sched_yield",
                |mut caller: wasmtime::Caller<'_, SandboxState>| -> i32 {
                    take_syscall(caller.data_mut())
                },
            );
            let _ = linker.func_wrap(
                "wasi_snapshot_preview1",
                "poll_oneoff",
                |mut caller: wasmtime::Caller<'_, SandboxState>,
                 _in: i32,
                 _out: i32,
                 _nsubs: i32,
                 _nevents: i32|
                 -> i32 {
                    let err = take_syscall(caller.data_mut());
                    if err != WASI_ESUCCESS {
                        return err;
                    }
                    WASI_NOSYS
                },
            );
            let _ = linker.func_wrap(
                "wasi_snapshot_preview1",
                "proc_exit",
                |mut caller: wasmtime::Caller<'_, SandboxState>,
                 code: i32|
                 -> Result<(), anyhow::Error> {
                    caller.data_mut().exit_code = Some(code);
                    anyhow::bail!("wasm-sandbox-proc-exit:{}", code)
                },
            );

            // ── Instantiate and run ──────────────────────────────────────
            let instance = linker
                .instantiate(&mut store, &module)
                .map_err(|e| SandboxError::ExecutionError(e.to_string()))?;

            let func = instance
                .get_typed_func::<(), i32>(&mut store, "_start")
                .or_else(|_| instance.get_typed_func::<(), i32>(&mut store, "main"))
                .map_err(|e| {
                    SandboxError::ExecutionError(format!("No entry point (_start/main): {}", e))
                })?;

            let call_result = func.call(&mut store, ()).map_err(|e| {
                // Wasmtime attaches its WebAssembly backtrace as the outer
                // error context. Displaying that context can hide the actual
                // trap name, so classify fuel exhaustion and epoch interrupts
                // from the typed trap instead of relying only on formatted text.
                match e.downcast_ref::<wasmtime::Trap>() {
                    Some(wasmtime::Trap::OutOfFuel) => {
                        SandboxError::ResourceLimitExceeded(format!(
                            "Fuel limit exceeded (limit: {} fuel units): {}",
                            max_fuel, e
                        ))
                    }
                    Some(wasmtime::Trap::Interrupt) => SandboxError::Timeout(timeout_duration),
                    _ => SandboxError::ExecutionError(e.to_string()),
                }
            });

            // Measure peak memory
            let memories: Vec<wasmtime::Memory> = instance
                .exports(&mut store)
                .filter_map(|exp| exp.into_memory())
                .collect();
            let peak_memory: usize = memories.iter().map(|mem| mem.data_size(&store)).sum();
            let wall_time_ms = started.elapsed().as_millis() as u64;

            match call_result {
                Ok(exit_code) => Ok(SandboxResult {
                    success: exit_code == 0,
                    exit_code,
                    stdout: store.data().stdout.clone(),
                    stderr: store.data().stderr.clone(),
                    stdout_truncated: store.data().stdout_truncated,
                    stderr_truncated: store.data().stderr_truncated,
                    resource_usage: ResourceUsage {
                        peak_memory,
                        cpu_time_ms: 0,
                        wall_time_ms,
                    },
                    findings: vec![],
                }),
                Err(SandboxError::ExecutionError(ref msg))
                    if msg.contains("wasm-sandbox-proc-exit:") =>
                {
                    let code = store.data().exit_code.unwrap_or(0);
                    Ok(SandboxResult {
                        success: code == 0,
                        exit_code: code,
                        stdout: store.data().stdout.clone(),
                        stderr: store.data().stderr.clone(),
                        stdout_truncated: store.data().stdout_truncated,
                        stderr_truncated: store.data().stderr_truncated,
                        resource_usage: ResourceUsage {
                            peak_memory,
                            cpu_time_ms: 0,
                            wall_time_ms,
                        },
                        findings: vec![],
                    })
                }
                Err(SandboxError::ExecutionError(ref msg))
                    if msg.contains("all fuel consumed") || msg.to_lowercase().contains("fuel") =>
                {
                    Err(SandboxError::ResourceLimitExceeded(format!(
                        "Fuel limit exceeded (limit: {} fuel units): {}",
                        max_fuel, msg
                    )))
                }
                Err(SandboxError::ExecutionError(ref msg)) if msg.contains("memory") => {
                    Err(SandboxError::ResourceLimitExceeded(format!(
                        "Memory limit exceeded (limit: {} bytes): {}",
                        memory_limit, msg
                    )))
                }
                Err(e) => Err(e),
            }
        });

        // Wait slightly longer than `timeout_duration` so Wasmtime's epoch
        // interrupt cleanly unwinds the blocking thread back to `Trap::Interrupt`
        // without leaving an orphaned blocking thread behind.
        let hard_deadline = timeout_duration.saturating_add(Duration::from_millis(500));
        let outcome = match tokio::time::timeout(hard_deadline, handle).await {
            Ok(Ok(result)) => result,
            Ok(Err(join_err)) => {
                // JoinError — the blocking task panicked
                Err(SandboxError::ExecutionError(format!(
                    "WASM execution task failed: {}",
                    join_err
                )))
            }
            Err(_elapsed) => {
                warn!("WASM execution timed out (limit: {:?})", timeout_duration);
                Err(SandboxError::Timeout(timeout_duration))
            }
        };
        watchdog_done.store(true, Ordering::Release);
        watchdog.abort();
        outcome
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
        assert_eq!(config.limits.max_memory, 1024);
        assert_eq!(config.limits.max_time_ms, 10_000);
    }

    #[test]
    fn test_sandbox_input() {
        let input = SandboxInput::new("test-pkg", "1.0.0", "npm");

        assert_eq!(input.package_name, "test-pkg");
        assert_eq!(input.ecosystem, "npm");
    }

    fn fd_write_wat() -> &'static str {
        r#"(module
            (import "wasi_snapshot_preview1" "fd_write"
                (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 8) "hi")
            (func (export "_start") (result i32)
                (i32.store (i32.const 0) (i32.const 8))
                (i32.store (i32.const 4) (i32.const 2))
                (drop (call $fd_write
                    (i32.const 1)
                    (i32.const 0)
                    (i32.const 1)
                    (i32.const 16)))
                (i32.const 0)
            )
        )"#
    }

    #[tokio::test]
    async fn test_fd_write_captures_stdout_when_stdio_granted() {
        let wasm = wat::parse_str(fd_write_wat()).expect("wat");
        let sandbox = WasmSandbox::new(SandboxConfig::default()).expect("sandbox");
        let result = sandbox
            .run(&wasm, &SandboxInput::new("t", "1", "wasm"))
            .await
            .expect("run");
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "hi");
        assert!(!result.stdout_truncated);
    }

    #[tokio::test]
    async fn test_fd_write_denied_without_stdio() {
        let wasm = wat::parse_str(fd_write_wat()).expect("wat");
        let sandbox =
            WasmSandbox::new(SandboxConfig::default().with_capabilities(CapabilitySet::empty()))
                .expect("sandbox");
        let result = sandbox
            .run(&wasm, &SandboxInput::new("t", "1", "wasm"))
            .await
            .expect("run");
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.is_empty());
    }

    #[tokio::test]
    async fn test_fuel_limit_exceeded() {
        // Infinite loop WASM module — must hit OutOfFuel on Windows and Linux.
        let wat = r#"(module
            (func (export "_start") (result i32)
                (loop $infinite (br $infinite))
                (i32.const 0)
            )
        )"#;
        let wasm = wat::parse_str(wat).expect("wat");
        let limits = ResourceLimits::default().with_fuel(10_000);
        let sandbox = WasmSandbox::new(SandboxConfig::default().with_resource_limits(limits))
            .expect("sandbox");
        let err = sandbox
            .run(&wasm, &SandboxInput::new("loop-test", "1.0.0", "wasm"))
            .await
            .unwrap_err();

        match err {
            SandboxError::ResourceLimitExceeded(msg) => {
                assert!(
                    msg.to_lowercase().contains("fuel"),
                    "expected fuel error, got: {}",
                    msg
                );
            }
            other => panic!("expected ResourceLimitExceeded, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_epoch_timeout_interrupts_infinite_loop() {
        let wat = r#"(module
            (func (export "_start") (result i32)
                (loop $infinite (br $infinite))
                (i32.const 0)
            )
        )"#;
        let wasm = wat::parse_str(wat).expect("wat");
        // Huge fuel budget so epoch timeout triggers before fuel exhaustion.
        let limits = ResourceLimits::default()
            .with_fuel(u64::MAX / 4)
            .with_time_ms(1_000);
        let sandbox = WasmSandbox::new(
            SandboxConfig::default()
                .with_resource_limits(limits)
                .with_timeout_secs(1),
        )
        .expect("sandbox");
        let err = sandbox
            .run(&wasm, &SandboxInput::new("epoch-loop", "1.0.0", "wasm"))
            .await
            .unwrap_err();

        match err {
            SandboxError::Timeout(_) => {}
            other => panic!("expected Timeout via epoch interruption, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_fd_write_caps_stdio_at_64kib() {
        // Write a 1024-byte buffer 128 times (128 KiB attempted > 64 KiB cap).
        let wat = r#"(module
            (import "wasi_snapshot_preview1" "fd_write"
                (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 2)
            (func (export "_start") (result i32)
                (local $i i32)
                (i32.store (i32.const 0) (i32.const 1024))
                (i32.store (i32.const 4) (i32.const 1024))
                (local.set $i (i32.const 0))
                (loop $w
                    (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 16)))
                    (local.set $i (i32.add (local.get $i) (i32.const 1)))
                    (br_if $w (i32.lt_u (local.get $i) (i32.const 128)))
                )
                (i32.const 0)
            )
        )"#;
        let wasm = wat::parse_str(wat).expect("wat");
        let sandbox = WasmSandbox::new(SandboxConfig::default()).expect("sandbox");
        let result = sandbox
            .run(&wasm, &SandboxInput::new("stdio-flood", "1.0.0", "wasm"))
            .await
            .expect("run should succeed cleanly with bounded stdout");
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.len(), MAX_WASM_STDIO_BYTES);
        assert!(result.stdout_truncated);
        assert!(!result.stderr_truncated);
    }

    #[tokio::test]
    async fn test_syscall_budget_enforced() {
        let wat = r#"(module
            (import "wasi_snapshot_preview1" "sched_yield" (func $yield (result i32)))
            (func (export "_start") (result i32)
                (drop (call $yield))
                (drop (call $yield))
                (i32.const 0)
            )
        )"#;
        let wasm = wat::parse_str(wat).expect("wat");
        let limits = ResourceLimits::default().with_fuel(100_000);
        let sandbox = WasmSandbox::new(SandboxConfig::default().with_resource_limits(limits))
            .expect("sandbox");
        let result = sandbox
            .run(&wasm, &SandboxInput::new("syscall-test", "1.0.0", "wasm"))
            .await
            .expect("run");
        assert_eq!(result.exit_code, 0);
    }
}

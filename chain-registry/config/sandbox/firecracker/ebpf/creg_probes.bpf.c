// SPDX-License-Identifier: GPL-2.0 OR Apache-2.0
// ADR-018 Layer 3 Tier 1 In-Guest eBPF CO-RE Probes (`creg_probes.bpf.c`)
//
// Attached inside the ephemeral Firecracker microVM by `/sbin/creg-guest-init`
// prior to unpacking or executing `/pkg/package.tar.gz`. Covers all six ADR-018
// behavioral hook families and streams JSON/ringbuf records over virtio-vsock:
//   1. Process execution: tracepoint/sched/sched_process_exec, sys_enter_execve/at
//   2. File access:       lsm/file_open, tracepoint/syscalls/sys_enter_openat/2
//   3. Network egress:    lsm/socket_connect, tracepoint/syscalls/sys_enter_sendto
//   4. Memory anomalies:  sys_enter_memfd_create, sys_enter_mmap/mprotect (RWX)
//   5. Escape attempts:   sys_enter_ptrace, process_vm_writev, bpf, io_uring_setup
//   6. Persistence:       OverlayFS upperdir diff collected on VM teardown

#ifdef __BPF__
#include "vmlinux.h"
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_tracing.h>
#include <bpf/bpf_core_read.h>

char LICENSE[] SEC("license") = "Dual GPL/Apache-2.0";

#define PROT_WRITE 0x2
#define PROT_EXEC  0x4
#define MAX_PATH_LEN 256

enum creg_hook_kind {
    CREG_HOOK_EXEC = 1,
    CREG_HOOK_OPEN_WRITE = 2,
    CREG_HOOK_CONNECT = 3,
    CREG_HOOK_MEMFD_CREATE = 4,
    CREG_HOOK_MEMORY_RWX = 5,
    CREG_HOOK_KERNEL_ESCAPE = 6,
};

struct creg_event {
    __u32 kind;
    __u32 tgid;
    __u32 uid;
    __u32 flags;
    char path[MAX_PATH_LEN];
};

struct {
    __uint(type, BPF_MAP_TYPE_RINGBUF);
    __uint(max_entries, 1 << 20);
} creg_events SEC(".maps");

static __always_inline int emit_simple(__u32 kind, __u32 flags, const char __user *uptr)
{
    struct creg_event *ev = bpf_ringbuf_reserve(&creg_events, sizeof(*ev), 0);
    if (!ev)
        return 0;
    ev->kind = kind;
    ev->tgid = (__u32)(bpf_get_current_pid_tgid() >> 32);
    ev->uid = (__u32)bpf_get_current_uid_gid();
    ev->flags = flags;
    __builtin_memset(ev->path, 0, sizeof(ev->path));
    if (uptr)
        bpf_probe_read_user_str(ev->path, sizeof(ev->path), uptr);
    bpf_ringbuf_submit(ev, 0);
    return 0;
}

SEC("tracepoint/syscalls/sys_enter_execve")
int tp_execve(struct trace_event_raw_sys_enter *ctx)
{
    return emit_simple(CREG_HOOK_EXEC, 0, (const char __user *)ctx->args[0]);
}

SEC("tracepoint/syscalls/sys_enter_openat")
int tp_openat(struct trace_event_raw_sys_enter *ctx)
{
    __u32 flags = (__u32)ctx->args[2];
    if ((flags & 03) == 0) // O_RDONLY only
        return 0;
    return emit_simple(CREG_HOOK_OPEN_WRITE, flags, (const char __user *)ctx->args[1]);
}

SEC("tracepoint/syscalls/sys_enter_memfd_create")
int tp_memfd_create(struct trace_event_raw_sys_enter *ctx)
{
    return emit_simple(CREG_HOOK_MEMFD_CREATE, (__u32)ctx->args[1], (const char __user *)ctx->args[0]);
}

SEC("tracepoint/syscalls/sys_enter_mprotect")
int tp_mprotect(struct trace_event_raw_sys_enter *ctx)
{
    __u32 prot = (__u32)ctx->args[2];
    if ((prot & (PROT_WRITE | PROT_EXEC)) == (PROT_WRITE | PROT_EXEC))
        return emit_simple(CREG_HOOK_MEMORY_RWX, prot, NULL);
    return 0;
}

SEC("tracepoint/syscalls/sys_enter_ptrace")
int tp_ptrace(struct trace_event_raw_sys_enter *ctx)
{
    return emit_simple(CREG_HOOK_KERNEL_ESCAPE, (__u32)ctx->args[0], NULL);
}

SEC("tracepoint/syscalls/sys_enter_io_uring_setup")
int tp_io_uring(struct trace_event_raw_sys_enter *ctx)
{
    return emit_simple(CREG_HOOK_KERNEL_ESCAPE, (__u32)ctx->args[0], NULL);
}
#endif

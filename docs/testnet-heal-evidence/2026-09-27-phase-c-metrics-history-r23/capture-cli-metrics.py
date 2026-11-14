#!/usr/bin/env python3
import datetime
import fcntl
import json
import os
import pathlib
import pty
import re
import select
import signal
import struct
import subprocess
import termios
import time

container = "creg-phase-c-r7-o1"
output = bytearray()
master, slave = pty.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 140, 0, 0))
env = os.environ.copy()
env["TERM"] = "xterm-256color"
env["LANG"] = "C.UTF-8"
command = ["docker", "exec", "-it", container, "/app/creg", "--node-url", "http://127.0.0.1:8080", "console"]
pid = os.fork()
if pid == 0:
    os.close(master)
    os.dup2(slave, 0)
    os.dup2(slave, 1)
    os.dup2(slave, 2)
    if slave > 2:
        os.close(slave)
    os.execvpe(command[0], command, env)
os.close(slave)

def drain(seconds):
    deadline = time.time() + seconds
    while time.time() < deadline:
        if os.waitpid(pid, os.WNOHANG)[0] == pid:
            break
        readable, _, _ = select.select([master], [], [], min(0.2, max(0, deadline - time.time())))
        if readable:
            try:
                chunk = os.read(master, 65536)
            except OSError:
                break
            if not chunk:
                break
            output.extend(chunk)

drain(8)
os.write(master, b"m")
drain(3)
os.write(master, b"m")
drain(3)
os.write(master, b"q")
drain(0.4)
os.write(master, b"q")
drain(2)
status = os.waitpid(pid, os.WNOHANG)
if status[0] == 0:
    os.kill(pid, signal.SIGTERM)
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass
os.close(master)
raw = bytes(output)
text = re.sub(rb"\x1b\][^\x07]*(?:\x07|\x1b\\)", b"", raw)
text = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", text)
text = text.replace(b"\r", b"\n").replace(b"\x00", b"")
clean = text.decode("utf-8", errors="replace")
report = {
    "captured_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "container": container,
    "command": command,
    "terminal_size": {"columns": 140, "rows": 40},
    "metrics_view_rendered": "retained chain height" in clean and "Prometheus history: ready" in clean,
    "prometheus_ready_visible": "Prometheus history: ready" in clean,
    "retained_samples_visible": "retained samples" in clean,
    "retained_samples_count": int(re.search(r"retained samples\s*(\d+)", clean).group(1)) if re.search(r"retained samples\s*(\d+)", clean) else None,
    "chain_height_14_visible": bool(re.search(r"Tip\s*14\b", clean)),
    "transcript_sha256": __import__("hashlib").sha256(raw).hexdigest(),
    "transcript_bytes": len(raw),
    "clean_transcript": clean[-12000:],
}
evidence = pathlib.Path("/var/lib/creg/phase-c-r7/evidence")
evidence.mkdir(parents=True, exist_ok=True)
(evidence / "r23-o1-cli-metrics.ansi").write_bytes(raw)
(evidence / "r23-o1-cli-metrics.txt").write_text(clean, encoding="utf-8")
(evidence / "r23-o1-cli-metrics.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
print(json.dumps({key: report[key] for key in ("captured_at_utc", "container", "metrics_view_rendered", "prometheus_ready_visible", "retained_samples_visible", "retained_samples_count", "chain_height_14_visible", "transcript_sha256", "transcript_bytes")}, separators=(",", ":")))
#!/usr/bin/env python3
"""
stress-test.py

Stress-tests the Chain Registry testnet by publishing dummy packages
and measuring consensus latency.

Usage:
  # Full testnet must be running first
  python scripts/stress-test.py --nodes 10 --packages 1000 --concurrency 20

Output:
  - Terminal summary with throughput and latency percentiles
  - stress-test-report.json with full metrics
"""

import argparse
import asyncio
import hashlib
import io
import json
import os
import random
import sys
import tarfile
import time
from dataclasses import dataclass, field, asdict
from datetime import datetime
from pathlib import Path
from typing import Optional

import aiohttp

try:
    from nacl.signing import SigningKey
    from nacl.encoding import HexEncoder
    NACL_AVAILABLE = True
except ImportError:
    NACL_AVAILABLE = False
    print("Warning: pynacl not installed. Run: pip install pynacl")


@dataclass
class PublishResult:
    canonical: str
    submitted_at: float
    accepted_at: Optional[float] = None
    verified_at: Optional[float] = None
    error: Optional[str] = None
    status_checks: int = 0


@dataclass
class StressReport:
    total_packages: int
    successful_submissions: int = 0
    verified_packages: int = 0
    failed_submissions: int = 0
    timed_out: int = 0
    results: list = field(default_factory=list)
    p50_latency_ms: float = 0.0
    p95_latency_ms: float = 0.0
    p99_latency_ms: float = 0.0
    throughput_pkgs_per_sec: float = 0.0
    start_time: str = ""
    end_time: str = ""


def create_dummy_tarball(name: str, version: str) -> bytes:
    """Create a minimal tarball that looks like a real package."""
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tar:
        # Add a package.json / setup.py style manifest
        manifest = json.dumps({
            "name": name,
            "version": version,
            "description": f"Stress test package {name}@{version}",
        }).encode()
        info = tarfile.TarInfo(name="package.json")
        info.size = len(manifest)
        tar.addfile(info, io.BytesIO(manifest))

        # Add a small source file
        code = f"// {name} v{version}\nconsole.log('hello');\n".encode()
        info2 = tarfile.TarInfo(name="index.js")
        info2.size = len(code)
        tar.addfile(info2, io.BytesIO(code))
    return buf.getvalue()


async def ipfs_upload(session: aiohttp.ClientSession, data: bytes) -> str:
    """Upload bytes to IPFS and return the CID."""
    # In Docker mode, use the internal IPFS URL
    docker_mode = os.getenv("DOCKER_MODE", "false").lower() == "true"
    if docker_mode:
        ipfs_url = os.getenv("CREG_IPFS_URL", "http://creg-testnet-ipfs:5001")
    else:
        ipfs_url = os.getenv("CREG_IPFS_URL", "http://localhost:5001")
    form = aiohttp.FormData()
    form.add_field("file", data, filename="package.tgz")
    async with session.post(f"{ipfs_url}/api/v0/add", data=form) as resp:
        resp.raise_for_status()
        result = await resp.json()
        return result["Hash"]


def sign_publish_request(canonical: str, content_hash: str, publisher_key_hex: str) -> str:
    """Sign the publish request using Ed25519."""
    if not NACL_AVAILABLE or not publisher_key_hex:
        return "00" * 64  # fallback dummy
    
    try:
        # The signed message is: canonical + content_hash
        # e.g., "npm:stress-pkg-0001@1.2.3<sha256_hash>"
        message = f"{canonical}{content_hash}".encode('utf-8')
        
        # Load the signing key from hex
        sk = SigningKey(publisher_key_hex, encoder=HexEncoder)
        signed = sk.sign(message)
        # signed.signature is the actual signature bytes
        return signed.signature.hex()
    except Exception as e:
        print(f"Warning: Failed to sign request: {e}")
        return "00" * 64


async def submit_package(
    session: aiohttp.ClientSession,
    node_url: str,
    publisher_key: str,
    publisher_pubkey: str,
    canonical: str,
    content_hash: str,
    ipfs_cid: str,
) -> PublishResult:
    """Submit a PublishRequest to a node API."""
    result = PublishResult(canonical=canonical, submitted_at=time.time())

    # Generate proper Ed25519 signature
    signature = sign_publish_request(canonical, content_hash, publisher_key)

    payload = {
        "id": {
            "ecosystem": "npm",
            "name": canonical.split(":")[1].split("@")[0],
            "version": canonical.split("@")[1],
        },
        "content_hash": content_hash,
        "ipfs_cid": ipfs_cid,
        "publisher_pubkey": publisher_pubkey,
        "signature": signature,  # Proper Ed25519 signature
        "manifest": {
            "allowed_network_hosts": [],
            "allowed_fs_writes": [],
            "spawns_processes": False,
        },
        "submitted_at": datetime.utcnow().isoformat() + "Z",
        "shielded": False,
        "key_bundle": None,
    }

    try:
        async with session.post(
            f"{node_url}/v1/packages", json=payload, timeout=aiohttp.ClientTimeout(total=30)
        ) as resp:
            if resp.status == 202:
                result.accepted_at = time.time()
            else:
                text = await resp.text()
                result.error = f"HTTP {resp.status}: {text[:200]}"
    except Exception as e:
        result.error = str(e)

    return result


async def wait_for_verification(
    session: aiohttp.ClientSession,
    node_url: str,
    canonical: str,
    result: PublishResult,
    max_wait: float = 60.0,
    poll_interval: float = 1.0,
) -> None:
    """Poll the package endpoint until it is verified or timeout."""
    deadline = time.time() + max_wait
    while time.time() < deadline:
        try:
            async with session.get(
                f"{node_url}/v1/packages/{canonical}",
                timeout=aiohttp.ClientTimeout(total=10),
            ) as resp:
                if resp.status == 200:
                    data = await resp.json()
                    result.status_checks += 1
                    if data.get("status") == "verified":
                        result.verified_at = time.time()
                        return
                    elif data.get("status") == "revoked":
                        result.error = "Package was revoked"
                        return
                await asyncio.sleep(poll_interval)
        except Exception:
            await asyncio.sleep(poll_interval)
    result.error = result.error or "Timeout waiting for verification"


async def producer(
    queue: asyncio.Queue,
    total: int,
    publisher_pubkey: str,
) -> None:
    """Generate dummy packages and put them on the queue."""
    for i in range(total):
        name = f"stress-pkg-{i:04d}"
        version = f"{random.randint(1, 9)}.{random.randint(0, 9)}.{random.randint(0, 9)}"
        canonical = f"npm:{name}@{version}"
        tarball = create_dummy_tarball(name, version)
        content_hash = hashlib.sha256(tarball).hexdigest()
        await queue.put((canonical, content_hash, tarball))


async def consumer(
    queue: asyncio.Queue,
    session: aiohttp.ClientSession,
    node_urls: list[str],
    publisher_key: str,
    publisher_pubkey: str,
    results: list,
    max_wait: float,
) -> None:
    """Consume packages from the queue, upload to IPFS, submit, and verify."""
    while True:
        item = await queue.get()
        if item is None:
            queue.task_done()
            break

        canonical, content_hash, tarball = item
        node_url = random.choice(node_urls)

        try:
            ipfs_cid = await ipfs_upload(session, tarball)
            result = await submit_package(
                session, node_url, publisher_key, publisher_pubkey,
                canonical, content_hash, ipfs_cid
            )
            if result.accepted_at:
                await wait_for_verification(session, node_url, canonical, result, max_wait)
            results.append(result)
        except Exception as e:
            results.append(PublishResult(
                canonical=canonical,
                submitted_at=time.time(),
                error=str(e),
            ))
        finally:
            queue.task_done()


def compute_percentile(values: list[float], p: float) -> float:
    if not values:
        return 0.0
    s = sorted(values)
    k = (len(s) - 1) * p / 100.0
    f = int(k)
    c = min(f + 1, len(s) - 1)
    return s[f] + (s[c] - s[f]) * (k - f)


async def discover_live_nodes(session, node_urls):
    """Health-check pre-flight: only route to nodes that are actually alive."""
    live = []
    for url in node_urls:
        try:
            async with session.get(f"{url}/v1/health", timeout=aiohttp.ClientTimeout(total=3)) as resp:
                if resp.status == 200:
                    live.append(url)
        except Exception:
            pass
    return live


async def run_stress_test(args) -> StressReport:
    # Support both Docker network hostnames and localhost (for external testing)
    docker_mode = os.getenv("DOCKER_MODE", "false").lower() == "true"
    if docker_mode:
        # Use Docker service names for internal Docker networking
        node_urls = [f"http://creg-testnet-node-{i}:8080" for i in range(1, args.nodes + 1)]
    else:
        # Use localhost for external testing
        node_urls = [f"http://localhost:{8080 + i if i > 0 else 8080}" for i in range(args.nodes)]
    
    publisher_key = os.getenv("TESTNET_PUBLISHER_KEY", "")
    publisher_pubkey = os.getenv("TESTNET_PUBLISHER_PUBKEY", "")

    if not publisher_key or not publisher_pubkey:
        print("WARNING: TESTNET_PUBLISHER_KEY / TESTNET_PUBLISHER_PUBKEY not set.")
        print("Run: python scripts/generate-testnet-keys.py")
        # Continue with dummy values — submission will fail but we can test API load

    report = StressReport(total_packages=args.packages)
    report.start_time = datetime.utcnow().isoformat() + "Z"

    queue: asyncio.Queue = asyncio.Queue(maxsize=args.concurrency * 2)
    results: list[PublishResult] = []

    async with aiohttp.ClientSession() as session:
        # Discover live nodes before sending traffic
        live_nodes = await discover_live_nodes(session, node_urls)
        if not live_nodes:
            print("ERROR: No live nodes found! Aborting stress test.")
            print(f"  Checked URLs: {node_urls}")
            report.end_time = datetime.utcnow().isoformat() + "Z"
            return report
        print(f"Discovered {len(live_nodes)}/{len(node_urls)} live nodes: {live_nodes}")

        # Start consumers
        consumers = [
            asyncio.create_task(consumer(
                queue, session, live_nodes, publisher_key, publisher_pubkey, results, args.timeout
            ))
            for _ in range(args.concurrency)
        ]

        # Produce work
        await producer(queue, args.packages, publisher_pubkey)

        # Signal end of work
        for _ in range(args.concurrency):
            await queue.put(None)

        await asyncio.gather(*consumers)

    report.end_time = datetime.utcnow().isoformat() + "Z"
    report.results = [asdict(r) for r in results]

    # Compute metrics
    latencies = []
    for r in results:
        if r.verified_at and r.submitted_at:
            latencies.append((r.verified_at - r.submitted_at) * 1000)
            report.verified_packages += 1
        if r.accepted_at:
            report.successful_submissions += 1
        if r.error:
            if "Timeout" in r.error:
                report.timed_out += 1
            else:
                report.failed_submissions += 1

    if latencies:
        report.p50_latency_ms = compute_percentile(latencies, 50)
        report.p95_latency_ms = compute_percentile(latencies, 95)
        report.p99_latency_ms = compute_percentile(latencies, 99)

    total_duration = sum(latencies) / 1000.0 if latencies else 1.0
    report.throughput_pkgs_per_sec = report.verified_packages / total_duration if total_duration > 0 else 0.0

    return report


def print_report(report: StressReport) -> None:
    print("\n" + "=" * 60)
    print("  Chain Registry Testnet Stress Test Report")
    print("=" * 60)
    print(f"  Total packages submitted:     {report.total_packages}")
    print(f"  Accepted by API:              {report.successful_submissions}")
    print(f"  Verified by consensus:        {report.verified_packages}")
    print(f"  Failed submissions:           {report.failed_submissions}")
    print(f"  Timed out (>{args.timeout}s): {report.timed_out}")
    print(f"  Verification rate:            {report.verified_packages / report.total_packages * 100:.1f}%")
    print()
    print(f"  P50 consensus latency:        {report.p50_latency_ms:.0f} ms")
    print(f"  P95 consensus latency:        {report.p95_latency_ms:.0f} ms")
    print(f"  P99 consensus latency:        {report.p99_latency_ms:.0f} ms")
    print(f"  Throughput:                   {report.throughput_pkgs_per_sec:.2f} pkg/s")
    print("=" * 60 + "\n")


def main():
    global args
    parser = argparse.ArgumentParser(description="Chain Registry testnet stress test")
    parser.add_argument("--nodes", type=int, default=10, help="Number of validator nodes")
    parser.add_argument("--packages", type=int, default=1000, help="Total packages to publish")
    parser.add_argument("--concurrency", type=int, default=20, help="Concurrent publish tasks")
    parser.add_argument("--timeout", type=float, default=60.0, help="Max seconds to wait for verification")
    parser.add_argument("--output", type=str, default="stress-test-report.json", help="JSON report path")
    args = parser.parse_args()

    print(f"Starting stress test: {args.packages} packages across {args.nodes} nodes")
    print(f"Concurrency: {args.concurrency}, Verification timeout: {args.timeout}s\n")

    report = asyncio.run(run_stress_test(args))
    print_report(report)

    Path(args.output).write_text(json.dumps(asdict(report), indent=2))
    print(f"Full report written to: {args.output}")


if __name__ == "__main__":
    main()

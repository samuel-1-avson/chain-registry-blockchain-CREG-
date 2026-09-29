#!/usr/bin/env python3
"""Submit one actual 100 tCREG claim to the private R29 candidate on Sepolia."""

import hashlib
import json
import subprocess
import sys
import time
import urllib.request
from pathlib import Path


BASE = "http://127.0.0.1:18093"
RPC = "https://api.testnet.cregnet.dev/sepolia"
TOKEN = "0xbc2a72107c6765364d0a822a88a6e6c61f30eaa6"
FAUCET = "0x721c8dbd8208ca9dbdcc12ae9339225458608b81"
RECIPIENT = "0x67420a75fa0950bb955145f2ff37bfc65cc1b09f"
AMOUNT = 100 * 10**18
PRIVATE_APP = "creg-faucet-r29-private"


def request(url, body=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(
        url, data=data,
        headers={"Content-Type": "application/json", "User-Agent": "creg-r29-real-claim/1"},
    )
    with urllib.request.urlopen(req, timeout=90) as response:
        if response.status != 200:
            raise RuntimeError(f"HTTP {response.status} from {url}")
        return json.load(response)


def rpc(method, params):
    result = request(RPC, {"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
    if "error" in result:
        raise RuntimeError(f"Sepolia RPC failed: {result['error']}")
    return result["result"]


def balance(address):
    data = "0x70a08231" + address[2:].lower().rjust(64, "0")
    return int(rpc("eth_call", [{"to": TOKEN, "data": data}, "latest"]), 16)


def query_db(sql):
    completed = subprocess.run(
        ["sudo", "docker", "exec", "-i", "creg-unified-faucet-db", "sh", "-c",
         'exec psql -X -q -v ON_ERROR_STOP=1 -U "$POSTGRES_USER" -d "$POSTGRES_DB" -At'],
        input=sql, text=True, capture_output=True, timeout=30,
    )
    if completed.returncode != 0:
        raise RuntimeError("Production faucet database read failed: " + completed.stderr.strip())
    return completed.stdout.strip()


def save(path, report):
    path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


def main():
    if len(sys.argv) != 2:
        raise SystemExit("Provide a new claim evidence JSON path")
    out = Path(sys.argv[1])
    if out.exists():
        raise RuntimeError("Claim evidence already exists; do not repeat an uncertain claim")
    stats = request(BASE + "/api/stats")
    health = request(BASE + "/health")
    public = request("https://faucet.testnet.cregnet.dev/api/stats")
    if (stats.get("token_claims_enabled") is not True or
        stats.get("ledger_reconciled") is not True or
        stats.get("token_distribution_open") is not True or
        health.get("status") != "healthy" or
        public.get("token_claims_enabled") is not False or
        stats.get("drip_amount") != str(AMOUNT) or
        int(stats.get("faucet_balance", "-1")) != 100_000 * 10**18):
        raise RuntimeError("Private/public faucet release gates differ from reviewed state")
    if int(rpc("eth_chainId", []), 16) != 11155111:
        raise RuntimeError("RPC is not Sepolia")
    recipient_before = balance(RECIPIENT)
    faucet_before = balance(FAUCET)
    if recipient_before != 0 or faucet_before != 100_000 * 10**18:
        raise RuntimeError("Acceptance recipient or faucet balance changed")
    prior = query_db(
        "SELECT count(*) FROM faucet_claim_ledger WHERE recipient = '" + RECIPIENT + "';\n"
    )
    if prior != "0":
        raise RuntimeError("Acceptance recipient preflight count was " + repr(prior))
    challenge = request(BASE + "/api/challenge")
    threshold = 1 << (256 - int(challenge["difficulty"]))
    payload = None
    for nonce in range(20_000_000):
        digest = hashlib.sha256((challenge["challenge"] + str(nonce)).encode()).digest()
        if int.from_bytes(digest, "big") < threshold:
            payload = {"address": RECIPIENT, "challenge": challenge["challenge"], "nonce": str(nonce)}
            break
    if payload is None:
        raise RuntimeError("Real proof of work challenge was not solved")
    report = {
        "status": "request_prepared_do_not_retry_if_response_uncertain",
        "submitted_at_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "network": "Sepolia", "chain_id": 11155111, "token": TOKEN,
        "faucet": FAUCET, "recipient": RECIPIENT,
        "recipient_key_secret_name": "creg-phase-c-r29-acceptance-evm",
        "recipient_before_wei": str(recipient_before),
        "faucet_before_wei": str(faucet_before),
        "expected_claim_wei": str(AMOUNT),
        "private_candidate_only": True, "public_claims_open": False,
        "requests_mocked": False,
    }
    save(out, report)
    claim = request(BASE + "/api/drip", payload)
    report["claim_response"] = claim
    report["status"] = "claim_response_received"
    save(out, report)
    if claim.get("success") is not True or not claim.get("token_tx_hash"):
        raise RuntimeError("Claim did not return a confirmed token transaction")
    tx_hash = claim["token_tx_hash"]
    receipt = rpc("eth_getTransactionReceipt", [tx_hash])
    if not receipt or int(receipt["status"], 16) != 1:
        raise RuntimeError("Real Sepolia claim receipt is not successful")
    token_logs = [log for log in receipt["logs"] if log["address"].lower() == TOKEN]
    if len(token_logs) != 1 or int(token_logs[0]["data"], 16) != AMOUNT:
        raise RuntimeError("Receipt does not contain exactly one 100 tCREG Transfer")
    recipient_after = balance(RECIPIENT)
    faucet_after = balance(FAUCET)
    if recipient_after - recipient_before != AMOUNT or faucet_before - faucet_after != AMOUNT:
        raise RuntimeError("Real balances differ from one 100 tCREG claim")
    report.update({
        "status": "receipt_and_balances_confirmed", "token_tx_hash": tx_hash,
        "receipt_block_number": int(receipt["blockNumber"], 16),
        "recipient_after_wei": str(recipient_after),
        "faucet_after_wei": str(faucet_after),
    })
    save(out, report)

    subprocess.run(["sudo", "docker", "restart", "--time", "10", PRIVATE_APP],
                   check=True, stdout=subprocess.DEVNULL, timeout=45)
    for _ in range(40):
        try:
            recovered = request(BASE + "/health")
            if recovered.get("status") == "healthy" and recovered.get("ledger_reconciled") is True:
                break
        except Exception:
            pass
        time.sleep(3)
    else:
        raise RuntimeError("Private faucet did not recover after process restart")
    durable = query_db(
        "SELECT count(*)||'|'||COALESCE(max(token_amount_wei)::text,'')||'|'||"
        "COALESCE(max(token_status),'') FROM faucet_claim_ledger WHERE recipient='"
        + RECIPIENT + "' AND token_tx_hash='" + tx_hash.lower() + "';\n"
    )
    if durable != f"1|{AMOUNT}|confirmed":
        raise RuntimeError("Restarted app has no exact durable confirmed claim row: " + durable)
    after_restart = request(BASE + "/api/stats")
    if (after_restart.get("token_claims_enabled") is not True or
        int(after_restart["stats"]["total_distributed"]) != 2_100 * 10**18 or
        int(after_restart["faucet_balance"]) != 99_900 * 10**18 or
        after_restart["stats"]["unresolved_claims"] != 0):
        raise RuntimeError("Post-restart faucet statistics do not match confirmed claim")
    report.update({
        "status": "passed", "private_candidate_restart": "passed",
        "durable_claim_row": durable, "unresolved_claims_after_restart": 0,
        "public_claims_open": False,
    })
    save(out, report)
    print(json.dumps({"status": "passed", "tx_hash": tx_hash,
                      "recipient": RECIPIENT, "claim_tcreg": 100,
                      "private_candidate_restart": "passed"}))


if __name__ == "__main__":
    main()

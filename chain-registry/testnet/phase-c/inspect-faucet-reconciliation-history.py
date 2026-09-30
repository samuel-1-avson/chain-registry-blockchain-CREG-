#!/usr/bin/env python3
"""Compare real Sepolia faucet Transfer logs with balanceOf at a safe block."""

import json
import sys
import time
import urllib.request


def load_env(path):
    result = {}
    with open(path, encoding="utf-8") as stream:
        for raw in stream:
            item = raw.strip()
            if item and not item.startswith("#") and "=" in item:
                key, value = item.split("=", 1)
                result[key] = value
    return result


def quantity(value, label):
    if not isinstance(value, str) or not value.startswith("0x"):
        raise RuntimeError(f"RPC returned invalid {label}")
    return int(value, 16)


env = load_env(sys.argv[1])
rpc_url = env["FAUCET_RECONCILIATION_RPC_URL"]
token = env["FAUCET_TOKEN_CONTRACT"].lower()
faucet = env["FAUCET_ADDRESS"].lower()
deployment = int(env["FAUCET_TOKEN_DEPLOYMENT_BLOCK"])
confirmations = int(env.get("FAUCET_RECONCILIATION_CONFIRMATIONS", "64"))
transfer_topic = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
faucet_topic = "0x" + faucet[2:].rjust(64, "0")
counter = 0


def rpc(method, params, endpoint=None):
    global counter
    counter += 1
    body = json.dumps({"jsonrpc": "2.0", "id": counter, "method": method, "params": params}).encode()
    request = urllib.request.Request(
        endpoint or rpc_url,
        data=body,
        headers={"Content-Type": "application/json", "User-Agent": "curl/7.81.0"},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        data = json.load(response)
    if "error" in data:
        raise RuntimeError(f"{method} failed: {data['error'].get('message', 'RPC error')}")
    return data.get("result")


if len(sys.argv) > 2 and sys.argv[2] == "--blocks":
    for endpoint_name, endpoint in (
        ("configured-primary", env.get("FAUCET_RPC_URL", "")),
        ("publicnode", rpc_url),
        ("one-rpc", "https://1rpc.io/sepolia"),
    ):
        if not endpoint:
            continue
        for block_text in sys.argv[3:]:
            block_number = int(block_text)
            for attempt in range(1, 4):
                try:
                    found = {}
                    for topics in ([transfer_topic, faucet_topic, None], [transfer_topic, None, faucet_topic]):
                        result = rpc("eth_getLogs", [{
                            "address": token,
                            "fromBlock": hex(block_number),
                            "toBlock": hex(block_number),
                            "topics": topics,
                        }], endpoint)
                        for log in result:
                            key = (log["transactionHash"].lower(), quantity(log["logIndex"], "log index"))
                            found[key] = log
                    summary = []
                    for log in found.values():
                        direction = "in" if "0x" + log["topics"][2][-40:].lower() == faucet else "out"
                        summary.append({"direction": direction, "amount_wei": str(quantity(log["data"], "Transfer amount"))})
                    print(json.dumps({"endpoint": endpoint_name, "block": block_number, "attempt": attempt, "events": summary}))
                except Exception as error:
                    code = getattr(error, "code", type(error).__name__)
                    print(json.dumps({"endpoint": endpoint_name, "block": block_number, "attempt": attempt, "error": code}))
    sys.exit(0)

if len(sys.argv) > 2 and sys.argv[2] == "--receipt":
    tx_hash = env["FAUCET_TOKEN_DEPLOYMENT_TX_HASH"]
    for endpoint_name, endpoint in (
        ("configured-primary", env.get("FAUCET_RPC_URL", "")),
        ("publicnode", rpc_url),
        ("one-rpc", "https://1rpc.io/sepolia"),
    ):
        if not endpoint:
            continue
        try:
            receipt = rpc("eth_getTransactionReceipt", [tx_hash], endpoint)
            logs = receipt.get("logs", []) if receipt else []
            transfers = [log for log in logs if log.get("topics", [None])[0] == transfer_topic]
            faucet_mints = [
                log for log in transfers
                if log["topics"][1][-40:] == "0" * 40
                and "0x" + log["topics"][2][-40:].lower() == faucet
            ]
            print(json.dumps({
                "endpoint": endpoint_name,
                "receipt_found": receipt is not None,
                "block": quantity(receipt["blockNumber"], "receipt block") if receipt else None,
                "receipt_log_count": len(logs),
                "transfer_log_count": len(transfers),
                "minted_transfer_count": sum(1 for log in transfers if log["topics"][1][-40:] == "0" * 40),
                "deployment_mints_to_faucet": len(faucet_mints),
                "deployment_minted_to_faucet_wei": str(sum(quantity(log["data"], "mint amount") for log in faucet_mints)),
                "transfer_topics": sorted({log["topics"][0] for log in transfers}),
                "event_topics": sorted({log.get("topics", [None])[0] for log in logs if log.get("topics")}),
            }))
        except Exception as error:
            print(json.dumps({"endpoint": endpoint_name, "error": getattr(error, "code", type(error).__name__)}))
    sys.exit(0)

if len(sys.argv) > 3 and sys.argv[2] == "--tx":
    tx_hash = sys.argv[3]
    for endpoint_name, endpoint in (
        ("configured-primary", env.get("FAUCET_RPC_URL", "")),
        ("publicnode", rpc_url),
        ("one-rpc", "https://1rpc.io/sepolia"),
    ):
        if not endpoint:
            continue
        try:
            receipt = rpc("eth_getTransactionReceipt", [tx_hash], endpoint)
            transfers = []
            for log in receipt.get("logs", []) if receipt else []:
                if log.get("address", "").lower() != token or log.get("topics", [None])[0] != transfer_topic:
                    continue
                sender = "0x" + log["topics"][1][-40:].lower()
                recipient = "0x" + log["topics"][2][-40:].lower()
                if sender == faucet:
                    direction = "out"
                elif recipient == faucet:
                    direction = "in"
                else:
                    direction = "other"
                transfers.append({"direction": direction, "amount_wei": str(quantity(log["data"], "Transfer amount"))})
            print(json.dumps({
                "endpoint": endpoint_name,
                "receipt_found": receipt is not None,
                "receipt_status": quantity(receipt["status"], "receipt status") if receipt else None,
                "block": quantity(receipt["blockNumber"], "receipt block") if receipt else None,
                "token_transfers": transfers,
            }))
        except Exception as error:
            print(json.dumps({"endpoint": endpoint_name, "error": getattr(error, "code", type(error).__name__)}))
    sys.exit(0)

if len(sys.argv) > 2 and sys.argv[2] == "--scan-one-rpc-incoming":
    latest = rpc("eth_getBlockByNumber", ["latest", False], "https://1rpc.io/sepolia")
    through = max(0, quantity(latest["number"], "latest block number") - confirmations)
    start = deployment
    events = {}
    chunks = 0
    while start <= through:
        end = min(start + 49, through)
        attempt = 0
        while True:
            try:
                logs = rpc("eth_getLogs", [{
                    "address": token,
                    "fromBlock": hex(start),
                    "toBlock": hex(end),
                    "topics": [transfer_topic, None, faucet_topic],
                }], "https://1rpc.io/sepolia")
                if not isinstance(logs, list):
                    raise RuntimeError("eth_getLogs returned a non-list")
                break
            except Exception as error:
                attempt += 1
                if attempt >= 6:
                    raise RuntimeError(f"incoming log scan failed at block {start}: {getattr(error, 'code', type(error).__name__)}") from None
                time.sleep(min(2 ** (attempt - 1), 16))
        for log in logs:
            key = (log["transactionHash"].lower(), quantity(log["logIndex"], "log index"))
            events[key] = {
                "block": quantity(log["blockNumber"], "Transfer block"),
                "amount_wei": str(quantity(log["data"], "Transfer amount")),
            }
        chunks += 1
        if chunks % 100 == 0:
            print(json.dumps({"scanned_to_block": end, "chunks": chunks, "incoming_events": len(events)}), flush=True)
        start = end + 1
        time.sleep(0.15)
    print(json.dumps({
        "endpoint": "one-rpc",
        "chain_id": quantity(rpc("eth_chainId", [], "https://1rpc.io/sepolia"), "chain ID"),
        "deployment_block": deployment,
        "safe_through_block": through,
        "chunks_50_blocks": chunks,
        "incoming_events": len(events),
        "incoming_wei": str(sum(int(event["amount_wei"]) for event in events.values())),
        "events": sorted(events.values(), key=lambda event: event["block"]),
    }), flush=True)
    sys.exit(0)

if len(sys.argv) > 2 and sys.argv[2] == "--scan-primary-incoming":
    endpoint = env.get("FAUCET_RPC_URL", "")
    if not endpoint:
        raise RuntimeError("protected config has no primary RPC URL")
    finalized = rpc("eth_getBlockByNumber", ["finalized", False], endpoint)
    latest = rpc("eth_getBlockByNumber", ["latest", False], endpoint)
    through = min(
        quantity(finalized["number"], "finalized block number"),
        max(0, quantity(latest["number"], "latest block number") - confirmations),
    )
    start = deployment
    events = {}
    chunks = 0
    while start <= through:
        end = min(start + 4_999, through)
        logs = rpc("eth_getLogs", [{
            "address": token,
            "fromBlock": hex(start),
            "toBlock": hex(end),
            "topics": [transfer_topic, None, faucet_topic],
        }], endpoint)
        if not isinstance(logs, list):
            raise RuntimeError(f"primary RPC returned non-list logs at block {start}")
        for log in logs:
            key = (log["transactionHash"].lower(), quantity(log["logIndex"], "log index"))
            events[key] = {
                "block": quantity(log["blockNumber"], "Transfer block"),
                "amount_wei": str(quantity(log["data"], "Transfer amount")),
            }
        chunks += 1
        start = end + 1
    print(json.dumps({
        "endpoint": "configured-primary",
        "chain_id": quantity(rpc("eth_chainId", [], endpoint), "chain ID"),
        "deployment_block": deployment,
        "safe_through_block": through,
        "chunks_5000_blocks": chunks,
        "incoming_events": len(events),
        "incoming_wei": str(sum(int(event["amount_wei"]) for event in events.values())),
        "events": sorted(events.values(), key=lambda event: event["block"]),
    }), flush=True)
    sys.exit(0)


finalized = rpc("eth_getBlockByNumber", ["finalized", False])
latest = rpc("eth_getBlockByNumber", ["latest", False])
finalized_number = quantity(finalized["number"], "finalized block number")
latest_number = quantity(latest["number"], "latest block number")
through = min(finalized_number, max(0, latest_number - confirmations))
logs_by_key = {}
all_transfer_logs = {}
start = deployment
while start <= through:
    end = min(start + 4_999, through)
    for topics in ([transfer_topic, faucet_topic, None], [transfer_topic, None, faucet_topic]):
        logs = rpc("eth_getLogs", [{
            "address": token,
            "fromBlock": hex(start),
            "toBlock": hex(end),
            "topics": topics,
        }])
        if not isinstance(logs, list):
            raise RuntimeError("eth_getLogs returned a non-list")
        for log in logs:
            logs_by_key[(log["transactionHash"].lower(), quantity(log["logIndex"], "log index"))] = log
    all_logs = rpc("eth_getLogs", [{
        "address": token,
        "fromBlock": hex(start),
        "toBlock": hex(end),
        "topics": [transfer_topic],
    }])
    if not isinstance(all_logs, list):
        raise RuntimeError("unfiltered eth_getLogs returned a non-list")
    for log in all_logs:
        all_transfer_logs[(log["transactionHash"].lower(), quantity(log["logIndex"], "log index"))] = log
    start = end + 1

incoming = []
outgoing = []
for log in logs_by_key.values():
    topics = log["topics"]
    sender = "0x" + topics[1][-40:].lower()
    recipient = "0x" + topics[2][-40:].lower()
    amount = quantity(log["data"], "Transfer amount")
    entry = {"block": quantity(log["blockNumber"], "Transfer block"), "amount_wei": str(amount)}
    if recipient == faucet:
        incoming.append(entry)
    if sender == faucet:
        outgoing.append(entry)

block_tag = hex(through)
balance_data = "0x70a08231" + faucet[2:].rjust(64, "0")
supply_data = "0x18160ddd"
balance_result = rpc("eth_call", [{"to": token, "data": balance_data}, block_tag])
supply_result = rpc("eth_call", [{"to": token, "data": supply_data}, block_tag])
balance = quantity(balance_result, "faucet balance")
supply = quantity(supply_result, "total supply")
try:
    deployment_balance_result = rpc("eth_call", [{"to": token, "data": balance_data}, hex(deployment)])
    deployment_balance = str(quantity(deployment_balance_result, "deployment-block faucet balance"))
except RuntimeError:
    deployment_balance = "historical state pruned by RPC"
in_total = sum(int(event["amount_wei"]) for event in incoming)
out_total = sum(int(event["amount_wei"]) for event in outgoing)
minted_total = sum(
    quantity(log["data"], "mint amount")
    for log in all_transfer_logs.values()
    if log["topics"][1][-40:] == "0" * 40
)
burned_total = sum(
    quantity(log["data"], "burn amount")
    for log in all_transfer_logs.values()
    if log["topics"][2][-40:] == "0" * 40
)
report = {
    "chain_id": quantity(rpc("eth_chainId", []), "chain ID"),
    "deployment_block": deployment,
    "safe_through_block": through,
    "finalized_block": finalized_number,
    "incoming_events": len(incoming),
    "outgoing_events": len(outgoing),
    "all_transfer_events": len(all_transfer_logs),
    "minted_event_total_wei": str(minted_total),
    "burned_event_total_wei": str(burned_total),
    "faucet_balance_at_deployment_wei": deployment_balance,
    "incoming_wei": str(in_total),
    "outgoing_wei": str(out_total),
    "history_net_wei": str(in_total - out_total),
    "balance_of_wei": str(balance),
    "total_supply_wei": str(supply),
    "balance_matches_history": in_total - out_total == balance,
    "events": sorted(
        [{"direction": "in", **event} for event in incoming]
        + [{"direction": "out", **event} for event in outgoing],
        key=lambda event: (event["block"], event["direction"]),
    ),
}
print(json.dumps(report, indent=2))
if not report["balance_matches_history"]:
    sys.exit(2)

#!/usr/bin/env python3
"""Classify only the two receipt-backed faucet funding rows after safe import.

Run on the production faucet VM. This changes accounting labels in PostgreSQL;
it never signs, broadcasts, mints, or opens public claims.
"""

import json
import subprocess
import sys
import urllib.request
from datetime import datetime, timezone
from pathlib import Path


TOKEN = "0xbc2a72107c6765364d0a822a88a6e6c61f30eaa6"
TREASURY = "0x3a3206d683441aa91cca7cd5757b1f0ca463213a"
FAUCET = "0x721c8dbd8208ca9dbdcc12ae9339225458608b81"
TRANSFER_HASH = "0x11f797c1d4514973ee88770f359ce63dcafa22c27de8996c909f12d9f713df21"
SEED_HASH = "0x39894f356780e065fd9f3d676a458a6d639b8e69a8d7a71b48de860cc530537f"
AMOUNT = 92_000 * 10**18
SEED = 10_000 * 10**18
BALANCE = 100_000 * 10**18
SUPPLY = 20_000_000 * 10**18
TRANSFER_BLOCK = 11_810_180
DB = "creg-unified-faucet-db"


def public_json(route):
    request = urllib.request.Request(
        "https://faucet.testnet.cregnet.dev" + route,
        headers={"User-Agent": "creg-r28-funding-classifier/1"},
    )
    with urllib.request.urlopen(request, timeout=25) as response:
        if response.status != 200:
            raise RuntimeError(f"{route} returned HTTP {response.status}")
        return json.load(response)


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def main():
    if len(sys.argv) != 2:
        raise SystemExit("Provide a new evidence JSON path")
    evidence_path = Path(sys.argv[1])
    require(not evidence_path.exists(), "Evidence already exists; inspect before rerunning")
    stats = public_json("/api/stats")
    supply = public_json("/api/supply")
    health = public_json("/health")
    require(stats.get("status") == "available", "Live faucet stats unavailable")
    require(stats.get("ledger_reconciled") is True, "Faucet ledger is not reconciled")
    require(stats.get("token_claims_enabled") is False, "Claims unexpectedly enabled")
    require(stats.get("token_distribution_open") is False, "Distribution gate unexpectedly open")
    require(stats.get("stats", {}).get("unresolved_claims") == 0, "Unresolved faucet claims")
    require(int(stats.get("faucet_balance", "-1")) == BALANCE, "Live wallet is not at 100,000 tCREG")
    require(stats.get("token_contract", "").lower() == TOKEN, "Unexpected live token")
    require(stats.get("faucet_address", "").lower() == FAUCET, "Unexpected live faucet")
    require(health.get("ledger_reconciled") is True, "Health route disagrees on reconciliation")
    require(supply.get("status") == "available", "Safe supply snapshot unavailable")
    require(supply.get("chain_id") == 11155111, "Wrong Sepolia chain")
    require(supply.get("block_number", 0) >= TRANSFER_BLOCK, "Safe watermark has not imported the transfer")
    require(int(supply.get("faucet", {}).get("balance_wei", "-1")) == BALANCE, "Safe balance is not 100,000 tCREG")
    require(int(supply.get("total_supply_wei", "-1")) == SUPPLY, "Total supply changed")
    require(supply.get("token_contract", "").lower() == TOKEN, "Safe token changed")
    require(supply.get("faucet", {}).get("address", "").lower() == FAUCET, "Safe faucet changed")
    require(supply.get("treasury", {}).get("address", "").lower() == TREASURY, "Safe treasury changed")
    require(int(supply.get("treasury", {}).get("balance_wei", "-1")) == 19_897_590 * 10**18, "Safe treasury balance changed")

    sql = f"""
BEGIN;
DO $r28$
DECLARE
    watermark bigint;
    recorded_balance numeric;
    recorded_supply numeric;
    matching_rows integer;
    class_name text;
    paid numeric;
BEGIN
    SELECT reconciled_through_block, faucet_balance_wei, total_supply_wei
      INTO watermark, recorded_balance, recorded_supply
      FROM faucet_reconciliation_state
     WHERE chain_id = 11155111 AND token_address = '{TOKEN}' FOR SHARE;
    IF watermark IS NULL OR watermark < {TRANSFER_BLOCK} OR
       recorded_balance <> {BALANCE} OR recorded_supply <> {SUPPLY} THEN
        RAISE EXCEPTION 'Production reconciliation watermark or balance differs';
    END IF;
    SELECT COALESCE(SUM(token_amount_wei), 0) INTO paid
      FROM faucet_claim_ledger WHERE token_status = 'confirmed';
    IF paid <> {2_000 * 10**18} OR EXISTS (
        SELECT 1 FROM faucet_claim_ledger
         WHERE token_status IN ('reserved', 'submitted')
            OR native_status IN ('reserved', 'submitted')
    ) THEN
        RAISE EXCEPTION 'Claim history or unresolved reservations changed';
    END IF;

    SELECT count(*) INTO matching_rows FROM faucet_token_transfers
     WHERE chain_id = 11155111 AND token_address = '{TOKEN}'
       AND tx_hash = '{TRANSFER_HASH}' AND block_number = {TRANSFER_BLOCK}
       AND from_address = '{TREASURY}' AND to_address = '{FAUCET}'
       AND amount_wei = {AMOUNT};
    IF matching_rows <> 1 THEN RAISE EXCEPTION 'Approved R28 Transfer event missing or ambiguous'; END IF;
    SELECT count(*) INTO matching_rows FROM faucet_funding_ledger
     WHERE chain_id = 11155111 AND token_address = '{TOKEN}'
       AND tx_hash = '{TRANSFER_HASH}' AND block_number = {TRANSFER_BLOCK}
       AND sender = '{TREASURY}' AND amount_wei = {AMOUNT};
    IF matching_rows <> 1 THEN RAISE EXCEPTION 'Approved R28 funding row missing or ambiguous'; END IF;
    SELECT classification INTO class_name FROM faucet_funding_ledger
     WHERE chain_id = 11155111 AND token_address = '{TOKEN}'
       AND tx_hash = '{TRANSFER_HASH}' AND block_number = {TRANSFER_BLOCK}
       AND sender = '{TREASURY}' AND amount_wei = {AMOUNT} FOR UPDATE;
    IF class_name NOT IN ('historical_unallocated', 'approved_r28_working_balance_20260929')
       OR class_name IS NULL THEN
        RAISE EXCEPTION 'Approved R28 funding row missing or differently classified';
    END IF;

    SELECT count(*) INTO matching_rows FROM faucet_token_transfers
     WHERE chain_id = 11155111 AND token_address = '{TOKEN}'
       AND tx_hash = '{SEED_HASH}' AND from_address = '{TREASURY}'
       AND to_address = '{FAUCET}' AND amount_wei = {SEED};
    IF matching_rows <> 1 THEN RAISE EXCEPTION 'Prior approved seed Transfer event missing or ambiguous'; END IF;
    SELECT count(*) INTO matching_rows FROM faucet_funding_ledger
     WHERE chain_id = 11155111 AND token_address = '{TOKEN}'
       AND tx_hash = '{SEED_HASH}' AND sender = '{TREASURY}'
       AND amount_wei = {SEED};
    IF matching_rows <> 1 THEN RAISE EXCEPTION 'Approved seed funding row missing or ambiguous'; END IF;
    SELECT classification INTO class_name FROM faucet_funding_ledger
     WHERE chain_id = 11155111 AND token_address = '{TOKEN}'
       AND tx_hash = '{SEED_HASH}' AND sender = '{TREASURY}'
       AND amount_wei = {SEED} FOR UPDATE;
    IF class_name NOT IN ('historical_unallocated', 'approved_seed_20260927')
       OR class_name IS NULL THEN
        RAISE EXCEPTION 'Approved seed funding row missing or differently classified';
    END IF;

    UPDATE faucet_funding_ledger SET classification = 'approved_r28_working_balance_20260929'
     WHERE chain_id = 11155111 AND token_address = '{TOKEN}' AND tx_hash = '{TRANSFER_HASH}';
    UPDATE faucet_funding_ledger SET classification = 'approved_seed_20260927'
     WHERE chain_id = 11155111 AND token_address = '{TOKEN}' AND tx_hash = '{SEED_HASH}';
END $r28$;
COMMIT;
SELECT json_build_object(
  'watermark', (SELECT reconciled_through_block FROM faucet_reconciliation_state
                 WHERE chain_id = 11155111 AND token_address = '{TOKEN}'),
  'recorded_balance_wei', (SELECT faucet_balance_wei::text FROM faucet_reconciliation_state
                 WHERE chain_id = 11155111 AND token_address = '{TOKEN}'),
  'funding', (SELECT json_agg(json_build_object('tx_hash', tx_hash,
              'amount_wei', amount_wei::text, 'classification', classification)
              ORDER BY block_number) FROM faucet_funding_ledger
              WHERE chain_id = 11155111 AND token_address = '{TOKEN}'
                AND tx_hash IN ('{SEED_HASH}', '{TRANSFER_HASH}'))
);
"""
    command = [
        "sudo", "docker", "exec", "-i", DB, "sh", "-c",
        'exec psql -X -q -v ON_ERROR_STOP=1 -U "$POSTGRES_USER" -d "$POSTGRES_DB" -At',
    ]
    completed = subprocess.run(command, input=sql, text=True, capture_output=True, timeout=60)
    require(completed.returncode == 0, "Database classification failed: " + completed.stderr.strip())
    result = json.loads(completed.stdout.strip().splitlines()[-1])
    require(result["watermark"] >= TRANSFER_BLOCK, "Database watermark fell behind")
    require(int(result["recorded_balance_wei"]) == BALANCE, "Database balance changed")
    require(len(result["funding"]) == 2, "Expected exactly two approved funding records")
    evidence = {
        "recorded_at_utc": datetime.now(timezone.utc).isoformat(),
        "status": "classified_after_safe_sepolia_reconciliation",
        "transaction_hash": TRANSFER_HASH,
        "safe_block_number": supply["block_number"],
        "safe_block_hash": supply["block_hash"],
        "live_balance_wei": stats["faucet_balance"],
        "total_supply_wei": supply["total_supply_wei"],
        "confirmed_claims_wei": str(2_000 * 10**18),
        "database": result,
        "token_claims_remain_closed": True,
        "on_chain_transaction_sent": False,
    }
    evidence_path.parent.mkdir(parents=True, exist_ok=True)
    evidence_path.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(evidence, indent=2))


if __name__ == "__main__":
    main()

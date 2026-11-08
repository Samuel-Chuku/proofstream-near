#!/usr/bin/env python3
"""Verify the OutLayer executions in one NEAR testnet transaction, off-chain.

Usage:  python3 -I scripts/verify-execution.py <tx_hash> [--sender ACCOUNT] [--builds builds.json]

For every execution the transaction requested, it prints one PASS/FAIL line per check:

  task-hash      sha256 of the task fields (OutLayer worker/src/tdx_attestation.rs, V1)
                 equals report_data[0:32] of the TDX quote, and report_data[32:64] is zero
  measurements   the quote's MRTD and RTMR0-3 are one approved set on worker.outlayer.testnet,
                 read at the block the execution ran
  wasm           the executed wasm sha256 is the build the request named (the contract sends
                 its milestone pin as version_key), and is listed in builds.json if given
  input          input_hash is the sha256 of the input_data recorded on chain
  output         output_hash is the sha256 of the result OutLayer delivered on chain
  caller         the attested caller and project are the ones that made the request
  callback       the resume came from a resolve_execution tx signed by worker.outlayer.testnet
                 with a function-call key scoped to outlayer.testnet (a key the register
                 contract adds only after checking that worker's own TDX quote)
  intel-chain    the quote is signed by a genuine Intel TDX platform whose TCB is UpToDate,
                 via dcap-qvl with Intel's own collateral; SKIP if dcap-qvl isn't installed

Standard library only. Reads public endpoints: NEAR archival RPC, the OutLayer API and
NearBlocks (only to look up a tx's signer when --sender is not given), and Intel's PCS
through dcap-qvl. Exit code 0 only
if every check passes.
"""

import argparse
import base64
import bisect
import hashlib
import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

# Archival endpoints, tried in turn: public RPCs rate-limit, and old blocks need archival.
RPCS = ["https://archival-rpc.testnet.near.org", "https://archival-rpc.testnet.fastnear.com"]
OUTLAYER_API = "https://testnet-api.outlayer.ai"
NEARBLOCKS = "https://api-testnet.nearblocks.io/v3"
INTEL_PCS = "https://api.trustedservices.intel.com"
OUTLAYER = "outlayer.testnet"
REGISTER = "worker.outlayer.testnet"
USER_AGENT = "proofstream-verify-execution/1"
MAX_ATTEMPTS = 6
TIMEOUT_S = 30
# How far back from the callback block to look for the worker's resolve tx.
RESOLVE_SCAN_BLOCKS = 6

# TDX quote v4 offsets: 48-byte header, then the TD report body.
QUOTE_HEADER = 48
MEASUREMENT_SIZE = 48
MEASUREMENT_OFFSETS = {"mrtd": 136, "rtmr0": 328, "rtmr1": 376, "rtmr2": 424, "rtmr3": 472}
REPORT_DATA_OFFSET = 520
REPORT_DATA_SIZE = 64

results = []


def report(request_id, check, ok, detail):
    status = "PASS" if ok is True else ("SKIP" if ok is None else "FAIL")
    results.append(status)
    print(f"{status}  request {request_id}  {check:<13} {detail}")


def http_json(urls, body=None):
    """GET (or POST body) with exponential backoff on 429, 5xx and network errors,
    moving to the next url in the list on each retry."""
    urls = [urls] if isinstance(urls, str) else urls
    data = json.dumps(body).encode() if body is not None else None
    headers = {"user-agent": USER_AGENT, "content-type": "application/json"}
    last = "no attempt"
    for attempt in range(MAX_ATTEMPTS):
        url = urls[attempt % len(urls)]
        try:
            with urllib.request.urlopen(urllib.request.Request(url, data, headers), timeout=TIMEOUT_S) as r:
                return json.load(r)
        except urllib.error.HTTPError as e:
            if e.code != 429 and e.code < 500:
                raise SystemExit(f"error: {url} answered HTTP {e.code}")
            last = f"HTTP {e.code} from {url}"
        except (urllib.error.URLError, TimeoutError) as e:
            last = f"{e} from {url}"
        time.sleep(2 ** attempt)
    raise SystemExit(f"error: gave up after {MAX_ATTEMPTS} attempts; last: {last}")


def rpc(method, params):
    r = http_json(RPCS, {"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
    if "error" in r:
        raise SystemExit(f"error: RPC {method}: {json.dumps(r['error'])[:300]}")
    return r["result"]


def view(account, method, args, block_id):
    r = rpc("query", {"request_type": "call_function", "block_id": block_id, "account_id": account,
                      "method_name": method, "args_base64": base64.b64encode(json.dumps(args).encode()).decode()})
    return json.loads(bytes(r["result"]))


def tx_signer(tx_hash):
    d = http_json(f"{NEARBLOCKS}/txns/{tx_hash}")
    signer = (d.get("data") or {}).get("signer_account_id")
    if not signer:
        raise SystemExit("error: could not look up the tx signer; pass --sender")
    return signer


def function_calls(receipt):
    action = receipt["receipt"].get("Action") or {}
    for a in action.get("actions", []):
        if isinstance(a, dict) and "FunctionCall" in a:
            fc = a["FunctionCall"]
            yield fc["method_name"], json.loads(base64.b64decode(fc["args"]) or b"{}")


def task_hash(att):
    """OutLayer worker/src/tdx_attestation.rs generate_task_attestation, V1 layout."""
    h = hashlib.sha256()
    h.update(att["task_type"].encode())
    h.update(struct.pack("<q", att["task_id"]))
    for key in ("repo", "commit", "build_target", "wasm_hash", "input_hash"):
        if att.get(key) is not None:
            h.update(att[key].encode())
    h.update(att["output_hash"].encode())
    if att.get("block_height") is not None:
        h.update(struct.pack("<Q", att["block_height"]))
    for key in ("caller_account_id", "project_id", "secrets_ref"):
        if att.get(key) is not None:
            h.update(att[key].encode())
    h.update(struct.pack("<q", att["timestamp"]))
    if att.get("attached_usd") is not None:
        h.update(str(att["attached_usd"]).encode())
    return h.digest()


def quote_parts(quote):
    need = QUOTE_HEADER + REPORT_DATA_OFFSET + REPORT_DATA_SIZE
    if len(quote) < need:
        return None, None
    m = {k: quote[QUOTE_HEADER + o:QUOTE_HEADER + o + MEASUREMENT_SIZE].hex() for k, o in MEASUREMENT_OFFSETS.items()}
    rd = quote[QUOTE_HEADER + REPORT_DATA_OFFSET:QUOTE_HEADER + REPORT_DATA_OFFSET + REPORT_DATA_SIZE]
    return m, rd


def shard_of(account, block_id):
    """A transaction is included in its signer's shard, so only that chunk needs reading."""
    layout = rpc("EXPERIMENTAL_protocol_config", {"block_id": block_id})["shard_layout"]
    version = next(iter(layout.values()))
    return version["shard_ids"][bisect.bisect_right(version["boundary_accounts"], account)]


def find_resolve(request_id, callback_block):
    shard = shard_of(REGISTER, callback_block)
    for height in range(callback_block, callback_block - RESOLVE_SCAN_BLOCKS, -1):
        block = rpc("block", {"block_id": height})
        for c in block["chunks"]:
            if c["shard_id"] != shard or c["height_included"] != height:
                continue
            for t in rpc("chunk", {"chunk_id": c["chunk_hash"]})["transactions"]:
                if t["receiver_id"] != OUTLAYER:
                    continue
                for a in t["actions"]:
                    fc = a.get("FunctionCall") if isinstance(a, dict) else None
                    if fc and fc["method_name"] in ("resolve_execution", "submit_execution_output_and_resolve"):
                        args = json.loads(base64.b64decode(fc["args"]) or b"{}")
                        if str(args.get("request_id")) == str(request_id):
                            return height, t
    return None, None


def intel_chain(request_id, quote):
    tool = shutil.which("dcap-qvl")
    if not tool:
        report(request_id, "intel-chain", None, "dcap-qvl not installed (cargo install dcap-qvl-cli --version 0.7.1)")
        return
    # Collateral (certs, CRLs, TCB info) straight from Intel unless PCCS_URL says otherwise;
    # dcap-qvl's own default is a third-party mirror.
    env = {**os.environ, "PCCS_URL": os.environ.get("PCCS_URL") or INTEL_PCS}
    with tempfile.NamedTemporaryFile(suffix=".bin") as f:
        f.write(quote)
        f.flush()
        p = subprocess.run([tool, "verify", f.name], capture_output=True, text=True, timeout=120, env=env)
    if p.returncode != 0:
        out = (p.stdout + p.stderr).strip().splitlines()
        report(request_id, "intel-chain", False, out[-1][:160] if out else f"exit {p.returncode}")
        return
    status = next((json.loads(l).get("status") for l in p.stdout.splitlines() if l.startswith("{")), None)
    report(request_id, "intel-chain", status == "UpToDate", f"signed by Intel, TCB {status} (collateral: {env['PCCS_URL']})")


def verify_request(request_id, request, delivered, callback_block, builds):
    att = http_json(f"{OUTLAYER_API}/attestations/by-request/{request_id}")
    quote = base64.b64decode(att.get("tdx_quote") or "")
    measurements, report_data = quote_parts(quote)

    expected = task_hash(att)
    ok = report_data is not None and report_data[:32] == expected and report_data[32:] == bytes(32)
    report(request_id, "task-hash", ok, f"{expected.hex()[:16]}… vs quote {report_data[:32].hex()[:16] if report_data else 'none'}…")

    if measurements is None:
        report(request_id, "measurements", False, "quote too short")
    else:
        approved = view(REGISTER, "get_approved_measurements", {}, att["block_height"])
        ok = any(all(a.get(k) == v for k, v in measurements.items()) for a in approved)
        report(request_id, "measurements", ok, f"MRTD {measurements['mrtd'][:12]}…, RTMR3 {measurements['rtmr3'][:12]}…; {len(approved)} approved sets at block {att['block_height']}")

    source = request["args"].get("source") or {}
    named = (source.get("Project") or {}).get("version_key") or (source.get("WasmUrl") or {}).get("hash")
    executed = att.get("executed_wasm_sha256")
    ok = executed is not None and executed == att.get("wasm_hash") and (named is None or executed == named)
    detail = f"executed {executed}" + (f", requested {named}" if named else "")
    if builds is not None:
        listed = [b["project"] for b in builds if b.get("sha256") == executed]
        ok = ok and bool(listed)
        detail += f", builds.json: {', '.join(listed) or 'not listed'}"
    report(request_id, "wasm", ok, detail)

    input_data = request["event"].get("input_data")
    ok = input_data is not None and hashlib.sha256(input_data.encode()).hexdigest() == att.get("input_hash")
    report(request_id, "input", ok, f"sha256(input_data) {'=' if ok else '!='} input_hash {att.get('input_hash', '')[:16]}…")

    if delivered is None:
        report(request_id, "output", False, "no delivered result found in this tx")
    else:
        ok = hashlib.sha256(delivered).hexdigest() == att.get("output_hash")
        report(request_id, "output", ok, f"sha256 of {len(delivered)} delivered bytes {'=' if ok else '!='} output_hash")

    # OutLayer attests the tx signer as the caller. The contract that sent the request is
    # the event's predecessor_id, which the quote does not cover: printed, not attested.
    event = request["event"]
    secrets = event.get("secrets_ref")
    secrets = f"{secrets['account_id']}/{secrets['profile']}" if isinstance(secrets, dict) else None
    ok = (att.get("caller_account_id") == event.get("sender_id") and att.get("project_id") == event.get("project_id")
          and att.get("secrets_ref") == secrets)
    report(request_id, "caller", ok, f"signer {att.get('caller_account_id')}, project {att.get('project_id')}, "
                                     f"secrets {att.get('secrets_ref')}; sent by {event.get('predecessor_id')} (not attested)")

    if callback_block is None:
        report(request_id, "callback", False, "no OutLayer callback receipt in this tx")
    else:
        height, rtx = find_resolve(request_id, callback_block)
        if rtx is None:
            report(request_id, "callback", False, f"no resolve tx within {RESOLVE_SCAN_BLOCKS} blocks before {callback_block}")
        else:
            key = rpc("query", {"request_type": "view_access_key", "block_id": height,
                                "account_id": rtx["signer_id"], "public_key": rtx["public_key"]})
            perm = key.get("permission")
            fc = perm.get("FunctionCall") if isinstance(perm, dict) else None
            ok = rtx["signer_id"] == REGISTER and fc is not None and fc.get("receiver_id") == OUTLAYER
            curve = rtx["public_key"].split(":")[0]
            report(request_id, "callback", ok, f"tx {rtx['hash']} by {rtx['signer_id']}, {curve} function-call key on {fc.get('receiver_id') if fc else 'FULL ACCESS'}")

    if quote:
        intel_chain(request_id, quote)
    else:
        report(request_id, "intel-chain", False, "no quote")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("tx_hash")
    ap.add_argument("--sender", help="the tx signer (looked up on NearBlocks when omitted)")
    ap.add_argument("--builds", help="builds.json; the executed wasm must be listed in it")
    a = ap.parse_args()

    builds = None
    if a.builds:
        with open(a.builds) as f:
            builds = json.load(f)

    sender = a.sender or tx_signer(a.tx_hash)
    tx = rpc("EXPERIMENTAL_tx_status", {"tx_hash": a.tx_hash, "sender_account_id": sender, "wait_until": "FINAL"})
    outcomes = {o["id"]: o for o in tx["receipts_outcome"]}

    requests, delivered, callback_block = {}, {}, {}
    for receipt in tx["receipts"]:
        outcome = outcomes.get(receipt["receipt_id"])
        for method, args in function_calls(receipt):
            if receipt["receiver_id"] == OUTLAYER and method == "request_execution" and outcome:
                for log in outcome["outcome"]["logs"]:
                    if log.startswith("EVENT_JSON:") and '"execution_requested"' in log:
                        event = json.loads(json.loads(log[len("EVENT_JSON:"):])["data"][0]["request_data"])
                        requests[str(event["request_id"])] = {"args": args, "event": event}
            if receipt["receiver_id"] == OUTLAYER and method == "on_execution_response" and outcome:
                rid = str(args.get("request_id"))
                value = outcome["outcome"]["status"].get("SuccessValue") if isinstance(outcome["outcome"]["status"], dict) else None
                delivered[rid] = base64.b64decode(value) if value is not None else None
                callback_block[rid] = rpc("block", {"block_id": outcome["block_hash"]})["header"]["height"]

    if not requests:
        raise SystemExit(f"error: tx {a.tx_hash} requested no OutLayer execution")
    print(f"tx {a.tx_hash} (signer {sender}): {len(requests)} execution(s)")
    for rid, request in sorted(requests.items(), key=lambda kv: int(kv[0])):
        verify_request(rid, request, delivered.get(rid), callback_block.get(rid), builds)

    failed = results.count("FAIL")
    skipped = results.count("SKIP")
    verdict = "FAIL" if failed else ("PARTIAL" if skipped else "PASS")
    print(f"{verdict}: {results.count('PASS')} passed, {failed} failed, {skipped} skipped")
    sys.exit(0 if verdict == "PASS" else 1)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Derive a libp2p PeerId from a CREG node's p2p/identity.key file.

The identity file is a libp2p-protobuf PrivateKey envelope. For Ed25519 keys
(<=42 byte marshalled public key) the PeerId is base58(identity-multihash(
protobuf-marshalled-public-key)).

Usage:
    python3 derive-peer-id.py <path-to-identity.key>
    python3 derive-peer-id.py ~/.creg-validator-validator-1/chain-data/p2p/identity.key
"""
import sys

B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def b58encode(b: bytes) -> str:
    n = int.from_bytes(b, "big")
    out = ""
    while n > 0:
        n, r = divmod(n, 58)
        out = B58[r] + out
    # leading zero bytes -> '1'
    for byte in b:
        if byte == 0:
            out = "1" + out
        else:
            break
    return out or "1"


def read_varint(buf: bytes, pos: int):
    result = 0
    shift = 0
    while True:
        byte = buf[pos]
        pos += 1
        result |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return result, pos
        shift += 7


def parse_fields(buf: bytes):
    """Minimal protobuf TLV parse -> {field_number: [values]} (varint as int, len-delim as bytes)."""
    fields = {}
    pos = 0
    while pos < len(buf):
        tag, pos = read_varint(buf, pos)
        number, wire = tag >> 3, tag & 0x07
        if wire == 0:
            val, pos = read_varint(buf, pos)
        elif wire == 2:
            ln, pos = read_varint(buf, pos)
            val, pos = buf[pos:pos + ln], pos + ln
        else:
            raise ValueError(f"unsupported wire type {wire}")
        fields.setdefault(number, []).append(val)
    return fields


def main(path: str) -> int:
    with open(path, "rb") as f:
        raw = f.read()
    fields = parse_fields(raw)
    key_type = fields.get(1, [None])[0]
    data = fields.get(2, [None])[0]
    if key_type != 1 or not isinstance(data, bytes):
        print(f"not an Ed25519 libp2p private key envelope: type={key_type}", file=sys.stderr)
        return 1
    if len(data) == 64:
        pub = data[32:]  # seed || public
    elif len(data) == 32:
        print("only a 32-byte seed present; cannot derive pubkey without ed25519", file=sys.stderr)
        return 1
    else:
        print(f"unexpected private payload length {len(data)}", file=sys.stderr)
        return 1
    marshalled_pub = b"\x08\x01\x12\x20" + pub  # PublicKey{key_type:1, data:pub}
    peer_id = b58encode(b"\x00\x24" + marshalled_pub)  # identity multihash, base58
    print(peer_id)
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print(__doc__.strip().splitlines()[-3], file=sys.stderr)
        sys.exit(2)
    sys.exit(main(sys.argv[1]))

// ADR-019 Increment 1 (ECA-29, ECA-36): client address for rate limiting.
//
// The old limiter keyed on the LEFTMOST X-Forwarded-For entry, which the
// client writes, and fell back to a shared "unknown" bucket. Here:
// - forwarding headers are believed only when the TCP peer is a trusted
//   proxy (loopback / private network: hub-web's nginx on the Docker
//   network), never from a public peer;
// - X-Real-IP (set by hub-web from the realip-recovered address) wins;
//   otherwise X-Forwarded-For is walked from the RIGHT, skipping trusted
//   proxy hops, and the first public address is the client;
// - anything that is not an IP address is ignored, so arbitrary strings
//   never become limiter keys;
// - IPv6 clients are bucketed by /64.
//
// Dependency-free so it can be unit-tested without the server runtime.

import { isIP } from "node:net";

export type ClientAddressInput = {
  /** Socket peer address as reported by Node (may be IPv4-mapped IPv6). */
  peer: string | undefined;
  realIp?: string | null;
  forwardedFor?: string | null;
};

export type ClientAddressOptions = {
  /** Believe forwarding headers from loopback/private peers (default true). */
  trustPrivateProxies: boolean;
};

export function normalizeIp(raw: string | undefined | null): string | undefined {
  if (!raw) return undefined;
  let value = raw.trim();
  if (value.startsWith("[") && value.includes("]")) {
    value = value.slice(1, value.indexOf("]"));
  }
  const zone = value.indexOf("%");
  if (zone !== -1) value = value.slice(0, zone);
  const mapped = /^::ffff:(\d{1,3}(?:\.\d{1,3}){3})$/i.exec(value);
  if (mapped) value = mapped[1];
  return isIP(value) === 0 ? undefined : value.toLowerCase();
}

function ipv4Octets(ip: string): number[] {
  return ip.split(".").map((part) => Number(part));
}

/** Loopback, RFC 1918, unique-local and link-local addresses. */
export function isTrustedProxyAddress(ip: string): boolean {
  if (isIP(ip) === 4) {
    const [a, b] = ipv4Octets(ip);
    return (
      a === 10 ||
      a === 127 ||
      (a === 172 && b >= 16 && b <= 31) ||
      (a === 192 && b === 168)
    );
  }
  const lower = ip.toLowerCase();
  return (
    lower === "::1" ||
    lower.startsWith("fc") ||
    lower.startsWith("fd") ||
    /^fe[89ab]/.test(lower)
  );
}

/** Limiter bucket for an address: IPv4 as-is, IPv6 truncated to /64. */
export function bucketFor(ip: string): string {
  if (isIP(ip) !== 6) return ip;
  const [head, tail = ""] = ip.split("::");
  const headParts = head ? head.split(":") : [];
  const tailParts = tail ? tail.split(":") : [];
  const missing = 8 - headParts.length - tailParts.length;
  const full = [...headParts, ...Array<string>(Math.max(0, missing)).fill("0"), ...tailParts];
  const prefix = full
    .slice(0, 4)
    .map((part) => (Number.parseInt(part || "0", 16) || 0).toString(16))
    .join(":");
  return `${prefix}::/64`;
}

/**
 * Resolve the limiter key for one request. Returns `undefined` when no
 * address can be established (no socket peer), so production callers can
 * fail closed instead of sharing one bucket.
 */
export function resolveClientAddress(
  input: ClientAddressInput,
  options: ClientAddressOptions,
): string | undefined {
  const peer = normalizeIp(input.peer);
  if (!peer) return undefined;
  if (!options.trustPrivateProxies || !isTrustedProxyAddress(peer)) {
    return bucketFor(peer);
  }
  const realIp = normalizeIp(input.realIp);
  if (realIp) return bucketFor(realIp);
  const hops = (input.forwardedFor ?? "")
    .split(",")
    .map((entry) => normalizeIp(entry))
    .filter((entry): entry is string => entry !== undefined);
  for (let index = hops.length - 1; index >= 0; index -= 1) {
    if (!isTrustedProxyAddress(hops[index])) return bucketFor(hops[index]);
  }
  return bucketFor(peer);
}

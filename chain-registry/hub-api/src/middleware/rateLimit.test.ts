// ADR-019 Increment 1 — eco-adv-09 and ECA-36 limiter tests.
import assert from "node:assert/strict";
import test from "node:test";
import {
  bucketFor,
  isTrustedProxyAddress,
  normalizeIp,
  resolveClientAddress,
} from "./clientAddress.js";
import { BoundedBucketStore } from "./rateLimit.js";

const trust = { trustPrivateProxies: true };

function randomIpv4(): string {
  return Array.from({ length: 4 }, () => Math.floor(Math.random() * 256)).join(".");
}

test("eco-adv-09: 10,000 random X-Forwarded-For values from a public peer share one bucket", () => {
  const store = new BoundedBucketStore(1000);
  const max = 30;
  let allowed = 0;
  for (let i = 0; i < 10_000; i += 1) {
    const key = resolveClientAddress(
      { peer: "203.0.113.50", forwardedFor: `${randomIpv4()}, ${randomIpv4()}` },
      trust,
    );
    assert.equal(key, "203.0.113.50");
    if (store.hit(`status:${key}`, 1_000, 60_000).count <= max) allowed += 1;
  }
  assert.equal(allowed, max);
  assert.equal(store.size, 1);
});

test("store size stays bounded even with many distinct real clients", () => {
  const store = new BoundedBucketStore(100);
  for (let i = 0; i < 10_000; i += 1) {
    store.hit(`k:${i}`, 1_000, 60_000);
  }
  assert.equal(store.size, 100);
});

test("LRU eviction keeps recently active clients counted", () => {
  const store = new BoundedBucketStore(2);
  store.hit("a", 0, 60_000);
  store.hit("b", 0, 60_000);
  store.hit("a", 1, 60_000); // a becomes most recent
  store.hit("c", 2, 60_000); // evicts b
  assert.equal(store.hit("a", 3, 60_000).count, 3);
  assert.equal(store.hit("b", 4, 60_000).count, 1);
});

test("windows reset after expiry", () => {
  const store = new BoundedBucketStore(10);
  assert.equal(store.hit("a", 0, 1_000).count, 1);
  assert.equal(store.hit("a", 500, 1_000).count, 2);
  assert.equal(store.hit("a", 1_000, 1_000).count, 1);
});

test("forwarding headers are believed only from private proxies, right to left", () => {
  // Behind hub-web: X-Real-IP from the realip-recovered address wins.
  assert.equal(
    resolveClientAddress({ peer: "172.18.0.5", realIp: "198.51.100.7", forwardedFor: "6.6.6.6" }, trust),
    "198.51.100.7",
  );
  // No X-Real-IP: right-most public X-Forwarded-For entry.
  assert.equal(
    resolveClientAddress({ peer: "172.18.0.5", forwardedFor: "6.6.6.6, 198.51.100.7, 10.0.0.2" }, trust),
    "198.51.100.7",
  );
  // Proxy trust disabled: the peer is the client.
  assert.equal(
    resolveClientAddress(
      { peer: "172.18.0.5", realIp: "198.51.100.7" },
      { trustPrivateProxies: false },
    ),
    "172.18.0.5",
  );
  // Garbage is never a key.
  assert.equal(
    resolveClientAddress({ peer: "172.18.0.5", realIp: "not-an-ip", forwardedFor: "' OR 1=1" }, trust),
    "172.18.0.5",
  );
  // No socket peer: unknown, so production callers can fail closed.
  assert.equal(resolveClientAddress({ peer: undefined, realIp: "198.51.100.7" }, trust), undefined);
});

test("address normalisation and buckets", () => {
  assert.equal(normalizeIp("::ffff:192.0.2.1"), "192.0.2.1");
  assert.equal(normalizeIp("[2001:DB8::1]"), "2001:db8::1");
  assert.equal(normalizeIp("fe80::1%eth0"), "fe80::1");
  assert.equal(normalizeIp("1.2.3.4.5"), undefined);
  assert.equal(bucketFor("2001:db8:1:2:aaaa::1"), "2001:db8:1:2::/64");
  assert.equal(bucketFor("2001:db8::1"), "2001:db8:0:0::/64");
  assert.equal(bucketFor("192.0.2.1"), "192.0.2.1");
  assert.ok(isTrustedProxyAddress("10.1.2.3"));
  assert.ok(isTrustedProxyAddress("172.31.0.1"));
  assert.ok(!isTrustedProxyAddress("172.32.0.1"));
  assert.ok(isTrustedProxyAddress("fd00::1"));
  assert.ok(!isTrustedProxyAddress("2001:db8::1"));
});

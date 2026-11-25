export type UpstreamProbe = {
  name: string;
  url?: string;
  ok: boolean;
  statusCode?: number;
  latencyMs: number;
  message?: string;
};

export type PublicStatus = {
  status: "ok" | "degraded";
  service: string;
  phase: string;
  checkedAt: string;
  upstreams: UpstreamProbe[];
  chain: {
    height: number | null;
    finalizedHeight: number | null;
    validators: number | null;
    packages: number | null;
    finalizationLag: number | null;
  };
  faucet: {
    tokenDripsAvailable: boolean | null;
    nativeDripsAvailable: boolean | null;
    totalDrips: number | null;
    cooldownSeconds: number | null;
    tokenReserve: string | null;
    nativeReserve: string | null;
  };
};

export async function fetchPublicStatus(
  signal?: AbortSignal,
): Promise<PublicStatus> {
  const response = await fetch("/api/status/public", {
    headers: { accept: "application/json" },
    signal,
  });

  if (!response.ok) {
    throw new Error(`Hub status unavailable (HTTP ${response.status})`);
  }

  return (await response.json()) as PublicStatus;
}

export type IndexQuarantinePreview = {
  canonical: string;
  reason_code: string;
  expires_at_unix: number;
};

export type IndexBountyProverPreview = {
  prover_address: string;
  awards: number;
  prover_bonus_wei: string;
  committee_reserved_wei: string;
};

export type IndexDiversityPreview = {
  epoch: number;
  observations: number;
  hybrid_signers: number;
  classical_signers: number;
  model_families: Array<{ model_family: string; votes: number }>;
};

export type IndexSummary = {
  available: boolean;
  checkedAt: string;
  serverTimeUnix: number | null;
  quarantineCount: number | null;
  activeQuarantines: IndexQuarantinePreview[];
  bountyProvers: IndexBountyProverPreview[];
  committeeDiversity: IndexDiversityPreview | null;
};

export async function fetchIndexSummary(
  signal?: AbortSignal,
): Promise<IndexSummary | null> {
  try {
    const response = await fetch("/api/v1/index/summary", {
      headers: { accept: "application/json" },
      signal,
    });
    if (!response.ok) return null;
    return (await response.json()) as IndexSummary;
  } catch {
    return null;
  }
}

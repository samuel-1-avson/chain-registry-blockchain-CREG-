export type UpstreamProbe = {
  name: string;
  url: string;
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

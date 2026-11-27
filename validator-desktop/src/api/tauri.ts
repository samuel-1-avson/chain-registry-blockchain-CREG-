import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type RuntimeMode = "docker_compose" | "local_supervisor";

export type EnvironmentInfo = {
  repo_root: string | null;
  compose_file: string | null;
  docker_available: boolean;
  supervisor_binary: string | null;
  creg_cli_available: boolean;
  default_operator_key_file: string;
  default_ai_config_path: string;
};

export type RuntimeContainerStatus = {
  name: string;
  state: string;
  status: string;
};

export type RuntimeStatus = {
  mode: RuntimeMode;
  running: boolean;
  pid: number | null;
  started_at_unix: number | null;
  containers: RuntimeContainerStatus[];
  last_error: string | null;
};

export type R7AdmissionSummary = {
  stage: string;
  stage_reason: string;
  l1_approvals_collected: number;
  l1_approvals_required: number;
  transition_approvals_collected: number;
  transition_approvals_required: number;
  l1_tx_status: string;
  l1_tx_hash: string | null;
  l1_confirmed_block: number | null;
  voting_activation_height: number | null;
  first_finalized_vote_height: number | null;
  missing_signer_setup: string[];
  unavailable_reason: string | null;
};

export type R7RuntimeStatus = {
  node_id: string;
  container_state: string;
  health: {
    is_validator: boolean;
    synced: boolean;
    finalized_height: number;
  } | null;
  admission: R7AdmissionSummary | null;
};

export type ZoneCapabilitiesV1 =
  | {
      kind: "gateway";
      public_listen: string;
      admin_listen: string;
      ipc_listen: string;
      rate_limit_max: number;
      rate_limit_window_secs: number;
      admin_token_configured: boolean;
      consensus_ipc_reachable: boolean;
    }
  | {
      kind: "consensus";
      node_id: string;
      chain_id: string;
      tip_height: number;
      hybrid_active: boolean;
      signing_backend: string;
      active_validators: number;
    }
  | {
      kind: "sandbox";
      selected_engine: string;
      available_engines: string[];
      fallback_active: boolean;
      rootfs_configured: boolean;
      max_timeout_secs: number;
    }
  | {
      kind: "agent_runner";
      ai_mode: string;
      configured_providers: string[];
      distinct_model_families: number;
      diversity_warning: string | null;
      daily_budget_usd: number;
      spent_today_usd: number;
      ipfs_configured: boolean;
      sandbox_ipc_configured: boolean;
    };

export type ZoneCapabilityReportV1 = {
  schema_version: number;
  zone: string;
  status: string;
  capabilities: ZoneCapabilitiesV1;
};

export type ZoneProbeStatus = {
  zone: string;
  endpoint: string;
  reachable: boolean;
  healthy: boolean;
  status: string | null;
  capabilities: ZoneCapabilityReportV1 | null;
  error: string | null;
};

export type AiProviderCheckV1 = {
  name: string;
  kind: string;
  model: string;
  model_family: string;
  base_url: string;
  endpoint_class: string | null;
  url_ok: boolean;
  secret_scheme: string | null;
  secret_ok: boolean;
  issues: string[];
};

export type AiConfigCheckReportV1 = {
  schema_version: number;
  config_path: string | null;
  mode: string;
  ok: boolean;
  distinct_model_families: number;
  diversity_advisory: string | null;
  providers: AiProviderCheckV1[];
  errors: string[];
  warnings: string[];
};

export type DesktopDoctorReport = {
  schema_version: number;
  ok: boolean;
  operator_key_file: string | null;
  zones: ZoneProbeStatus[];
  ai_config: AiConfigCheckReportV1 | null;
  errors: string[];
  warnings: string[];
};

export type ProviderProbeReportV1 = {
  schema_version: number;
  ok: boolean;
  providers: Array<{
    provider: string;
    kind: string;
    model: string;
    model_family: string;
    endpoint_class: string;
    ok: boolean;
    latency_ms: number;
    valid_tool_json: boolean;
    error: string | null;
  }>;
};

export type NodeOverview = {
  ok: boolean;
  chain_id: string | null;
  tip_height: number | null;
  finalized_height: number | null;
  active_validators: number | null;
  package_count: number | null;
  status_text: string;
};

export type CommitteeSummaryV1 = {
  effective_size: number;
  approvals: number;
  quarantine_reviews: number;
  rejections: number;
  abstentions: number;
  model_families: Record<string, number>;
  hybrid_signers: number;
  classical_signers: number;
};

export type QuarantineQueueV1 = {
  server_time_unix: number;
  truncated: boolean;
  entries: Array<{
    canonical: string;
    content_hash: string;
    reason_code: string;
    opened_height: number;
    opened_at_unix: number;
    expires_at_unix: number;
    committee: CommitteeSummaryV1;
  }>;
};

export type CommitteeDiversityV1 = {
  epoch: number;
  validator_set_hash: string;
  observations: number;
  approvals: number;
  quarantine_reviews: number;
  rejections: number;
  abstentions: number;
  hybrid_signers: number;
  classical_signers: number;
  model_families: Array<{ model_family: string; votes: number }>;
  model_families_truncated: boolean;
};

export type PackageEvidenceBundle = {
  canonical: string;
  status_v4: {
    schema: string;
    canonical: string;
    content_hash: string;
    quorum: string;
    status: {
      state: string;
      height?: number;
      expires_at_unix?: number;
      reason_code?: string;
      authority?: string;
      reason?: string;
      witness_digest?: string;
      certificate_digest?: string;
      committee?: CommitteeSummaryV1;
    };
    committee?: CommitteeSummaryV1;
    witness_digests: string[];
  } | null;
  witnesses: {
    schema: string;
    canonical: string;
    witnesses: Array<{
      digest: string;
      prover_id: string;
      prover_key_id: string;
      hybrid: boolean;
      env_overrides: Array<[string, string]>;
      clock_offset_secs: number;
      entry_function: string | null;
      args: string[];
      violations: Array<{ code: string | null; title: string; unknown: string | null }>;
      graph_canonical_digest: string;
      verification: { kind: string; height?: number };
    }>;
  } | null;
  history: {
    canonical: string;
    entries: Array<{
      height: number;
      tx_index: number;
      source_tx_type: string;
      status: string | null;
      reason_code: string | null;
      reason: string | null;
      revocation_authority: string | null;
      witness_digest: string | null;
      certificate_digest: string | null;
    }>;
  } | null;
  warnings: string[];
};

export type SseEnvelopeV2 = {
  id: string;
  kind: string;
  height: number;
  tx_index: number;
  canonical?: string | null;
  data: unknown;
};

export function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export const desktopApi = {
  detectEnvironment: () => invoke<EnvironmentInfo>("detect_environment"),
  openValidatorOnboarding: () => invoke<void>("open_validator_onboarding"),
  getR7RuntimeStatus: () => invoke<R7RuntimeStatus[]>("get_r7_runtime_status"),
  getRuntimeStatus: (mode?: RuntimeMode, composeFile?: string) =>
    invoke<RuntimeStatus>("get_runtime_status", { mode, composeFile }),
  startValidator: (args: {
    mode: RuntimeMode;
    composeFile?: string;
    supervisorBinary?: string;
    args?: string[];
  }) => invoke<RuntimeStatus>("start_validator", args),
  stopValidator: (mode: RuntimeMode, composeFile?: string) =>
    invoke<RuntimeStatus>("stop_validator", { mode, composeFile }),
  getRuntimeLogs: (mode: RuntimeMode, composeFile?: string, tail = 200) =>
    invoke<string[]>("get_runtime_logs", { mode, composeFile, tail }),
  runValidatorDoctor: (operatorKeyFile?: string, aiConfigPath?: string) =>
    invoke<DesktopDoctorReport>("run_validator_doctor", {
      operatorKeyFile,
      aiConfigPath,
    }),
  checkAiConfig: (configPath?: string, tomlText?: string) =>
    invoke<AiConfigCheckReportV1>("check_ai_config", { configPath, tomlText }),
  readAiConfig: (configPath: string) =>
    invoke<string>("read_ai_config", { configPath }),
  saveAiConfig: (configPath: string, tomlText: string) =>
    invoke<AiConfigCheckReportV1>("save_ai_config", { configPath, tomlText }),
  probeAiProviders: (operatorKeyFile?: string, provider?: string) =>
    invoke<ProviderProbeReportV1>("probe_ai_providers", {
      operatorKeyFile,
      provider,
    }),
  fetchNodeOverview: (nodeUrl: string) =>
    invoke<NodeOverview>("fetch_node_overview", { nodeUrl }),
  fetchQuarantineQueue: (nodeUrl: string, indexerUrl?: string) =>
    invoke<QuarantineQueueV1>("fetch_quarantine_queue", {
      nodeUrl,
      indexerUrl,
    }),
  fetchCommitteeDiversity: (indexerUrl: string, epoch: number) =>
    invoke<CommitteeDiversityV1>("fetch_committee_diversity", {
      indexerUrl,
      epoch,
    }),
  fetchPackageEvidence: (
    nodeUrl: string,
    indexerUrl: string | undefined,
    canonical: string,
  ) =>
    invoke<PackageEvidenceBundle>("fetch_package_evidence", {
      nodeUrl,
      indexerUrl,
      canonical,
    }),
  startSseStream: (streamUrl: string) =>
    invoke<void>("start_sse_stream", { streamUrl }),
  stopSseStream: () => invoke<void>("stop_sse_stream"),
  onSseEvent: (cb: (env: SseEnvelopeV2) => void): Promise<UnlistenFn> =>
    listen<SseEnvelopeV2>("creg://sse-event", (e) => cb(e.payload)),
  onSseState: (cb: (state: string) => void): Promise<UnlistenFn> =>
    listen<string>("creg://sse-state", (e) => cb(e.payload)),
};

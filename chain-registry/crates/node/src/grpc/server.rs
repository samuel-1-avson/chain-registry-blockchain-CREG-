// crates/node/src/grpc/server.rs
// Implementation of the gRPC Services defined in node.proto.
#![allow(dead_code)] // gRPC server handlers invoked by creg-node main binary.

use futures::Stream;
use sha2::Digest;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status};

const MAX_GRPC_WATCHERS: usize = 128;

use crate::{events::EventBus, SharedState};
use common::proto::{
    explorer_service_server::ExplorerService, registry_service_server::RegistryService,
    watch_service_server::WatchService,
};
use common::proto::{
    BlockRequest, BlockResponse, ChainStats as ProtoStats, Empty, GetVersionRequest,
    GetVersionResponse, PublishChunk, RegistryEvent as ProtoEvent, SubmitRequest, SubmitResponse,
    WatchRequest,
};

/// ISSUE-011 / CR-02 / ADR-009: leftover publisher Groth16 fields must be
/// empty/zero. Ignore-and-admit would still let placeholder proofs ride the
/// wire; reject so admission cannot be mistaken for ZK qualification.
#[allow(clippy::result_large_err)]
fn reject_retired_publisher_zk_attestation(req: &SubmitRequest) -> Result<(), Status> {
    if req.publisher_attestation_proof.is_empty()
        && req.claimed_static_analysis_score == 0
        && !req.claimed_sandbox_safe
    {
        return Ok(());
    }
    Err(Status::invalid_argument(
        "publisher ZK attestation fields are retired; send an empty proof, score 0, and \
         claimed_sandbox_safe=false. Validators perform independent analysis.",
    ))
}

fn admission_status(error: crate::package_admission::AdmissionError) -> Status {
    use crate::package_admission::{AdmissionError, PublisherAdmissionError};

    let message = error.to_string();
    match error {
        AdmissionError::InvalidPackageId(_)
        | AdmissionError::ShieldedPublishDisabled(_)
        | AdmissionError::Scanner(
            crate::admission_scan::AdmissionScanError::Rejected { .. }
            | crate::admission_scan::AdmissionScanError::ContentHashMismatch { .. },
        )
        | AdmissionError::Publisher(PublisherAdmissionError::InvalidAddress(_)) => {
            Status::invalid_argument(message)
        }
        AdmissionError::InvalidPublisherSignature(_) => Status::permission_denied(message),
        AdmissionError::Publisher(PublisherAdmissionError::Unstaked(_)) => {
            Status::permission_denied(message)
        }
        AdmissionError::Publisher(PublisherAdmissionError::Unavailable(_))
        | AdmissionError::Scanner(crate::admission_scan::AdmissionScanError::IpfsFetch {
            ..
        }) => Status::unavailable(message),
        AdmissionError::Scanner(crate::admission_scan::AdmissionScanError::PayloadTooLarge {
            ..
        }) => Status::resource_exhausted(message),
        AdmissionError::Scanner(
            crate::admission_scan::AdmissionScanError::RulesUnavailable
            | crate::admission_scan::AdmissionScanError::ExtractionFailed { .. },
        )
        | AdmissionError::CertifiedCommitteeRequired(_)
        | AdmissionError::Revoked(_) => Status::failed_precondition(message),
        AdmissionError::AlreadyVerified(_)
        | AdmissionError::AlreadyPending(_)
        | AdmissionError::NamespaceOwned(_) => Status::already_exists(message),
        AdmissionError::Storage(_) => Status::internal(message),
    }
}

pub struct MyRegistry {
    state: SharedState,
}

impl MyRegistry {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl RegistryService for MyRegistry {
    async fn get_latest_version(
        &self,
        request: Request<GetVersionRequest>,
    ) -> Result<Response<GetVersionResponse>, Status> {
        let req = request.into_inner();
        let s = self.state.read().await;

        match s.chain.get_latest_version(&req.ecosystem, &req.name) {
            Ok(Some(record)) => Ok(Response::new(GetVersionResponse {
                found: true,
                version: record.id.version,
                content_hash: record.content_hash,
                status: format!("{:?}", record.status),
            })),
            Ok(None) => Ok(Response::new(GetVersionResponse {
                found: false,
                ..Default::default()
            })),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn submit_package(
        &self,
        request: Request<SubmitRequest>,
    ) -> Result<Response<SubmitResponse>, Status> {
        let req = request.into_inner();
        let publish_req = parse_submit_request_to_publish_request(req)?;

        let receipt = crate::package_admission::admit_publish_request(
            &self.state,
            publish_req,
            crate::package_admission::AdmissionOptions {
                surface: crate::package_admission::AdmissionSurface::Grpc,
                verify_publisher_auth: true,
            },
        )
        .await
        .map_err(admission_status)?;

        Ok(Response::new(SubmitResponse {
            accepted: true,
            message: format!(
                "{} accepted into pending pool for independent validator review ({} pending)",
                receipt.canonical, receipt.pending_count
            ),
        }))
    }

    async fn stream_publish_package(
        &self,
        request: Request<tonic::Streaming<PublishChunk>>,
    ) -> Result<Response<SubmitResponse>, Status> {
        let mut stream = request.into_inner();
        let first_chunk = stream
            .message()
            .await?
            .ok_or_else(|| Status::invalid_argument("Empty publish stream"))?;

        let metadata = match first_chunk.payload {
            Some(common::proto::publish_chunk::Payload::Metadata(meta)) => meta,
            _ => {
                return Err(Status::invalid_argument(
                    "First stream chunk must contain metadata",
                ))
            }
        };

        let publish_req = parse_submit_request_to_publish_request(metadata)?;
        if publish_req.content_hash.trim().is_empty() {
            return Err(Status::invalid_argument(
                "Stream metadata must declare content_hash before streaming body chunks",
            ));
        }

        // Authenticate publisher & stake on Chunk 0 BEFORE buffering/hashing
        // any streamed payload bytes (VNA-04, VNA-31).
        crate::package_admission::verify_publish_auth(&self.state, &publish_req)
            .await
            .map_err(admission_status)?;

        let mut total_bytes = 0usize;
        let mut hasher = sha2::Sha256::new();
        const MAX_STREAM_BYTES: usize = 64 * 1024 * 1024; // 64 MB

        while let Some(chunk) = stream.message().await? {
            if let Some(common::proto::publish_chunk::Payload::ChunkData(bytes)) = chunk.payload {
                total_bytes = total_bytes.saturating_add(bytes.len());
                if total_bytes > MAX_STREAM_BYTES {
                    return Err(Status::resource_exhausted(format!(
                        "Streamed payload exceeds maximum size limit ({} MB)",
                        MAX_STREAM_BYTES / (1024 * 1024)
                    )));
                }
                hasher.update(&bytes);
            }
        }

        if total_bytes > 0 {
            let actual_hash = hex::encode(hasher.finalize());
            if !publish_req.content_hash.eq_ignore_ascii_case(&actual_hash) {
                return Err(Status::invalid_argument(format!(
                    "Streamed content hash mismatch: declared {}, actual {}",
                    publish_req.content_hash, actual_hash
                )));
            }
        }

        let receipt = crate::package_admission::admit_publish_request(
            &self.state,
            publish_req,
            crate::package_admission::AdmissionOptions {
                surface: crate::package_admission::AdmissionSurface::Grpc,
                verify_publisher_auth: false,
            },
        )
        .await
        .map_err(admission_status)?;

        Ok(Response::new(SubmitResponse {
            accepted: true,
            message: format!(
                "{} accepted into pending pool via binary stream ({} pending)",
                receipt.canonical, receipt.pending_count
            ),
        }))
    }
}

#[allow(clippy::result_large_err)]
fn parse_submit_request_to_publish_request(
    req: SubmitRequest,
) -> Result<common::PublishRequest, Status> {
    reject_retired_publisher_zk_attestation(&req)?;

    let manifest = if req.manifest_json.trim().is_empty() {
        common::PackageManifest::default()
    } else {
        serde_json::from_str::<common::PackageManifest>(&req.manifest_json)
            .map_err(|e| Status::invalid_argument(format!("Invalid manifest payload: {}", e)))?
    };

    let derived_manifest_hash_hex = common::sha256_hex(
        &serde_json::to_vec(&manifest)
            .map_err(|e| Status::internal(format!("Serialize manifest: {}", e)))?,
    );
    if !req.manifest_hash.trim().is_empty()
        && !req
            .manifest_hash
            .eq_ignore_ascii_case(&derived_manifest_hash_hex)
    {
        return Err(Status::invalid_argument(
            "manifest_hash does not match manifest_json",
        ));
    }
    let pkg_id = common::PackageId::new(&req.ecosystem, &req.name, &req.version);
    let ownership_proof = if req.ownership_proof_json.trim().is_empty() {
        None
    } else {
        Some(
            serde_json::from_str(&req.ownership_proof_json).map_err(|error| {
                Status::invalid_argument(format!("Invalid ownership proof: {error}"))
            })?,
        )
    };
    let namespace_authority_certificate =
        if req.namespace_authority_certificate_json.trim().is_empty() {
            None
        } else {
            Some(
                serde_json::from_str(&req.namespace_authority_certificate_json).map_err(
                    |error| {
                        Status::invalid_argument(format!(
                            "Invalid namespace authority certificate: {error}"
                        ))
                    },
                )?,
            )
        };
    let submitted_at = if req.publisher_issued_at == 0 {
        if !req.publisher_evm_signature.trim().is_empty() {
            return Err(Status::invalid_argument(
                "publisher_issued_at is required for a dual-key publisher proof",
            ));
        }
        chrono::Utc::now()
    } else {
        chrono::DateTime::<chrono::Utc>::from_timestamp(req.publisher_issued_at, 0)
            .ok_or_else(|| Status::invalid_argument("publisher_issued_at is invalid"))?
    };
    Ok(common::PublishRequest {
        id: pkg_id,
        content_hash: req.content_hash.clone(),
        ipfs_cid: req.ipfs_cid.clone(),
        publisher_address: common::canonical_publisher_address(&req.publisher_address),
        publisher_pubkey: req.publisher_pubkey.clone(),
        signature: req.signature.clone(),
        manifest,
        submitted_at,
        shielded: req.shielded,
        key_bundle: (!req.key_bundle.is_empty()).then_some(req.key_bundle.clone()),
        pgp_signature: (!req.pgp_signature.is_empty()).then_some(req.pgp_signature.clone()),
        pgp_public_key: (!req.pgp_public_key.is_empty()).then_some(req.pgp_public_key.clone()),
        publisher_pubkeys: req.publisher_pubkeys.clone(),
        signatures: req.signatures.clone(),
        threshold: req.threshold as usize,
        publisher_evm_signature: req.publisher_evm_signature.clone(),
        publisher_nonce: req.publisher_nonce.clone(),
        publisher_expires_at: req.publisher_expires_at,
        source_hash: req.source_hash.clone(),
        manifest_hash: derived_manifest_hash_hex,
        ownership_claim: req.ownership_claim.clone(),
        publisher_chain_id: req.publisher_chain_id.clone(),
        publisher_genesis_hash: req.publisher_genesis_hash.clone(),
        publisher_staking_contract: req.publisher_staking_contract.clone(),
        publisher_authorization_version: req.publisher_authorization_version as u16,
        ownership_proof,
        namespace_authority_certificate,
    })
}

fn event_matches_filter(event: &crate::events::RegistryEvent, filter: &str) -> bool {
    let trimmed = filter.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("all") {
        return true;
    }
    if trimmed.eq_ignore_ascii_case("packages") {
        return matches!(
            event.kind,
            crate::events::EventKind::PackageSubmitted
                | crate::events::EventKind::PackageVerified
                | crate::events::EventKind::PackageRejected
                | crate::events::EventKind::PackageRevoked
        );
    }
    if trimmed.eq_ignore_ascii_case("blocks") {
        return matches!(event.kind, crate::events::EventKind::BlockProduced);
    }
    if trimmed.eq_ignore_ascii_case("votes") {
        return matches!(event.kind, crate::events::EventKind::ValidatorVoted);
    }
    let needle = trimmed.to_ascii_lowercase();
    let kind_name = format!("{:?}", event.kind).to_ascii_lowercase();
    let kind_snake = event.kind.to_string().to_ascii_lowercase();
    kind_name.contains(&needle) || kind_snake.contains(&needle)
}

pub struct MyWatcher {
    event_bus: EventBus,
    watcher_slots: Arc<Semaphore>,
}

#[allow(dead_code)]
impl MyWatcher {
    pub fn new(event_bus: EventBus) -> Self {
        Self::with_max_watchers(event_bus, MAX_GRPC_WATCHERS)
    }

    pub fn with_max_watchers(event_bus: EventBus, max_watchers: usize) -> Self {
        Self {
            event_bus,
            watcher_slots: Arc::new(Semaphore::new(max_watchers.max(1))),
        }
    }
}

#[tonic::async_trait]
impl WatchService for MyWatcher {
    type StreamEventsStream = Pin<Box<dyn Stream<Item = Result<ProtoEvent, Status>> + Send>>;

    async fn stream_events(
        &self,
        request: Request<WatchRequest>,
    ) -> Result<Response<Self::StreamEventsStream>, Status> {
        let permit = self
            .watcher_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("too many active gRPC event watchers"))?;
        let filter = request.into_inner().filter;
        let rx = self.event_bus.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(move |res| {
            let _keep_permit_alive = &permit;
            match res {
                Ok(event) => {
                    if !event_matches_filter(&event, &filter) {
                        return None;
                    }
                    Some(Ok(ProtoEvent {
                        kind: format!("{:?}", event.kind),
                        payload_json: serde_json::to_string(&event.payload).unwrap_or_default(),
                        timestamp: None, // In production, convert chrono to prost_types::Timestamp
                    }))
                }
                Err(_) => Some(Err(Status::data_loss("Stream lagged"))),
            }
        });

        Ok(Response::new(Box::pin(stream) as Self::StreamEventsStream))
    }
}

pub struct MyExplorer {
    state: SharedState,
}

impl MyExplorer {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl ExplorerService for MyExplorer {
    async fn get_chain_stats(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<ProtoStats>, Status> {
        let s = self.state.read().await;
        let stats = s.chain.stats();

        Ok(Response::new(ProtoStats {
            tip_height: stats.tip_height,
            tip_hash: stats.tip_hash,
            package_count: stats.package_count as u32,
            block_count: stats.block_count as u32,
        }))
    }

    async fn get_block_by_height(
        &self,
        request: Request<BlockRequest>,
    ) -> Result<Response<BlockResponse>, Status> {
        let req = request.into_inner();
        let s = self.state.read().await;

        match s.chain.get_block_by_height(req.height) {
            Ok(Some(block)) => Ok(Response::new(BlockResponse {
                height: block.header.height,
                hash: block.hash(),
                prev_hash: block.header.prev_hash,
                merkle_root: block.header.merkle_root,
            })),
            Ok(None) => Err(Status::not_found("Block not found")),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{reject_retired_publisher_zk_attestation, MyWatcher};
    use crate::events::{emit, new_event_bus, RegistryEvent};
    use common::proto::{watch_service_server::WatchService, SubmitRequest, WatchRequest};
    use tokio_stream::StreamExt;
    use tonic::Request;

    #[tokio::test]
    async fn my_watcher_streams_from_injected_event_bus() -> anyhow::Result<()> {
        let event_bus = new_event_bus();
        let watcher = MyWatcher::new(event_bus.clone());
        let response = watcher
            .stream_events(Request::new(WatchRequest {
                filter: "all".into(),
            }))
            .await?;

        emit(
            &event_bus,
            RegistryEvent::package_revoked("npm:test@1.0.0", "malware detected", "pubkey-123"),
        );

        let mut stream = response.into_inner();
        let event = stream
            .next()
            .await
            .expect("watch stream should yield an event")?;

        assert_eq!(event.kind, "PackageRevoked");
        assert!(event.payload_json.contains("malware detected"));
        assert!(event.payload_json.contains("pubkey-123"));

        Ok(())
    }

    #[test]
    fn retired_zk_attestation_accepts_empty_zero_form() {
        let req = SubmitRequest {
            publisher_attestation_proof: vec![],
            claimed_static_analysis_score: 0,
            claimed_sandbox_safe: false,
            ..Default::default()
        };
        assert!(reject_retired_publisher_zk_attestation(&req).is_ok());
    }

    #[test]
    fn retired_zk_attestation_rejects_placeholder_score_and_proof() {
        let leftover_score = SubmitRequest {
            claimed_static_analysis_score: 85,
            claimed_sandbox_safe: true,
            ..Default::default()
        };
        let err = reject_retired_publisher_zk_attestation(&leftover_score).unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("retired"));

        let leftover_proof = SubmitRequest {
            publisher_attestation_proof: vec![0xde, 0xad],
            ..Default::default()
        };
        assert!(reject_retired_publisher_zk_attestation(&leftover_proof).is_err());
    }

    #[tokio::test]
    async fn stream_publish_package_rejects_empty_stream() {
        use common::proto::registry_service_server::RegistryService;
        let tempdir = tempfile::tempdir().unwrap();
        let chain = crate::chain_store::ChainStore::open(tempdir.path()).unwrap();
        let state = std::sync::Arc::new(tokio::sync::RwLock::new(crate::NodeState {
            chain,
            pending_pool: crate::pending_pool::PendingPool::new(),
            publisher_index: crate::publisher_index::PublisherIndex::new(),
            validator_set_bootstrap: common::ValidatorSet::default(),
            validator_set: common::ValidatorSet::default(),
            validator_set_snapshot: std::sync::Arc::new(crate::state::ArcSwap::from_pointee(
                common::ValidatorSet::default(),
            )),
            package_rounds: std::collections::HashMap::new(),
            package_committees: std::collections::HashMap::new(),
            config: crate::config::NodeConfig {
                data_dir: tempdir.path().to_path_buf(),
                ..Default::default()
            },
            p2p_status: crate::P2PStatus::default(),
            bridge_status: crate::BridgeStatus::default(),
            vrf_proofs: std::collections::HashMap::new(),
            decryption_shares: std::collections::HashMap::new(),
            validator_registrations: std::collections::HashMap::new(),
            validator_set_sync: crate::state::ValidatorSetSyncStatus::default(),
            view_change_certs: std::collections::HashMap::new(),
            reorgs: Vec::new(),
            pbft_engine: crate::state::PbftEngine::new(),
            forced_inclusion_tracker: crate::state::ForcedInclusionTracker::new(),
            sync_lag_blocks: 0,
            sync_max_peer_tip: 0,
            reward_emissions_succeeded: 0,
            reward_emissions_failed: 0,
            reward_emissions_skipped: 0,
            treasury_balance: 0,
            pending_rewards_estimate: 0,
            total_rewards_distributed: 0,
            rewards_pool_balance: 0,
            last_incentives_refresh: None,
            publisher_stake_cache: std::collections::HashMap::new(),
            equivocation: crate::equivocation_monitor::EquivocationMonitor::disabled(),
        }));

        let registry = super::MyRegistry::new(state);
        use tonic::codec::Codec;
        let mut codec = tonic::codec::ProstCodec::<
            common::proto::PublishChunk,
            common::proto::PublishChunk,
        >::default();
        let empty_stream =
            tonic::Streaming::new_empty(codec.decoder(), tonic::body::BoxBody::default());
        let err = registry
            .stream_publish_package(tonic::Request::new(empty_stream))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn my_watcher_enforces_filter_and_concurrency_cap() -> anyhow::Result<()> {
        let event_bus = new_event_bus();
        let watcher = MyWatcher::with_max_watchers(event_bus.clone(), 1);
        let response = watcher
            .stream_events(Request::new(WatchRequest {
                filter: "blocks".into(),
            }))
            .await?;

        let Err(busy) = watcher
            .stream_events(Request::new(WatchRequest {
                filter: "all".into(),
            }))
            .await
        else {
            panic!("expected ResourceExhausted when watcher cap reached");
        };
        assert_eq!(busy.code(), tonic::Code::ResourceExhausted);

        emit(
            &event_bus,
            RegistryEvent::package_submitted("npm:ignored@1.0.0", "pubkey-a"),
        );
        emit(&event_bus, RegistryEvent::block_produced(7, "hash7", 1));

        let mut stream = response.into_inner();
        let first = stream
            .next()
            .await
            .expect("filtered watch stream should yield block event")?;
        assert_eq!(first.kind, "BlockProduced");

        drop(stream);
        let reacquired = watcher
            .stream_events(Request::new(WatchRequest {
                filter: "packages".into(),
            }))
            .await;
        assert!(reacquired.is_ok(), "dropping stream must release permit");
        Ok(())
    }
}

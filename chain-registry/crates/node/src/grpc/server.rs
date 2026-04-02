// crates/node/src/grpc/server.rs
// Implementation of the gRPC Services defined in node.proto.

use tonic::{Request, Response, Status};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use std::pin::Pin;
use futures::Stream;

use common::proto::{
    registry_service_server::RegistryService,
    watch_service_server::WatchService,
    explorer_service_server::ExplorerService,
};
use common::proto::{
    GetVersionRequest, GetVersionResponse, SubmitRequest, SubmitResponse,
    WatchRequest, RegistryEvent as ProtoEvent, ChainStats as ProtoStats,
    BlockRequest, BlockResponse, Empty
};
use crate::SharedState;

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
            Ok(Some(record)) => {
                Ok(Response::new(GetVersionResponse {
                    found: true,
                    version: record.id.version,
                    content_hash: record.content_hash,
                    status: format!("{:?}", record.status),
                }))
            }
            Ok(None) => Ok(Response::new(GetVersionResponse { found: false, ..Default::default() })),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn submit_package(
        &self,
        request: Request<SubmitRequest>,
    ) -> Result<Response<SubmitResponse>, Status> {
        let req = request.into_inner();
        let s = self.state.read().await;

        // ── 1. ZK-Proof Verification (L2 Safety Hardening) ────────────────────
        if req.zk_proof.is_empty() {
            return Err(Status::unauthenticated("Missing mandatory ZK safety proof"));
        }

        // Build Public Inputs for the ZK Circuit
        // Note: In production, we'd also verify the content_hash matches the proof.
        let mut content_hash_bytes = [0u8; 32];
        if let Ok(hash_vec) = hex::decode(&req.content_hash) {
            if hash_vec.len() == 32 {
                content_hash_bytes.copy_from_slice(&hash_vec);
            }
        }

        let inputs = zk_validator::PackageInputs::new(
            content_hash_bytes,
            [0u8; 32], // Sub-manifest hash (omitted for alpha simplicity)
            req.static_analysis_score as u8,
            req.sandbox_safe,
        );

        // Deserialize and Verify the Proof
        match zk_validator::ZkValidator::deserialize_proof(&req.zk_proof) {
            Ok(proof) => {
                let public_inputs = inputs.public_inputs();
                match s.zk_validator.verify_proof(&proof, &public_inputs) {
                    Ok(true) => {
                        tracing::info!("[ZK] Proof verified successfully for package: {}", req.name);
                    }
                    _ => {
                        tracing::warn!("[ZK] Proof verification FAILED for package: {}", req.name);
                        return Err(Status::permission_denied("Invalid ZK safety proof. Submission rejected."));
                    }
                }
            }
            Err(e) => {
                return Err(Status::invalid_argument(format!("Failed to deserialize ZK proof: {}", e)));
            }
        }

        // ── 2. Add to Pending Pool ──────────────────────────────────────────
        let pkg_id = common::PackageId::new(&req.ecosystem, &req.name, &req.version);
        let publish_req = common::PublishRequest {
            id: pkg_id,
            content_hash: req.content_hash,
            ipfs_cid: req.ipfs_cid,
            publisher_pubkey: req.publisher_pubkey,
            signature: req.signature,
            manifest: common::PackageManifest::default(),
            submitted_at: chrono::Utc::now(),
            shielded: false,
            key_bundle: None,
            pgp_signature: None,
            pgp_public_key: None,
            publisher_pubkeys: req.publisher_pubkeys,
            signatures: req.signatures,
            threshold: req.threshold as usize,
            ..Default::default()
        };

        {
            if let Err(e) = crate::api::verify_publish_sig(&publish_req) {
                return Err(Status::permission_denied(format!("Invalid publisher signature: {}", e)));
            }
            let mut state = self.state.write().await;
            state.pending_pool.insert(publish_req);
            tracing::info!("[Consensus] Package {} added to pending pool via gRPC", req.name);
        }

        Ok(Response::new(SubmitResponse {
            accepted: true,
            message: "Package verified by ZK-SNARK and accepted into pending pool".into(),
        }))
    }
}

pub struct MyWatcher {
    state: SharedState,
}

impl MyWatcher {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl WatchService for MyWatcher {
    type StreamEventsStream = Pin<Box<dyn Stream<Item = Result<ProtoEvent, Status>> + Send>>;

    async fn stream_events(
        &self,
        _request: Request<WatchRequest>,
    ) -> Result<Response<Self::StreamEventsStream>, Status> {
        let bus = {
            let s = self.state.read().await;
            s.event_bus.clone()
        };

        let rx = bus.subscribe();
        let stream = BroadcastStream::new(rx).map(|res| {
            match res {
                Ok(event) => {
                    Ok(ProtoEvent {
                        kind: format!("{:?}", event.kind),
                        payload_json: serde_json::to_string(&event.payload).unwrap_or_default(),
                        timestamp: None, // In production, convert chrono to prost_types::Timestamp
                    })
                }
                Err(_) => Err(Status::data_loss("Stream lagged")),
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
            Ok(Some(block)) => {
                Ok(Response::new(BlockResponse {
                    height: block.header.height,
                    hash: block.hash(),
                    prev_hash: block.header.prev_hash,
                    merkle_root: block.header.merkle_root,
                }))
            }
            Ok(None) => Err(Status::not_found("Block not found")),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }
}

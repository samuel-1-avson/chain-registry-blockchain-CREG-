// crates/node/src/grpc/server.rs
// Implementation of the gRPC Services defined in node.proto.

use futures::Stream;
use std::pin::Pin;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status};

use crate::{events::EventBus, SharedState};
use common::proto::{
    explorer_service_server::ExplorerService, registry_service_server::RegistryService,
    watch_service_server::WatchService,
};
use common::proto::{
    BlockRequest, BlockResponse, ChainStats as ProtoStats, Empty, GetVersionRequest,
    GetVersionResponse, RegistryEvent as ProtoEvent, SubmitRequest, SubmitResponse, WatchRequest,
};

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
        let publish_req = common::PublishRequest {
            id: pkg_id,
            content_hash: req.content_hash.clone(),
            ipfs_cid: req.ipfs_cid.clone(),
            publisher_address: common::canonical_publisher_address(&req.publisher_address),
            publisher_pubkey: req.publisher_pubkey.clone(),
            signature: req.signature.clone(),
            manifest,
            submitted_at,
            shielded: false,
            key_bundle: None,
            pgp_signature: None,
            pgp_public_key: None,
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
            publisher_staking_contract: req.publisher_staking_contract.clone(),
            ..Default::default()
        };

        crate::package_admission::verify_publish_auth(&self.state, &publish_req)
            .await
            .map_err(admission_status)?;

        // CR-02: publisher-generated ZK attestations are retired. Their safety
        // inputs were caller assertions, not independently measured analysis.
        // Keep the protobuf fields for wire compatibility but never require,
        // verify, store, or use them to affect admission or qualification.
        if !req.publisher_attestation_proof.is_empty()
            || req.claimed_static_analysis_score != 0
            || req.claimed_sandbox_safe
        {
            tracing::warn!(
                package = %req.name,
                "Ignoring retired publisher ZK attestation fields"
            );
        }

        let receipt = crate::package_admission::admit_publish_request(
            &self.state,
            publish_req,
            crate::package_admission::AdmissionOptions {
                surface: crate::package_admission::AdmissionSurface::Grpc,
                // gRPC is a public submission surface too.  Do not bypass
                // publisher authentication merely because the transport is
                // internal-looking; callers must satisfy the same admission
                // policy as REST.
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
}

pub struct MyWatcher {
    event_bus: EventBus,
}

impl MyWatcher {
    pub fn new(event_bus: EventBus) -> Self {
        Self { event_bus }
    }
}

#[tonic::async_trait]
impl WatchService for MyWatcher {
    type StreamEventsStream = Pin<Box<dyn Stream<Item = Result<ProtoEvent, Status>> + Send>>;

    async fn stream_events(
        &self,
        _request: Request<WatchRequest>,
    ) -> Result<Response<Self::StreamEventsStream>, Status> {
        let rx = self.event_bus.subscribe();
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
    use super::MyWatcher;
    use crate::events::{emit, new_event_bus, RegistryEvent};
    use common::proto::{watch_service_server::WatchService, WatchRequest};
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
}

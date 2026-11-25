//! Transactional CREG block projection.

use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Utc};
use common::{
    Block, ChainRecord, NamespaceLifecycleAction, PackageStatus, QuarantineOutcomeV1, Transaction,
    CLASS_C_QUARANTINE_TTL_SECS,
};
use registry_view::{CommitteeSummaryV1, SseEnvelopeV2, SseEventKindV2};
use serde_json::{json, Value};
use sqlx::{PgConnection, PgPool};

const EXPIRY_REASON: &str = "Class C quarantine expired without clearance";
const REJECTION_REASON: &str = "Class C quarantine rejected by committee certificate";

/// Advance the projection exactly once. The cursor lock serializes writers and
/// also establishes the order of the indexer's CREG and L1 SSE events.
pub async fn apply_block(pool: &PgPool, block: &Block) -> Result<bool> {
    let height = i64::try_from(block.header.height)?;
    let hash = block.hash();
    let mut tx = pool.begin().await?;
    let db = &mut *tx;
    let last = lock_cursor(db).await?;
    let existing = indexed_hash(&mut *db, height).await?;
    if let Some(existing) = &existing {
        ensure!(
            *existing == hash,
            "indexed hash conflict at height {height}"
        );
    }
    if height <= last {
        ensure!(
            existing.is_some(),
            "missing indexed block at height {height}"
        );
        tx.rollback().await?;
        return Ok(false);
    }
    ensure!(
        Some(height) == last.checked_add(1),
        "block height continuity break at {height}, indexed tip is {last}"
    );

    let mut epoch = 0_i64;
    if height == 0 {
        ensure!(
            block.header.prev_hash == "0".repeat(64) && block.transactions.is_empty(),
            "invalid genesis projection"
        );
    } else {
        let (parent, set, parent_epoch): (String, String, i64) = sqlx::query_as(
            "SELECT hash, validator_set_hash, committee_epoch FROM blocks WHERE height = $1",
        )
        .bind(last)
        .fetch_one(&mut *db)
        .await
        .context("read indexed parent")?;
        ensure!(
            block.header.prev_hash == parent,
            "prev_hash continuity break at height {height}"
        );
        epoch = if height == 1 || set == block.header.validator_set_hash {
            parent_epoch
        } else {
            height
        };
    }

    let vote_v4_active = block.header.status_root.is_some();
    let expired = expire_quarantines(db, height, block.header.timestamp).await?;
    for (index, transaction) in block.transactions.iter().enumerate() {
        let position = Position {
            height,
            index: i32::try_from(index)?,
        };
        project_transaction(
            db,
            position,
            block.header.timestamp,
            &hash,
            vote_v4_active,
            transaction,
        )
        .await
        .with_context(|| format!("project transaction at {height}.{index}"))?;
    }

    // This update path is only used during v1 replay. Its hash was checked
    // above, before any projected rows were written.
    let affected = sqlx::query(
        "INSERT INTO blocks (height, hash, prev_hash, merkle_root, proposer_id, timestamp,
                             validator_set_hash, view_number, status_root, vote_v4_active,
                             commit_signatures, committee_epoch)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
         ON CONFLICT (height) DO UPDATE SET validator_set_hash = EXCLUDED.validator_set_hash,
             view_number = EXCLUDED.view_number, status_root = EXCLUDED.status_root,
             vote_v4_active = EXCLUDED.vote_v4_active, commit_signatures = EXCLUDED.commit_signatures,
             committee_epoch = EXCLUDED.committee_epoch WHERE blocks.hash = EXCLUDED.hash",
    )
    .bind(height)
    .bind(&hash)
    .bind(&block.header.prev_hash)
    .bind(&block.header.merkle_root)
    .bind(&block.header.proposer_id)
    .bind(block.header.timestamp)
    .bind(&block.header.validator_set_hash)
    .bind(i64::from(block.header.view_number))
    .bind(block.header.status_root.map(hex::encode))
    .bind(vote_v4_active)
    .bind(serde_json::to_value(&block.pbft_signatures)?)
    .bind(epoch)
    .execute(&mut *db)
    .await?
    .rows_affected();
    ensure!(affected == 1, "indexed hash conflict at height {height}");

    for event in registry_view::block_events(block) {
        insert_event(db, &event).await?;
    }
    let following = block.transactions.len() + 1;
    for (offset, (canonical, content_hash)) in expired.into_iter().enumerate() {
        let event = SseEnvelopeV2::new(
            SseEventKindV2::QuarantineExpired,
            block.header.height,
            u32::try_from(following + offset)?,
            Some(&canonical),
            json!({ "content_hash": content_hash, "reason": EXPIRY_REASON }),
        );
        insert_event(db, &event).await?;
    }

    sqlx::query("UPDATE sync_state SET last_height = $1, updated_at = NOW() WHERE id = 1")
        .bind(height)
        .execute(db)
        .await?;
    tx.commit().await?;
    Ok(true)
}

pub(crate) async fn lock_cursor(db: &mut PgConnection) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT last_height FROM sync_state WHERE id = 1 FOR UPDATE")
            .fetch_one(db)
            .await?,
    )
}

pub async fn indexed_hash(db: impl sqlx::PgExecutor<'_>, height: i64) -> Result<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT hash FROM blocks WHERE height = $1")
            .bind(height)
            .fetch_optional(db)
            .await?,
    )
}

pub(crate) async fn insert_event(db: &mut PgConnection, event: &SseEnvelopeV2) -> Result<()> {
    sqlx::query("INSERT INTO index_events (height, tx_index, envelope) VALUES ($1,$2,$3)")
        .bind(i64::try_from(event.height)?)
        .bind(i64::from(event.tx_index))
        .bind(serde_json::to_value(event)?)
        .execute(db)
        .await?;
    Ok(())
}

#[derive(Clone, Copy)]
struct Position {
    height: i64,
    index: i32,
}

async fn project_transaction(
    db: &mut PgConnection,
    pos: Position,
    now: DateTime<Utc>,
    block_hash: &str,
    vote_v4_active: bool,
    tx: &Transaction,
) -> Result<()> {
    match tx {
        Transaction::Publish(record) | Transaction::PublishAuthorizedV1 { record, .. } => {
            let source = if let Transaction::PublishAuthorizedV1 { authorization, .. } = tx {
                let namespace = common::package_namespace_prefix(&record.id);
                sqlx::query(
                    "INSERT INTO namespace_controllers VALUES ($1,$2,TRUE)
                     ON CONFLICT (package_namespace) DO UPDATE
                     SET controller_pubkey = EXCLUDED.controller_pubkey, authorized = TRUE
                     WHERE NOT namespace_controllers.authorized",
                )
                .bind(namespace)
                .bind(&authorization.publisher_ed25519_pubkey)
                .execute(&mut *db)
                .await?;
                "publish_authorized_v1"
            } else {
                "publish"
            };
            if source == "publish" {
                sqlx::query(
                    "INSERT INTO namespace_controllers VALUES ($1,$2,FALSE)
                     ON CONFLICT (package_namespace) DO NOTHING",
                )
                .bind(common::package_namespace_prefix(&record.id))
                .bind(&record.publisher_pubkey)
                .execute(&mut *db)
                .await?;
            }
            let certificate_digest = match tx {
                Transaction::PublishAuthorizedV1 { committee_qc, .. } => committee_qc
                    .as_ref()
                    .map(common::CommitteeQuorumCertificateV1::digest),
                _ => None,
            };
            project_publication(db, pos, record, block_hash, source, certificate_digest).await?;
        }
        Transaction::Revoke { .. } | Transaction::RevokeV2 { .. } => {
            project_revocation(db, pos, tx, vote_v4_active).await?;
        }
        Transaction::QuarantineV1 {
            record,
            reason_code,
            quorum_certificate,
        } => {
            let canonical = record.id.canonical();
            let deadline = DateTime::from_timestamp(
                now.timestamp().saturating_add(CLASS_C_QUARANTINE_TTL_SECS),
                0,
            )
            .context("invalid quarantine deadline")?;
            let code = wire_string(reason_code)?;
            let committee = serde_json::to_value(CommitteeSummaryV1::for_record(record))?;
            sqlx::query(
                "INSERT INTO quarantines
                     (canonical,content_hash,reason_code,opened_height,opened_at,expires_at,committee_summary)
                 VALUES ($1,$2,$3,$4,$5,$6,$7)
                 ON CONFLICT (canonical) DO UPDATE SET content_hash = EXCLUDED.content_hash,
                     reason_code = EXCLUDED.reason_code, opened_height = EXCLUDED.opened_height,
                     opened_at = EXCLUDED.opened_at, expires_at = EXCLUDED.expires_at,
                     committee_summary = EXCLUDED.committee_summary, resolved_height = NULL,
                     resolution = NULL",
            )
            .bind(&canonical).bind(&record.content_hash).bind(&code).bind(pos.height)
            .bind(now).bind(deadline).bind(&committee)
            .execute(&mut *db).await?;
            set_status(db, &canonical, "quarantined", None).await?;
            StatusChange {
                status: "quarantined",
                source: "quarantine_v1",
                code: Some(&code),
                expires: Some(deadline),
                committee: Some(committee),
                certificate_digest: Some(quorum_certificate.digest()),
                ..StatusChange::default()
            }
            .insert(db, pos, &canonical, &record.content_hash)
            .await?;
            project_votes(db, pos, record, false).await?;
        }
        Transaction::QuarantineResolutionV1 {
            package_canonical,
            content_hash,
            outcome,
            quorum_certificate,
        } => {
            let canonical = package_canonical.trim();
            let (status, resolution, reason) = match outcome {
                QuarantineOutcomeV1::Release => ("verified", "released", None),
                QuarantineOutcomeV1::Reject => ("revoked", "rejected", Some(REJECTION_REASON)),
            };
            let affected = sqlx::query(
                "UPDATE quarantines SET resolved_height = $2, resolution = $3
                 WHERE canonical = $1 AND resolution IS NULL AND lower(btrim(content_hash)) = lower(btrim($4))",
            )
            .bind(canonical)
            .bind(pos.height)
            .bind(resolution)
            .bind(content_hash)
            .execute(&mut *db)
            .await?
            .rows_affected();
            ensure!(
                affected == 1,
                "resolution targets a closed, mismatched or unknown quarantine"
            );
            set_status(db, canonical, status, reason).await?;
            StatusChange {
                status,
                source: "quarantine_resolution_v1",
                reason,
                certificate_digest: Some(quorum_certificate.digest()),
                ..StatusChange::default()
            }
            .insert(db, pos, canonical, content_hash)
            .await?;
            if reason.is_some() {
                Revocation {
                    source: "quarantine_resolution_v1",
                    authority: "committee_quorum",
                    reason,
                    certificate: Some(serde_json::to_value(quorum_certificate)?),
                    ..Revocation::default()
                }
                .insert(db, pos, canonical)
                .await?;
            }
        }
        Transaction::RotatePublisherKey {
            canonical_prefix,
            old_pubkey,
            new_pubkey,
            timestamp,
            ..
        } => {
            sqlx::query(
                "UPDATE packages p SET publisher_pubkey = $1, updated_at = $2
                 WHERE p.package_namespace = $3 AND p.publisher_pubkey = $4
                   AND EXISTS (SELECT 1 FROM package_status_history h WHERE h.canonical = p.canonical)",
            )
            .bind(new_pubkey).bind(timestamp).bind(canonical_prefix).bind(old_pubkey)
            .execute(&mut *db).await?;
            sqlx::query(
                "UPDATE namespace_controllers SET controller_pubkey = $1
                 WHERE package_namespace = $2 AND controller_pubkey = $3 AND NOT authorized",
            )
            .bind(new_pubkey)
            .bind(canonical_prefix)
            .bind(old_pubkey)
            .execute(&mut *db)
            .await?;
        }
        Transaction::ValidatorJoin {
            validator_id,
            stake,
            ..
        } => {
            validator_event(db, pos, "join", validator_id, Some(*stake), None).await?;
        }
        Transaction::ValidatorLeave { validator_id } => {
            validator_event(db, pos, "leave", validator_id, None, None).await?;
        }
        Transaction::Slash {
            validator_id,
            amount,
            reason,
        } => {
            validator_event(db, pos, "slash", validator_id, Some(*amount), Some(reason)).await?;
        }
        Transaction::NamespaceLifecycle(event) => {
            // Legacy lifecycle effects are conditional, even in a valid block.
            if event.action != NamespaceLifecycleAction::Recover
                && event.is_valid_at(now)
                && event.verify_controller_signatures().is_ok()
            {
                sqlx::query(
                    "UPDATE namespace_controllers SET controller_pubkey = $2
                     WHERE package_namespace = $1 AND NOT authorized AND controller_pubkey = $3",
                )
                .bind(&event.namespace)
                .bind(&event.next_controller_ed25519_pubkey)
                .bind(&event.current_controller_ed25519_pubkey)
                .execute(&mut *db)
                .await?;
            }
        }
        Transaction::NamespaceLifecycleV2(event) => {
            if event.action != NamespaceLifecycleAction::Delegate {
                sqlx::query(
                    "UPDATE namespace_controllers SET controller_pubkey = $1
                     WHERE package_namespace = $2 AND controller_pubkey = $3",
                )
                .bind(&event.next_controller_ed25519_pubkey)
                .bind(&event.namespace)
                .bind(&event.current_controller_ed25519_pubkey)
                .execute(&mut *db)
                .await?;
            }
        }
    }
    Ok(())
}

#[derive(Default)]
struct StatusChange<'a> {
    status: &'a str,
    source: &'a str,
    code: Option<&'a str>,
    reason: Option<&'a str>,
    expires: Option<DateTime<Utc>>,
    committee: Option<Value>,
    certificate_digest: Option<String>,
}

impl StatusChange<'_> {
    async fn insert(
        self,
        db: &mut PgConnection,
        pos: Position,
        canonical: &str,
        content_hash: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO package_status_history (canonical,content_hash,status,reason_code,reason,
                 height,tx_index,quarantine_expires_at,source_tx_type,committee_summary,certificate_digest)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
        )
        .bind(canonical)
        .bind(content_hash)
        .bind(self.status)
        .bind(self.code)
        .bind(self.reason)
        .bind(pos.height)
        .bind(pos.index)
        .bind(self.expires)
        .bind(self.source)
        .bind(self.committee)
        .bind(self.certificate_digest)
        .execute(db)
        .await?;
        Ok(())
    }
}

async fn set_status(
    db: &mut PgConnection,
    canonical: &str,
    status: &str,
    reason: Option<&str>,
) -> Result<()> {
    sqlx::query("UPDATE packages SET status = $2, revocation_reason = $3, updated_at = NOW() WHERE canonical = $1")
        .bind(canonical).bind(status).bind(reason).execute(db).await?;
    Ok(())
}

async fn resolve_quarantine(
    db: &mut PgConnection,
    canonical: &str,
    height: i64,
    resolution: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE quarantines SET resolved_height = $2, resolution = $3
         WHERE canonical = $1 AND resolution IS NULL",
    )
    .bind(canonical)
    .bind(height)
    .bind(resolution)
    .execute(db)
    .await?;
    Ok(())
}

async fn expire_quarantines(
    db: &mut PgConnection,
    height: i64,
    now: DateTime<Utc>,
) -> Result<Vec<(String, String)>> {
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT canonical, content_hash FROM v_quarantine_queue WHERE expires_at <= $1
         ORDER BY expires_at, canonical COLLATE \"C\"",
    )
    .bind(now)
    .fetch_all(&mut *db)
    .await?;
    for (canonical, content_hash) in &rows {
        let pos = Position { height, index: -1 };
        resolve_quarantine(db, canonical, height, "expired").await?;
        set_status(db, canonical, "revoked", Some(EXPIRY_REASON)).await?;
        StatusChange {
            status: "revoked",
            source: "quarantine_expiry",
            reason: Some(EXPIRY_REASON),
            ..StatusChange::default()
        }
        .insert(db, pos, canonical, content_hash)
        .await?;
        Revocation {
            source: "quarantine_expiry",
            authority: "ttl_expiry",
            reason: Some(EXPIRY_REASON),
            ..Revocation::default()
        }
        .insert(db, pos, canonical)
        .await?;
    }
    Ok(rows)
}

async fn project_publication(
    db: &mut PgConnection,
    pos: Position,
    record: &ChainRecord,
    block_hash: &str,
    source: &str,
    certificate_digest: Option<String>,
) -> Result<()> {
    let canonical = record.id.canonical();
    let (status, reason, expires) = match &record.status {
        PackageStatus::Verified => ("verified", None, None),
        PackageStatus::Pending => ("pending", None, None),
        PackageStatus::Quarantined {
            reason,
            expires_at_unix,
        } => (
            "quarantined",
            Some(reason.as_str()),
            DateTime::from_timestamp(*expires_at_unix, 0),
        ),
        PackageStatus::Revoked { reason } => ("revoked", Some(reason.as_str()), None),
    };
    sqlx::query(
        "INSERT INTO packages (canonical, ecosystem, name, version, status, content_hash,
            ipfs_cid, publisher_pubkey, block_hash, published_at, shielded, findings,
            access_count, last_accessed, package_namespace, revocation_reason)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)
         ON CONFLICT (canonical) DO UPDATE SET status = EXCLUDED.status, content_hash = EXCLUDED.content_hash,
            ipfs_cid = EXCLUDED.ipfs_cid, publisher_pubkey = EXCLUDED.publisher_pubkey,
            block_hash = EXCLUDED.block_hash, findings = EXCLUDED.findings, shielded = EXCLUDED.shielded,
            package_namespace = EXCLUDED.package_namespace, revocation_reason = EXCLUDED.revocation_reason,
            updated_at = NOW()",
    )
    .bind(&canonical).bind(&record.id.ecosystem).bind(&record.id.name).bind(&record.id.version)
    .bind(status).bind(&record.content_hash).bind(&record.ipfs_cid).bind(&record.publisher_pubkey)
    .bind(block_hash).bind(record.published_at).bind(record.shielded)
    .bind(serde_json::to_value(&record.findings)?).bind(i32::try_from(record.access_count).unwrap_or(i32::MAX))
    .bind(record.last_accessed).bind(common::package_namespace_prefix(&record.id)).bind(reason)
    .execute(&mut *db).await?;
    StatusChange {
        status,
        source,
        reason,
        expires,
        committee: Some(serde_json::to_value(CommitteeSummaryV1::for_record(
            record,
        ))?),
        code: None,
        certificate_digest,
    }
    .insert(db, pos, &canonical, &record.content_hash)
    .await?;
    project_votes(db, pos, record, true).await?;
    Ok(())
}

async fn project_votes(
    db: &mut PgConnection,
    pos: Position,
    record: &ChainRecord,
    is_publication: bool,
) -> Result<()> {
    let canonical = record.id.canonical();
    for signature in &record.validator_signatures {
        let (vote, reason) = match &signature.vote {
            common::ValidatorVote::Approve => ("approve", None),
            common::ValidatorVote::Reject { reason } => ("reject", Some(reason.as_str())),
        };
        if is_publication {
            sqlx::query(
                "INSERT INTO validator_votes (canonical,validator_id,validator_pubkey,signature,vote,reason,signed_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$7)
                 ON CONFLICT (canonical,validator_id) DO UPDATE SET signature = EXCLUDED.signature,
                   vote = EXCLUDED.vote, reason = EXCLUDED.reason, signed_at = EXCLUDED.signed_at",
            )
            .bind(&canonical).bind(&signature.validator_id).bind(&signature.validator_pubkey)
            .bind(&signature.signature).bind(vote).bind(reason).bind(signature.signed_at).execute(&mut *db).await?;
        }
        let scheme = if signature.pq_signature.is_some() {
            "hybrid_ed25519_mldsa44_v1"
        } else {
            "ed25519"
        };
        let scanner_digest = signature.pq_signature.as_ref().map(|_| {
            common::scanner_profile_digest(&signature.ml_model_version, &signature.analysis_bundles)
        });
        sqlx::query(
            "INSERT INTO vote_signatures (canonical,height,tx_index,validator_id,voter_key_id,scheme,
                 approved,reject_reason,evidence_digest,scanner_profile_digest,is_mandatory,signed_at,
                 trigger_witness_digest,signature,pq_signature)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
             ON CONFLICT (canonical,height,tx_index,validator_id) DO NOTHING",
        )
        .bind(&canonical).bind(pos.height).bind(pos.index).bind(&signature.validator_id)
        .bind(&signature.voter_key_id).bind(scheme).bind(vote == "approve").bind(reason)
        .bind(&signature.evidence_digest).bind(scanner_digest).bind(signature.is_mandatory).bind(signature.signed_at)
        .bind(&signature.trigger_witness_digest).bind(&signature.signature).bind(&signature.pq_signature)
        .execute(&mut *db).await?;
        if let Some(semantic) = &signature.semantic_vote {
            sqlx::query(
                "INSERT INTO semantic_votes (canonical,height,tx_index,validator_id,disposition,model_family,finding_digests)
                 VALUES ($1,$2,$3,$4,$5,$6,$7)
                 ON CONFLICT (canonical,height,tx_index,validator_id) DO NOTHING",
            )
            .bind(&canonical).bind(pos.height).bind(pos.index).bind(&signature.validator_id)
            .bind(wire_string(&semantic.disposition)?).bind(semantic.model_family.trim().to_ascii_lowercase())
            .bind(&semantic.finding_digests).execute(&mut *db).await?;
        }
    }
    Ok(())
}

#[derive(Default)]
struct Revocation<'a> {
    source: &'a str,
    authority: &'a str,
    reason: Option<&'a str>,
    revoker: Option<&'a str>,
    evidence: Option<&'a str>,
    witness: Option<String>,
    certificate: Option<Value>,
}

impl Revocation<'_> {
    async fn insert(self, db: &mut PgConnection, pos: Position, canonical: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO revocations (canonical,height,tx_index,tx_type,authority,reason,revoker_pubkey,
                 evidence_hash,witness_digest,quorum_certificate) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(canonical).bind(pos.height).bind(pos.index).bind(self.source).bind(self.authority)
        .bind(self.reason).bind(self.revoker).bind(self.evidence).bind(self.witness).bind(self.certificate)
        .execute(db).await?;
        Ok(())
    }
}

async fn project_revocation(
    db: &mut PgConnection,
    pos: Position,
    tx: &Transaction,
    vote_v4_active: bool,
) -> Result<()> {
    let (canonical, reason, revoker, evidence) = tx
        .revocation_target()
        .context("missing revocation target")?;
    let canonical = canonical.trim();
    let package: Option<(String, String)> = sqlx::query_as(
        "SELECT content_hash, publisher_pubkey FROM packages p WHERE canonical = $1
         AND EXISTS (SELECT 1 FROM package_status_history h WHERE h.canonical = p.canonical)",
    )
    .bind(canonical)
    .fetch_optional(&mut *db)
    .await?;
    let mut details = Revocation {
        source: "revoke",
        authority: "legacy_single_validator",
        reason: Some(reason),
        revoker: Some(revoker),
        evidence: Some(evidence),
        ..Revocation::default()
    };
    if let Transaction::RevokeV2 {
        witness,
        quorum_certificate,
        ..
    } = tx
    {
        details.source = "revoke_v2";
        if let Some(certificate) = quorum_certificate {
            details.authority = "committee_quorum";
            details.certificate = Some(serde_json::to_value(certificate)?);
        } else if package
            .as_ref()
            .is_some_and(|(_, owner)| owner.eq_ignore_ascii_case(revoker))
        {
            details.authority = "owner";
        } else if let Some((prefix, _)) = canonical.rsplit_once('@') {
            let namespace = format!("{prefix}@");
            let controller: Option<String> = sqlx::query_scalar(
                "SELECT COALESCE(
                     (SELECT controller_pubkey FROM namespace_controllers WHERE package_namespace = $1),
                     (SELECT p.publisher_pubkey FROM packages p WHERE p.package_namespace = $1
                      AND p.publisher_pubkey <> '' AND EXISTS
                          (SELECT 1 FROM package_status_history h WHERE h.canonical = p.canonical)
                      ORDER BY p.canonical COLLATE \"C\" LIMIT 1))",
            )
            .bind(namespace).fetch_one(&mut *db).await?;
            if controller.is_some_and(|key| key.eq_ignore_ascii_case(revoker)) {
                details.authority = "controller";
            }
        }
        if let Some(witness) = witness.as_ref().filter(|_| vote_v4_active) {
            let digest = witness.canonical_digest();
            sqlx::query(
                "INSERT INTO trigger_witnesses (witness_digest,canonical,content_hash,manifest_sha256,
                     sandbox_policy_bundle_id,trigger_spec,observed_violations,graph_canonical_digest,
                     prover_id,prover_key_id,hybrid,height,tx_index)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
                 ON CONFLICT (witness_digest) DO NOTHING",
            )
            .bind(&digest).bind(canonical).bind(&witness.content_hash).bind(&witness.manifest_sha256)
            .bind(&witness.sandbox_policy_bundle_id).bind(serde_json::to_value(&witness.trigger_spec)?)
            .bind(&witness.observed_violations).bind(&witness.graph_canonical_digest)
            .bind(&witness.witness_prover_id).bind(&witness.prover_key_id).bind(witness.prover_pq_signature.is_some())
            .bind(pos.height).bind(pos.index).execute(&mut *db).await?;
            details.witness = Some(digest);
        }
    }
    if let Some((content_hash, _)) = package {
        resolve_quarantine(db, canonical, pos.height, "rejected").await?;
        set_status(db, canonical, "revoked", Some(reason)).await?;
        StatusChange {
            status: "revoked",
            source: details.source,
            reason: Some(reason),
            ..StatusChange::default()
        }
        .insert(db, pos, canonical, &content_hash)
        .await?;
    }
    details.insert(db, pos, canonical).await
}

async fn validator_event(
    db: &mut PgConnection,
    pos: Position,
    kind: &str,
    validator_id: &str,
    amount: Option<u64>,
    reason: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO validator_events VALUES ($1, $2, $3, $4, CAST($5 AS TEXT)::NUMERIC(78,0), $6)",
    )
    .bind(pos.height)
    .bind(pos.index)
    .bind(kind)
    .bind(validator_id)
    .bind(amount.map(|amount| amount.to_string()))
    .bind(reason)
    .execute(db)
    .await?;
    Ok(())
}

fn wire_string(value: &impl serde::Serialize) -> Result<String> {
    match serde_json::to_value(value)? {
        Value::String(value) => Ok(value),
        _ => anyhow::bail!("expected string-valued wire enum"),
    }
}

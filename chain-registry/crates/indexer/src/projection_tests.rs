use std::sync::{atomic::AtomicBool, Arc};

use anyhow::Result;
use chrono::TimeDelta;
use common::{
    Block, ChainRecord, CommitteeQuorumCertificateV1, CregFindingCategoryV1, PackageId,
    PackageStatus, Transaction,
};
use registry_view::index::{
    CommitteeDiversityV1, IndexEvidenceHistoryV1, IndexPackageStatusV1, QuarantineQueueV1,
};
use sqlx::PgPool;

use crate::{api::ApiState, apply::apply_block, test_db::isolate, MIGRATOR};

fn next(parent: &Block, transactions: Vec<Transaction>) -> Block {
    let mut header = parent.header.clone();
    header.height += 1;
    header.prev_hash = parent.hash();
    header.timestamp += TimeDelta::seconds(1);
    header.merkle_root = common::merkle_root(&transactions);
    header.validator_set_hash = "aa".repeat(32);
    Block {
        header,
        transactions,
        pbft_signatures: Vec::new(),
    }
}

fn record(name: &str) -> ChainRecord {
    ChainRecord {
        id: PackageId::new("pypi", name, "1.0.0"),
        publisher_pubkey: "old-key".into(),
        content_hash: "11".repeat(32),
        status: PackageStatus::Verified,
        ..ChainRecord::default()
    }
}

#[tokio::test]
#[ignore = "requires a PostgreSQL test database"]
async fn reapply_conflict_continuity_and_projection_are_atomic() -> Result<()> {
    isolate(|pool| async move {
        MIGRATOR.run(&pool).await?;
        MIGRATOR.run(&pool).await?;
        let genesis = Block::genesis();
        assert!(apply_block(&pool, &genesis).await?);
        let held = record("a_b");
        let canonical = held.id.canonical();
        let certificate = CommitteeQuorumCertificateV1::new(
            common::QUARANTINE_QC_DOMAIN,
            &canonical,
            &held.content_hash,
            &"22".repeat(32),
            &common::AnalysisCommittee::default(),
            1,
        );
        let b1 = next(
            &genesis,
            vec![
                Transaction::Publish(held.clone()),
                Transaction::Publish(record("a-b")),
                Transaction::QuarantineV1 {
                    record: held,
                    reason_code: CregFindingCategoryV1::Backdoor,
                    quorum_certificate: certificate,
                },
            ],
        );
        assert!(apply_block(&pool, &b1).await?);
        assert!(!apply_block(&pool, &b1).await?);
        let history_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM package_status_history")
            .fetch_one(&pool)
            .await?;
        assert_eq!(history_rows, 3);

        let mut conflict = b1.clone();
        conflict.header.proposer_id = "conflict".into();
        assert!(apply_block(&pool, &conflict)
            .await
            .unwrap_err()
            .to_string()
            .contains("indexed hash conflict"));
        let mut broken = next(&b1, vec![Transaction::Publish(record("wrong"))]);
        broken.header.prev_hash = "33".repeat(32);
        assert!(apply_block(&pool, &broken)
            .await
            .unwrap_err()
            .to_string()
            .contains("prev_hash continuity break"));
        assert_eq!(cursor(&pool).await?, 1);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let api = ApiState::new(
            pool.clone(),
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicBool::new(true)),
        )
        .router();
        let server = tokio::spawn(async move { axum::serve(listener, api).await });
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()?;

        let queue: QuarantineQueueV1 = client
            .get(format!("{base}/v1/index/quarantine-queue"))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert_eq!(queue.entries.len(), 1);
        assert_eq!(queue.entries[0].canonical.as_str(), canonical);
        let patch = next(&b1, Vec::new());
        let mut b2 = next(
            &b1,
            vec![
                Transaction::RotatePublisherKey {
                    canonical_prefix: "pypi:a_b@".into(),
                    old_pubkey: "old-key".into(),
                    new_pubkey: "new-key".into(),
                    sig_from_old: "sig".into(),
                    sig_from_new: "sig".into(),
                    timestamp: patch.header.timestamp,
                    nonce: 1,
                },
                Transaction::ValidatorJoin {
                    validator_id: "v1".into(),
                    pubkey: "key".into(),
                    stake: u64::MAX,
                },
                Transaction::Slash {
                    validator_id: "v1".into(),
                    amount: 17,
                    reason: "fixture".into(),
                },
                Transaction::ValidatorLeave {
                    validator_id: "v1".into(),
                },
            ],
        );
        b2.header.timestamp =
            b1.header.timestamp + TimeDelta::seconds(common::CLASS_C_QUARANTINE_TTL_SECS);
        // Inject a late constraint failure after expiry and the key rotation
        // were staged. The cursor, package and evidence must all roll back.
        sqlx::query(
            "ALTER TABLE validator_events ADD CONSTRAINT reject_join CHECK (kind <> 'join')",
        )
        .execute(&pool)
        .await?;
        assert!(apply_block(&pool, &b2).await.is_err());
        assert_eq!(cursor(&pool).await?, 1);
        assert_eq!(publisher(&pool, &canonical).await?, "old-key");
        let open: i64 = sqlx::query_scalar("SELECT count(*) FROM v_quarantine_queue")
            .fetch_one(&pool)
            .await?;
        assert_eq!(open, 1);
        sqlx::query("ALTER TABLE validator_events DROP CONSTRAINT reject_join")
            .execute(&pool)
            .await?;

        let (first, second) = tokio::try_join!(apply_block(&pool, &b2), apply_block(&pool, &b2))?;
        assert_ne!(first, second);
        assert_eq!(cursor(&pool).await?, 2);
        assert_eq!(publisher(&pool, &canonical).await?, "new-key");
        assert_eq!(publisher(&pool, "pypi:a-b@1.0.0").await?, "old-key");
        let events: Vec<(String, Option<String>)> =
            sqlx::query_as("SELECT kind,amount_wei::TEXT FROM validator_events ORDER BY tx_index")
                .fetch_all(&pool)
                .await?;
        assert_eq!(
            events,
            vec![
                ("join".into(), Some(u64::MAX.to_string())),
                ("slash".into(), Some("17".into())),
                ("leave".into(), None)
            ]
        );

        let evidence: IndexEvidenceHistoryV1 = client
            .get(format!(
                "{base}/v1/index/packages/{canonical}/evidence-history"
            ))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert_eq!(evidence.entries.len(), 3);
        assert!(matches!(
            evidence.entries[0].status,
            Some(IndexPackageStatusV1::Revoked)
        ));
        assert_eq!(evidence.entries[0].tx_index, -1);
        let diversity: CommitteeDiversityV1 = client
            .get(format!("{base}/v1/index/committee-diversity?epoch=0"))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert_eq!(diversity.observations, 2);

        let mut response = client
            .get(format!("{base}/v1/index/events?after=1.3"))
            .send()
            .await?
            .error_for_status()?;
        assert_eq!(response.headers()["content-type"], "text/event-stream");
        let mut frames = Vec::new();
        while !frames
            .windows(b"id: 2.5".len())
            .any(|chunk| chunk == b"id: 2.5")
        {
            frames.extend(
                response
                    .chunk()
                    .await?
                    .expect("live stream should stay open"),
            );
        }
        let frames = String::from_utf8(frames)?;
        assert!(frames.contains("id: 2.4"));
        assert!(!frames.contains("id: 1.3"));
        server.abort();
        Ok(())
    })
    .await
}

#[tokio::test]
#[ignore = "requires a PostgreSQL test database"]
async fn migrates_v1_and_replays_existing_hashes() -> Result<()> {
    isolate(|pool| async move {
        sqlx::raw_sql(include_str!(
            "../../../migrations/001_db_sync_bootstrap.sql"
        ))
        .execute(&pool)
        .await?;
        let genesis = Block::genesis();
        let block = next(&genesis, vec![Transaction::Publish(record("replay"))]);
        sqlx::query(
            "INSERT INTO blocks (height,hash,prev_hash,merkle_root,proposer_id,timestamp)
                     VALUES (1,$1,$2,$3,$4,$5)",
        )
        .bind(block.hash())
        .bind(&block.header.prev_hash)
        .bind(&block.header.merkle_root)
        .bind(&block.header.proposer_id)
        .bind(block.header.timestamp)
        .execute(&pool)
        .await?;
        sqlx::query("UPDATE sync_state SET last_height = 1 WHERE id = 1")
            .execute(&pool)
            .await?;
        MIGRATOR.run(&pool).await?;
        assert_eq!(cursor(&pool).await?, -1);
        assert!(apply_block(&pool, &genesis).await?);

        let mut conflict = block.clone();
        conflict.header.proposer_id = "wrong chain".into();
        assert!(apply_block(&pool, &conflict).await.is_err());
        assert_eq!(cursor(&pool).await?, 0);
        assert!(apply_block(&pool, &block).await?);
        assert_eq!(cursor(&pool).await?, 1);
        assert_eq!(publisher(&pool, "pypi:replay@1.0.0").await?, "old-key");
        Ok(())
    })
    .await
}

async fn cursor(pool: &PgPool) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT last_height FROM sync_state WHERE id = 1")
            .fetch_one(pool)
            .await?,
    )
}

async fn publisher(pool: &PgPool, canonical: &str) -> Result<String> {
    Ok(
        sqlx::query_scalar("SELECT publisher_pubkey FROM packages WHERE canonical = $1")
            .bind(canonical)
            .fetch_one(pool)
            .await?,
    )
}

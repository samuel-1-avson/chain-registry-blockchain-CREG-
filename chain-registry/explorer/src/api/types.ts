// AUTO-GENERATED — do not edit. Run `npm run gen-types` to refresh.
// Source: C:/Users/samue/AppData/Local/Temp/openapi.json
// Generated: 2026-04-18T14:34:25.223Z

/* eslint-disable */
export interface paths {
    "/v1/addresses/{address}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * Aggregate profile for an EVM address: validator identity (if any) plus
         * @description recent on-chain activity counts.
         */
        get: operations["get_address"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/addresses/{address}/transactions": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Transactions touching an address, scanned from the most recent blocks. */
        get: operations["get_address_transactions"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/blocks": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * Paginated block list. Supports offset/limit and `before_height` / `after_height` cursors.
         * @description - `before_height=H`: return blocks with height < H, newest first.
         *     - `after_height=H`: return blocks with height > H, newest first.
         *     - Omit cursors and use `offset`/`limit` for classic pagination.
         */
        get: operations["list_blocks"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/blocks/{height}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Fetch a block by height. */
        get: operations["get_block_by_height"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/blocks/hash/{hash}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Fetch a block by its 0x-prefixed hash. */
        get: operations["get_block_by_hash"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/bridge/status": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Bridge health + last L1 anchor commit. */
        get: operations["bridge_status"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/chain/stats": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Chain stats — tip height, finality, validator and peer counts. */
        get: operations["chain_stats"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/consensus/state": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Live PBFT round state — in-flight proposals with quorum progress. */
        get: operations["consensus_state"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/health": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Health probe. Returns 200 with build info when the node is live. */
        get: operations["health"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/nodes": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Validator set as understood by this node (includes "self" marker). */
        get: operations["get_nodes"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/packages": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Paginated package list. */
        get: operations["list_packages"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/packages/{canonical}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Fetch a package by canonical id (`<ecosystem>:<name>@<version>`). */
        get: operations["get_package"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/pending": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Pending pool contents — not yet finalized txs. */
        get: operations["list_pending"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/runtime/config": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Runtime configuration — testnet flag, contract addresses, validator flow mode. */
        get: operations["runtime_config"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/transactions/{canonical}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Fetch a transaction by its canonical id (`name@version`) or tx hash. */
        get: operations["get_transaction"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/validators/{address}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Validator profile — registration status, active-set info, and recent block proposals. */
        get: operations["get_validator_profile"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/validators/registrations": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Validator identity registrations known to this node. */
        get: operations["list_validator_registrations"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
}
export type webhooks = Record<string, never>;
export interface components {
    schemas: {
        AddressProfile: {
            /** @description Status from the active validator set when applicable ("online" | "self" | "offline"). */
            active_status?: string | null;
            address: string;
            /**
             * Format: int32
             * @description Blocks proposed by this address within `scanned_blocks`.
             */
            blocks_proposed: number;
            is_active_validator: boolean;
            is_validator: boolean;
            /** Format: int32 */
            reputation?: number | null;
            /**
             * Format: int64
             * @description How many most-recent blocks were inspected to build this profile.
             */
            scanned_blocks: number;
            stake?: string | null;
            /**
             * Format: int32
             * @description Txs referencing this address within `scanned_blocks`.
             */
            tx_count: number;
            validator?: components["schemas"]["ValidatorRegistration"] | null;
        };
        AddressTxList: {
            address: string;
            /** Format: int64 */
            scanned_blocks: number;
            total: number;
            transactions: components["schemas"]["AddressTxRef"][];
        };
        AddressTxRef: {
            block_hash: string;
            /** Format: int64 */
            block_height: number;
            canonical?: string | null;
            /** @description One of: "publish" | "revoke" | "slash" | "validator-join" | "validator-leave" | "rotate-key" | "propose". */
            kind: string;
            timestamp: string;
            tx_index: number;
        };
        ApiError: {
            /** @description Human-readable error message. Intended for display. */
            error: string;
        };
        BlockDetail: {
            finalized: boolean;
            hash: string;
            /** Format: int64 */
            height: number;
            prev_hash: string;
            producer: string;
            /** @description Quorum threshold this block was tested against. */
            quorum?: number | null;
            signature?: string | null;
            /** Format: int64 */
            timestamp_ms: number;
            transactions: components["schemas"]["TransactionSummary"][];
            /** @description Vote-producing validator addresses that approved this block. */
            votes: string[];
        };
        BlockSummary: {
            finalized: boolean;
            hash: string;
            /** Format: int64 */
            height: number;
            prev_hash: string;
            producer: string;
            /** Format: int64 */
            timestamp_ms: number;
            tx_count: number;
        };
        BridgeStatus: {
            bridge_contract?: string | null;
            bridge_sync_status: string;
            /** Format: int64 */
            l1_chain_id?: number | null;
            /** Format: int64 */
            last_anchor_block?: number | null;
            last_anchor_root?: string | null;
            /** Format: int64 */
            last_finalized_eth_block: number;
            signer_address?: string | null;
        };
        ChainStats: {
            /** @description Bridge sync state. "Synced" | "Syncing" | "Unknown" | error strings. */
            bridge_status: string;
            /**
             * Format: int64
             * @description Current tip height.
             */
            current_height: number;
            /** @description Finalized tip hash, hex. */
            finalized_hash?: string | null;
            /**
             * Format: int64
             * @description Highest finalized height. Trails tip by the finality window.
             */
            finalized_height: number;
            genesis_hash?: string | null;
            /**
             * Format: int64
             * @description Last L1 block that the bridge read.
             */
            l1_block: number;
            package_count?: number | null;
            /** @description Connected P2P peer count. */
            peer_count: number;
            pending_tx_count?: number | null;
            publisher_count?: number | null;
            /**
             * Format: int64
             * @description Sum of validator stakes (native units).
             */
            total_stake: number;
            /** @description Active validator count. */
            validator_count: number;
        };
        ConsensusRound: {
            approvals: number;
            block_hash: string;
            /** Format: int64 */
            height: number;
            phase: string;
            proposer: string;
            quorum: number;
            rejections: number;
        };
        ConsensusState: {
            active_rounds: components["schemas"]["ConsensusRound"][];
        };
        Health: {
            /**
             * @description Always "ok" when the node is responsive.
             * @example ok
             */
            status: string;
            /**
             * @description Semver of the node binary (from Cargo.toml).
             * @example 0.1.0
             */
            version: string;
        };
        NodeEntry: {
            address?: string | null;
            id: string;
            role?: string | null;
            status?: string | null;
        };
        PackageDetail: {
            block_hash?: string | null;
            canonical: string;
            content_hash?: string | null;
            ipfs_cid?: string | null;
            published_at?: string | null;
            publisher?: string | null;
            revocation_reason?: string | null;
            status: string;
        };
        PackageList: {
            limit: number;
            offset: number;
            packages: components["schemas"]["PackageSummary"][];
            total: number;
        };
        PackageSummary: {
            canonical: string;
            ecosystem: string;
            name: string;
            published_at: string;
            publisher: string;
            status: string;
            version: string;
        };
        Pending: {
            canonical: string;
            publisher: string;
            received_at: string;
            stage?: string | null;
        };
        PendingList: {
            pending: components["schemas"]["Pending"][];
            total: number;
        };
        RuntimeConfig: {
            is_testnet: boolean;
            /** @description Registry.sol contract address on the configured L1, if set. */
            registry_address?: string | null;
            staking_contract?: string | null;
            token_contract?: string | null;
            /** @description Machine-readable registration flow id (e.g. "staking-plus-identity-sync"). */
            validator_registration_mode: string;
            /** @description Operator-facing note describing the validator onboarding flow. */
            validator_registration_note: string;
        };
        TransactionDetail: {
            /** Format: int64 */
            block_height?: number | null;
            canonical: string;
            included_at?: string | null;
            ipfs_cid?: string | null;
            payload_hash?: string | null;
            publisher: string;
            status: string;
            /**
             * @description Arbitrary validation metadata. Shape stabilises once the pipeline
             *     stages are finalised — for now the explorer just surfaces key/value rows.
             */
            validation: Record<string, never>;
            version: string;
        };
        TransactionSummary: {
            /** Format: int64 */
            block_height?: number | null;
            canonical: string;
            publisher: string;
            status: string;
        };
        ValidatorIdentityInfo: {
            ed25519_pubkey: string;
            evm_address: string;
            node_id: string;
        };
        ValidatorProfile: {
            address: string;
            in_active_set: boolean;
            /** @description Blocks proposed by this validator within the recent window (newest first). */
            recent_proposals: components["schemas"]["BlockSummary"][];
            registration?: components["schemas"]["ValidatorRegistration"] | null;
            /** Format: int32 */
            reputation: number;
            stake: string;
            status: string;
        };
        ValidatorRegistration: {
            alias: string;
            identity: components["schemas"]["ValidatorIdentityInfo"];
            registered_with_node: boolean;
            /** Format: int64 */
            reputation: number;
            stake?: string | null;
            status: string;
        };
    };
    responses: never;
    parameters: never;
    requestBodies: never;
    headers: never;
    pathItems: never;
}
export type $defs = Record<string, never>;
export interface operations {
    get_address: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description EVM address (0x…) */
                address: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AddressProfile"];
                };
            };
            400: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiError"];
                };
            };
        };
    };
    get_address_transactions: {
        parameters: {
            query?: {
                /** @description Max txs to return (default 50, max 500) */
                limit?: number | null;
                /** @description Max blocks to scan backwards (default 500, max 5000) */
                scan?: number | null;
            };
            header?: never;
            path: {
                /** @description EVM address */
                address: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AddressTxList"];
                };
            };
            400: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiError"];
                };
            };
        };
    };
    list_blocks: {
        parameters: {
            query?: {
                /** @description Return blocks above this height */
                after_height?: number | null;
                /** @description Return blocks below this height */
                before_height?: number | null;
                /** @description Max blocks to return (default 20, max 100) */
                limit?: number | null;
                /** @description Offset into the list (ignored when cursors are set) */
                offset?: number | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BlockSummary"][];
                };
            };
        };
    };
    get_block_by_height: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description Block height */
                height: number;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BlockDetail"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiError"];
                };
            };
        };
    };
    get_block_by_hash: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description 0x-prefixed block hash */
                hash: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BlockDetail"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiError"];
                };
            };
        };
    };
    bridge_status: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BridgeStatus"];
                };
            };
        };
    };
    chain_stats: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ChainStats"];
                };
            };
        };
    };
    consensus_state: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ConsensusState"];
                };
            };
        };
    };
    health: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Node is alive */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Health"];
                };
            };
        };
    };
    get_nodes: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["NodeEntry"][];
                };
            };
        };
    };
    list_packages: {
        parameters: {
            query?: {
                /** @description Filter by ecosystem (npm, pypi, …) */
                ecosystem?: string | null;
                /** @description Max packages to return (default 50, max 200) */
                limit?: number | null;
                /** @description Offset into the result set (default 0) */
                offset?: number | null;
                /** @description Filter by status (verified | pending | revoked) */
                status?: string | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PackageList"];
                };
            };
        };
    };
    get_package: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description Package canonical id (`<ecosystem>:<name>@<version>`) */
                canonical: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PackageDetail"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiError"];
                };
            };
        };
    };
    list_pending: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PendingList"];
                };
            };
        };
    };
    runtime_config: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RuntimeConfig"];
                };
            };
        };
    };
    get_transaction: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description Package canonical or tx hash */
                canonical: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TransactionDetail"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiError"];
                };
            };
        };
    };
    get_validator_profile: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description Validator EVM address */
                address: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ValidatorProfile"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiError"];
                };
            };
        };
    };
    list_validator_registrations: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ValidatorRegistration"][];
                };
            };
        };
    };
}

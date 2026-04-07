// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "./Reputation.sol";
import "./CregToken.sol";

/// @title Staking
/// @notice Manages publisher and validator stakes using CREG tokens.
/// @dev Publishers stake CREG to publish packages — bad actors lose stake.
///      Validators must apply and be approved by governance before joining.
///      Slashed CREG is distributed to honest active validators, not burned.
///      Validators must wait UNBONDING_PERIOD before withdrawing stake.
contract Staking {
    // ── Reentrancy Guard ─────────────────────────────────────────────────────
    bool private _locked;
    modifier nonReentrant() {
        require(!_locked, "Reentrant call");
        _locked = true;
        _;
        _locked = false;
    }

    // ── Enums ────────────────────────────────────────────────────────────────

    /// Full lifecycle of a validator.
    enum ValidatorState { None, Pending, Active, Unbonding, Withdrawn, Rejected }

    /// Slashing severity levels.
    enum Severity { Low, Medium, Critical }

    // ── Constants ─────────────────────────────────────────────────────────────

    /// Minimum CREG stake to publish a package (1 CREG).
    uint256 public minPublisherStake = 1 * 10**18;
    /// Minimum CREG stake to apply as a validator (100 CREG).
    uint256 public minValidatorStake = 100 * 10**18;
    /// Minimum for light validators (50 CREG).
    uint256 public minLightValidatorStake = 50 * 10**18;
    /// Unbonding period — validators must wait 14 days before withdrawing stake.
    /// This prevents hit-and-run attacks where a validator misbehaves then immediately exits.
    uint256 public constant UNBONDING_PERIOD = 14 days;
    /// Cooldown before a slashed/ejected validator can re-stake.
    uint256 public constant RESTAKE_COOLDOWN = 7 days;
    /// Maximum slashes before auto-ejection.
    uint256 public constant MAX_SLASH_COUNT = 3;
    /// Slash percentage for Low severity (basis: 100).
    uint256 public constant SLASH_LOW_PCT = 2;
    /// Slash percentage for Medium severity.
    uint256 public constant SLASH_MEDIUM_PCT = 10;
    /// Slash percentage for Critical severity.
    uint256 public constant SLASH_CRITICAL_PCT = 30;

    // ── Storage ───────────────────────────────────────────────────────────────

    struct ValidatorEntry {
        uint256        stake;
        ValidatorState state;
        uint256        unbondingAt;  // Timestamp when unbonding was initiated
        uint256        slashCount;
        uint256        ejectedAt;    // Timestamp of slash-eject (for restake cooldown)
    }

    /// The CREG token contract — all staking uses this token.
    CregToken public cregToken;

    mapping(address => uint256)        public publisherStakes;
    mapping(address => ValidatorEntry) public validators;

    Reputation public reputation;
    address    public registry;    // Only Registry can trigger slashing
    address    public governance;
    uint256    public slashPool;   // Accumulated slashed CREG (distributed to honest validators)

    address[] private _validatorList;

    // ── Events ────────────────────────────────────────────────────────────────

    event PublisherStaked        (address indexed publisher, uint256 amount);
    event PublisherUnstaked      (address indexed publisher, uint256 amount);
    event ValidatorApplied       (address indexed validator, uint256 stake);
    event ValidatorApproved      (address indexed validator);
    event ValidatorRejected      (address indexed validator);
    event ValidatorUnbonding     (address indexed validator, uint256 unbondingAt);
    event ValidatorWithdrawn     (address indexed validator, uint256 amount);
    event ValidatorLeft          (address indexed validator);
    event Slashed                (address indexed account, uint256 amount, string reason);
    event SlashPoolDistributed   (uint256 amount, uint256 validatorCount);

    // ── Errors ────────────────────────────────────────────────────────────────

    error BelowMinStake      (uint256 provided, uint256 minimum);
    error AlreadyApplied     ();
    error NotPending         ();
    error NotValidator       ();
    error NotActive          ();
    error NotUnbonding       ();
    error StillUnbonding     (uint256 availableAt);
    error NotAuthorized      ();
    error InsufficientStake  ();
    error TransferFailed     ();
    error RestakeCooldownActive(uint256 availableAt);

    // ── Constructor ───────────────────────────────────────────────────────────

    /// @param _governance Address that can approve/reject validators and distribute slash pool.
    /// @param _cregToken  Address of the deployed CregToken contract.
    constructor(address _governance, address _cregToken) {
        governance = _governance;
        cregToken  = CregToken(_cregToken);
    }

    // ── Initializer ───────────────────────────────────────────────────────────

    function setContracts(address _registry, address _reputation) external {
        require(registry == address(0), "Already set");
        registry   = _registry;
        reputation = Reputation(_reputation);
    }

    // ── Publisher staking ─────────────────────────────────────────────────────

    /// @notice Stake CREG as a publisher. Must approve this contract first.
    /// @param amount Amount of CREG (in token units, 18 decimals) to stake.
    function stakeAsPublisher(uint256 amount) external {
        if (amount < minPublisherStake)
            revert BelowMinStake(amount, minPublisherStake);
        if (!cregToken.transferFrom(msg.sender, address(this), amount))
            revert TransferFailed();
        publisherStakes[msg.sender] += amount;
        emit PublisherStaked(msg.sender, amount);
    }

    /// @notice Withdraw publisher stake. Only allowed if no active packages depend on it.
    /// @param amount Amount of CREG to withdraw.
    function unstakeAsPublisher(uint256 amount) external nonReentrant {
        if (publisherStakes[msg.sender] < amount) revert InsufficientStake();
        publisherStakes[msg.sender] -= amount;
        if (!cregToken.transfer(msg.sender, amount))
            revert TransferFailed();
        emit PublisherUnstaked(msg.sender, amount);
    }

    function stakedBalance(address publisher) external view returns (uint256) {
        return publisherStakes[publisher];
    }

    // ── Validator staking — two-step (apply → approve) ────────────────────────

    /// @notice Step 1: Apply to become a validator by staking CREG.
    ///         Your stake is held in escrow until governance approves or rejects you.
    ///         If rejected, your CREG is returned in full.
    ///         Ejected validators must wait RESTAKE_COOLDOWN before re-applying.
    /// @param amount Amount of CREG to stake (must be >= minValidatorStake).
    function applyToBeValidator(uint256 amount) external {
        ValidatorEntry storage v = validators[msg.sender];
        if (v.state == ValidatorState.Pending || v.state == ValidatorState.Active)
            revert AlreadyApplied();

        // Enforce cooldown for re-staking after slash ejection
        if (v.ejectedAt > 0 && block.timestamp < v.ejectedAt + RESTAKE_COOLDOWN)
            revert RestakeCooldownActive(v.ejectedAt + RESTAKE_COOLDOWN);

        if (amount < minValidatorStake)
            revert BelowMinStake(amount, minValidatorStake);

        if (!cregToken.transferFrom(msg.sender, address(this), amount))
            revert TransferFailed();

        validators[msg.sender] = ValidatorEntry({
            stake:       amount,
            state:       ValidatorState.Pending,
            unbondingAt: 0,
            slashCount:  0,
            ejectedAt:   0
        });
        _validatorList.push(msg.sender);
        emit ValidatorApplied(msg.sender, amount);
    }

    /// @notice Step 2a: Governance approves a pending validator application.
    ///         The validator becomes Active and can now vote in consensus.
    function approveValidator(address validator) external {
        if (msg.sender != governance) revert NotAuthorized();
        ValidatorEntry storage v = validators[validator];
        if (v.state != ValidatorState.Pending) revert NotPending();
        v.state = ValidatorState.Active;
        emit ValidatorApproved(validator);
    }

    /// @notice Step 2b: Governance rejects a pending validator application.
    ///         The applicant's full CREG stake is returned immediately.
    function rejectValidator(address validator) external nonReentrant {
        if (msg.sender != governance) revert NotAuthorized();
        ValidatorEntry storage v = validators[validator];
        if (v.state != ValidatorState.Pending) revert NotPending();
        uint256 amount = v.stake;
        v.stake = 0;
        v.state = ValidatorState.Rejected;
        if (!cregToken.transfer(validator, amount))
            revert TransferFailed();
        emit ValidatorRejected(validator);
    }

    /// @notice Initiate unbonding. Stake is locked for UNBONDING_PERIOD before withdrawal.
    /// @dev Changes validator state to Unbonding. They can no longer participate in consensus
    ///      but their stake remains locked and can still be slashed during the unbonding period.
    function initiateUnbonding() external {
        ValidatorEntry storage v = validators[msg.sender];
        if (v.state != ValidatorState.Active) revert NotActive();

        v.state = ValidatorState.Unbonding;
        v.unbondingAt = block.timestamp;

        emit ValidatorUnbonding(msg.sender, block.timestamp);
    }

    /// @notice Withdraw validator stake after the unbonding period has elapsed.
    /// @dev Can only be called when in Unbonding state and UNBONDING_PERIOD has passed.
    function withdrawValidatorStake() external nonReentrant {
        ValidatorEntry storage v = validators[msg.sender];
        if (v.state != ValidatorState.Unbonding) revert NotUnbonding();

        uint256 availableAt = v.unbondingAt + UNBONDING_PERIOD;
        if (block.timestamp < availableAt)
            revert StillUnbonding(availableAt);

        uint256 amount = v.stake;
        v.stake = 0;
        v.state = ValidatorState.Withdrawn;

        if (!cregToken.transfer(msg.sender, amount))
            revert TransferFailed();

        emit ValidatorWithdrawn(msg.sender, amount);
        emit ValidatorLeft(msg.sender);
    }

    function isActiveValidator(address addr) external view returns (bool) {
        return validators[addr].state == ValidatorState.Active;
    }

    function activeValidatorCount() external view returns (uint256) {
        uint256 count;
        for (uint i = 0; i < _validatorList.length; i++) {
            if (validators[_validatorList[i]].state == ValidatorState.Active) count++;
        }
        return count;
    }

    /// @notice Check whether a validator is currently in the unbonding period.
    function isUnbonding(address addr) external view returns (bool, uint256) {
        ValidatorEntry storage v = validators[addr];
        if (v.state != ValidatorState.Unbonding) return (false, 0);
        return (true, v.unbondingAt + UNBONDING_PERIOD);
    }

    // ── Slashing ─────────────────────────────────────────────────────────────

    /// @notice Slash an account by severity. Slashed CREG goes to the slash pool.
    /// @dev Only callable by Registry (on revocation) or Governance (on misbehaviour).
    ///      Validators can be slashed even during the unbonding period.
    function slashSeverity(address account, Severity severity, string calldata reason)
        external
        nonReentrant
    {
        if (msg.sender != registry && msg.sender != governance)
            revert NotAuthorized();

        uint256 balance = publisherStakes[account] > 0
            ? publisherStakes[account]
            : validators[account].stake;

        uint256 amount;
        if      (severity == Severity.Low)      amount = balance * SLASH_LOW_PCT      / 100;
        else if (severity == Severity.Medium)   amount = balance * SLASH_MEDIUM_PCT   / 100;
        else                                    amount = balance * SLASH_CRITICAL_PCT  / 100;

        _executeSlash(account, amount, reason);
    }

    /// @notice Slash an exact CREG amount from an account.
    function slash(address account, uint256 amount, string calldata reason)
        external
        nonReentrant
    {
        if (msg.sender != registry && msg.sender != governance)
            revert NotAuthorized();
        _executeSlash(account, amount, reason);
    }

    function _executeSlash(address account, uint256 amount, string calldata reason) internal {
        if (publisherStakes[account] >= amount) {
            publisherStakes[account] -= amount;
        } else if (validators[account].stake >= amount) {
            validators[account].stake      -= amount;
            validators[account].slashCount += 1;
            // Auto-eject validator after MAX_SLASH_COUNT slashes.
            // Remaining stake enters unbonding; ejected validator must wait
            // RESTAKE_COOLDOWN before re-applying.
            if (validators[account].slashCount >= MAX_SLASH_COUNT) {
                validators[account].state     = ValidatorState.Unbonding;
                validators[account].unbondingAt = block.timestamp;
                validators[account].ejectedAt  = block.timestamp;
                emit ValidatorLeft(account);
            }
        } else {
            // Slash everything they have left.
            amount = publisherStakes[account] + validators[account].stake;
            publisherStakes[account]          = 0;
            validators[account].stake         = 0;
            validators[account].state         = ValidatorState.Withdrawn;
            validators[account].ejectedAt     = block.timestamp;
        }

        // Slashed CREG is not burned — it goes into the pool for honest validators.
        slashPool += amount;
        emit Slashed(account, amount, reason);
    }

    // ── Slash Pool Distribution ───────────────────────────────────────────────

    /// @notice Distribute all accumulated slashed CREG to active validators.
    ///         Each validator receives a share proportional to their reputation score.
    ///         Their distributed share is added directly to their staked balance —
    ///         they do not need to manually claim it.
    /// @dev Called periodically by governance to reward honest validators.
    function distributeSlashPool() external nonReentrant {
        if (msg.sender != governance) revert NotAuthorized();

        uint256 totalWeight;
        uint256 activeCount;

        for (uint i = 0; i < _validatorList.length; i++) {
            address val = _validatorList[i];
            if (validators[val].state == ValidatorState.Active) {
                totalWeight += uint256(reputation.scoreOf(val));
                activeCount++;
            }
        }

        require(activeCount > 0, "No active validators");
        require(totalWeight > 0, "No reputation weight");

        uint256 poolToDistribute = slashPool;
        slashPool = 0;

        for (uint i = 0; i < _validatorList.length; i++) {
            address val = _validatorList[i];
            if (validators[val].state == ValidatorState.Active) {
                uint256 score = uint256(reputation.scoreOf(val));
                uint256 share = (poolToDistribute * score) / totalWeight;
                // Share is added to their staked balance — compounds their position.
                validators[val].stake += share;
            }
        }

        emit SlashPoolDistributed(poolToDistribute, activeCount);
    }

    // ── Governance ────────────────────────────────────────────────────────────

    /// @notice Update the minimum CREG stakes required to publish or validate.
    function updateMinStakes(uint256 _pubStake, uint256 _valStake) external {
        if (msg.sender != governance) revert NotAuthorized();
        minPublisherStake = _pubStake;
        minValidatorStake = _valStake;
    }

    /// @notice Transfer governance to a new address (e.g. multisig on mainnet).
    function transferGovernance(address newGovernance) external {
        if (msg.sender != governance) revert NotAuthorized();
        governance = newGovernance;
    }
}

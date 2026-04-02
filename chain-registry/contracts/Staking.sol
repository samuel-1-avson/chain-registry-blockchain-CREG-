// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "./Reputation.sol";
import "./CregToken.sol";

/// @title Staking
/// @notice Manages publisher and validator stakes using CREG tokens.
/// @dev Publishers stake CREG to publish packages — bad actors lose stake.
///      Validators must apply and be approved by governance before joining.
///      Slashed CREG is distributed to honest active validators, not burned.
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
    /// Unbonding period before a validator can withdraw stake after leaving.
    uint256 public constant UNBONDING_PERIOD = 7 days;

    // ── Storage ───────────────────────────────────────────────────────────────

    struct ValidatorEntry {
        uint256        stake;
        ValidatorState state;
        uint256        unbondingAt;  // 0 unless in Unbonding state
        uint256        slashCount;
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
    event ValidatorLeft          (address indexed validator);
    event Slashed                (address indexed account, uint256 amount, string reason);
    event SlashPoolDistributed   (uint256 amount, uint256 validatorCount);

    // ── Errors ────────────────────────────────────────────────────────────────

    error BelowMinStake      (uint256 provided, uint256 minimum);
    error AlreadyApplied     ();
    error NotPending         ();
    error NotValidator       ();
    error NotActive          ();
    error StillUnbonding     (uint256 unbondingAt);
    error NotAuthorized      ();
    error InsufficientStake  ();

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
        require(cregToken.transferFrom(msg.sender, address(this), amount), "CREG transfer failed");
        publisherStakes[msg.sender] += amount;
        emit PublisherStaked(msg.sender, amount);
    }

    /// @notice Withdraw publisher stake. Only allowed if no active packages depend on it.
    /// @param amount Amount of CREG to withdraw.
    function unstakeAsPublisher(uint256 amount) external nonReentrant {
        if (publisherStakes[msg.sender] < amount) revert InsufficientStake();
        publisherStakes[msg.sender] -= amount;
        require(cregToken.transfer(msg.sender, amount), "CREG transfer failed");
        emit PublisherUnstaked(msg.sender, amount);
    }

    function stakedBalance(address publisher) external view returns (uint256) {
        return publisherStakes[publisher];
    }

    // ── Validator staking — two-step (apply → approve) ────────────────────────

    /// @notice Step 1: Apply to become a validator by staking CREG.
    ///         Your stake is held in escrow until governance approves or rejects you.
    ///         If rejected, your CREG is returned in full.
    /// @param amount Amount of CREG to stake (must be >= minValidatorStake).
    function applyToBeValidator(uint256 amount) external {
        ValidatorEntry storage v = validators[msg.sender];
        if (v.state == ValidatorState.Pending || v.state == ValidatorState.Active)
            revert AlreadyApplied();
        if (amount < minValidatorStake)
            revert BelowMinStake(amount, minValidatorStake);

        require(cregToken.transferFrom(msg.sender, address(this), amount), "CREG transfer failed");

        validators[msg.sender] = ValidatorEntry({
            stake:       amount,
            state:       ValidatorState.Pending,
            unbondingAt: 0,
            slashCount:  0
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
        require(cregToken.transfer(validator, amount), "CREG transfer failed");
        emit ValidatorRejected(validator);
    }

    /// @notice Active validator initiates exit. Stake enters a 7-day unbonding period.
    function initiateUnbonding() external {
        ValidatorEntry storage v = validators[msg.sender];
        if (v.state != ValidatorState.Active) revert NotActive();
        v.state       = ValidatorState.Unbonding;
        v.unbondingAt = block.timestamp + UNBONDING_PERIOD;
        emit ValidatorLeft(msg.sender);
    }

    /// @notice Withdraw staked CREG after the unbonding period has elapsed.
    function withdrawValidatorStake() external nonReentrant {
        ValidatorEntry storage v = validators[msg.sender];
        if (v.state != ValidatorState.Unbonding) revert NotValidator();
        if (block.timestamp < v.unbondingAt)
            revert StillUnbonding(v.unbondingAt);
        uint256 amount = v.stake;
        v.stake       = 0;
        v.state       = ValidatorState.Withdrawn;
        require(cregToken.transfer(msg.sender, amount), "CREG transfer failed");
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

    // ── Slashing ─────────────────────────────────────────────────────────────

    /// @notice Slash an account by severity. Slashed CREG goes to the slash pool.
    /// @dev Only callable by Registry (on revocation) or Governance (on misbehaviour).
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
        if      (severity == Severity.Low)      amount = balance * 2  / 100;  //  2%
        else if (severity == Severity.Medium)   amount = balance * 10 / 100;  // 10%
        else                                    amount = balance * 30 / 100;  // 30%

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
            // Auto-eject validator after 3 slashes — begin unbonding for remaining stake.
            if (validators[account].slashCount >= 3) {
                validators[account].state       = ValidatorState.Unbonding;
                validators[account].unbondingAt = block.timestamp + UNBONDING_PERIOD;
                emit ValidatorLeft(account);
            }
        } else {
            // Slash everything they have left.
            amount = publisherStakes[account] + validators[account].stake;
            publisherStakes[account]          = 0;
            validators[account].stake         = 0;
            validators[account].state         = ValidatorState.Unbonding;
            validators[account].unbondingAt   = block.timestamp + UNBONDING_PERIOD;
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
        require(msg.sender == governance, "Only governance");

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

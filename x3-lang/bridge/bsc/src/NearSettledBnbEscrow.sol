// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import "./INearSettlementVerifier.sol";

/**
 * @title NearSettledBnbEscrow
 * @notice Issue #42 BNB escrow whose ONLY payout condition is a verified,
 *         finalized NEAR terminal outcome of the exact matching funding window.
 * @dev    TESTNET-ONLY. Not audited. NOT deployable for real value: no
 *         implementation of `INearSettlementVerifier` exists yet (issue #56),
 *         and this contract is only as sound as the verifier it is pinned to.
 *
 *         Withdrawal condition (see docs/security/NEAR_BSC_FINALITY_BOUND_SETTLEMENT.md):
 *           BNB -> LP         iff verified final NEAR `Claimed`   for windowId
 *           BNB -> depositor  iff verified final NEAR `Refunded`  for windowId
 *                              or verified final NEAR `NotFunded` for windowId
 *         where windowId commits to every NEAR term AND to this escrow, this
 *         chain, the depositor, the LP, the BNB amount and a salt.
 *
 *         No function accepts a hash preimage, so a secret leaked by a failed,
 *         expired, front-run or never-finalized NEAR claim confers nothing here.
 *         There is no BSC clock in the payout rule: a late `Claimed` proof still
 *         pays the LP and no timeout refunds the depositor without evidence.
 *         If the verifier is permanently unavailable, BNB stays locked (fail
 *         closed); there is deliberately no admin or escape hatch.
 */
contract NearSettledBnbEscrow {
    enum State {
        Absent,
        Locked,
        PaidToLp,
        RefundedToDepositor
    }

    struct Swap {
        address depositor;
        address lp;
        uint256 amount;
        State state;
    }

    /// @notice Depositor-chosen NEAR terms. The NEAR vault is fixed per escrow.
    struct LockTerms {
        address lp;
        bytes32 salt;
        string nearInitiator;
        string nearRecipient;
        bytes32 hashlock;
        uint128 nearAmountYocto;
        uint64 fundingDeadlineNs;
        uint64 nearExpiryNs;
    }

    uint8 public constant OUTCOME_REGISTERED = 1;
    uint8 public constant OUTCOME_CLAIMED = 2;
    uint8 public constant OUTCOME_REFUNDED = 3;
    uint8 public constant OUTCOME_NOT_FUNDED = 4;

    bytes4 public constant NEAR_SETTLEMENT_VERIFIED = INearSettlementVerifier.verifySettlement.selector;

    bytes private constant SOURCE_DOMAIN = "X3-BSC-SOURCE-V1";
    bytes private constant WINDOW_DOMAIN = "X3-NEAR-BSC-WINDOW-V1";
    bytes private constant SETTLEMENT_DOMAIN = "X3-NEAR-SETTLEMENT-V1";

    address public immutable verifier;
    bytes32 public immutable nearGenesisHash;
    bytes32 public immutable nearVaultCodeHash;
    string public nearVault;

    mapping(bytes32 => Swap) public swaps;
    mapping(address => uint256) public credits;
    uint256 public lockedBnb;
    uint256 public owedBnb;
    bool private entered;

    event Locked(bytes32 indexed windowId, address indexed depositor, address indexed lp, uint256 amount, bytes32 salt);
    event Settled(bytes32 indexed windowId, uint8 outcome, address indexed payee, uint256 amount);
    event Withdrawn(address indexed owner, address indexed to, uint256 amount);

    constructor(address verifier_, bytes32 nearGenesisHash_, string memory nearVault_, bytes32 nearVaultCodeHash_) {
        require(block.chainid == 97 || block.chainid == 31337, "testnet only");
        require(verifier_.code.length > 0, "verifier must be a contract");
        require(nearGenesisHash_ != bytes32(0) && nearVaultCodeHash_ != bytes32(0), "zero NEAR identity");
        require(_endsWithTestnet(bytes(nearVault_)) && bytes(nearVault_).length <= 64, "testnet NEAR vault only");
        require(INearSettlementVerifier(verifier_).nearGenesisHash() == nearGenesisHash_, "verifier chain mismatch");
        verifier = verifier_;
        nearGenesisHash = nearGenesisHash_;
        nearVaultCodeHash = nearVaultCodeHash_;
        nearVault = nearVault_;
    }

    modifier nonReentrant() {
        require(!entered, "reentrant");
        entered = true;
        _;
        entered = false;
    }

    /// @notice Lock BNB against a NEAR funding window that is PROVEN to be
    ///         registered, final, with exactly these terms. Without that proof
    ///         a depositor could lock BNB against a window that can never reach
    ///         a terminal NEAR outcome, and the BNB could never be settled.
    function lock(LockTerms calldata t, bytes calldata registrationProof)
        external
        payable
        nonReentrant
        returns (bytes32 windowId)
    {
        require(msg.value > 0, "zero escrow");
        require(t.lp != address(0), "zero lp");
        require(t.hashlock != bytes32(0) && t.nearAmountYocto > 0, "bad NEAR terms");
        require(_accountLengthOk(t.nearInitiator) && _accountLengthOk(t.nearRecipient), "bad NEAR account");
        windowId = expectedWindowId(msg.sender, msg.value, t);
        require(swaps[windowId].state == State.Absent, "window already used");
        _requireVerified(windowId, OUTCOME_REGISTERED, registrationProof);
        swaps[windowId] = Swap({depositor: msg.sender, lp: t.lp, amount: msg.value, state: State.Locked});
        lockedBnb += msg.value;
        emit Locked(windowId, msg.sender, t.lp, msg.value, t.salt);
    }

    /// @notice Permissionless: anyone may relay a verified terminal NEAR outcome.
    function settle(bytes32 windowId, uint8 outcome, bytes calldata proof) external nonReentrant {
        Swap storage s = swaps[windowId];
        require(s.state == State.Locked, "not locked");
        require(
            outcome == OUTCOME_CLAIMED || outcome == OUTCOME_REFUNDED || outcome == OUTCOME_NOT_FUNDED,
            "not a terminal outcome"
        );
        _requireVerified(windowId, outcome, proof);
        address payee;
        if (outcome == OUTCOME_CLAIMED) {
            payee = s.lp;
            s.state = State.PaidToLp;
        } else {
            payee = s.depositor;
            s.state = State.RefundedToDepositor;
        }
        uint256 amount = s.amount;
        lockedBnb -= amount;
        owedBnb += amount;
        credits[payee] += amount;
        emit Settled(windowId, outcome, payee, amount);
    }

    /// @notice Pull payment. A recipient that cannot receive keeps its credit
    ///         and may direct it to another address.
    function withdraw(address payable to) external nonReentrant {
        require(to != address(0), "zero destination");
        uint256 amount = credits[msg.sender];
        require(amount > 0, "no credit");
        credits[msg.sender] = 0;
        owedBnb -= amount;
        (bool ok,) = to.call{value: amount}("");
        require(ok, "withdraw transfer failed");
        emit Withdrawn(msg.sender, to, amount);
    }

    // ---------------------------------------------------------------- encoding

    function expectedWindowId(address depositor, uint256 bnbAmount, LockTerms calldata t)
        public
        view
        returns (bytes32)
    {
        bytes32 binding = sourceBindingFor(block.chainid, address(this), depositor, t.lp, bnbAmount, t.salt);
        return windowIdFor(
            nearVault,
            t.nearInitiator,
            t.nearRecipient,
            t.hashlock,
            t.nearAmountYocto,
            t.fundingDeadlineNs,
            t.nearExpiryNs,
            binding
        );
    }

    function sourceBindingFor(
        uint256 chainId,
        address escrow,
        address depositor,
        address lp,
        uint256 bnbAmount,
        bytes32 salt
    ) public pure returns (bytes32) {
        return sha256(abi.encodePacked(SOURCE_DOMAIN, chainId, escrow, depositor, lp, bnbAmount, salt));
    }

    function windowIdFor(
        string memory vault,
        string memory initiator,
        string memory recipient,
        bytes32 hashlock,
        uint128 nearAmountYocto,
        uint64 fundingDeadlineNs,
        uint64 nearExpiryNs,
        bytes32 sourceBinding
    ) public pure returns (bytes32) {
        bytes memory parties = abi.encodePacked(
            WINDOW_DOMAIN, _lengthPrefixed(vault), _lengthPrefixed(initiator), _lengthPrefixed(recipient)
        );
        return
            sha256(abi.encodePacked(parties, hashlock, nearAmountYocto, fundingDeadlineNs, nearExpiryNs, sourceBinding));
    }

    function settlementCommitment(bytes32 windowId, uint8 outcome) public pure returns (bytes32) {
        return sha256(abi.encodePacked(SETTLEMENT_DOMAIN, windowId, outcome));
    }

    // ---------------------------------------------------------------- internal

    /// @dev Fail closed on revert, missing code, short/long/dirty return data,
    ///      or any value other than the exact magic word. A verifier that
    ///      returns ABI `true` (0x...01) is rejected.
    function _requireVerified(bytes32 windowId, uint8 outcome, bytes calldata proof) internal view {
        bytes32 commitment = settlementCommitment(windowId, outcome);
        (bool ok, bytes memory ret) = verifier.staticcall(
            abi.encodeCall(
                INearSettlementVerifier.verifySettlement,
                (nearGenesisHash, nearVault, nearVaultCodeHash, commitment, proof)
            )
        );
        require(
            ok && ret.length == 32 && bytes32(ret) == bytes32(NEAR_SETTLEMENT_VERIFIED), "NEAR outcome not verified"
        );
    }

    function _lengthPrefixed(string memory s) private pure returns (bytes memory) {
        return abi.encodePacked(uint32(bytes(s).length), bytes(s));
    }

    function _accountLengthOk(string calldata s) private pure returns (bool) {
        uint256 n = bytes(s).length;
        return n >= 2 && n <= 64;
    }

    function _endsWithTestnet(bytes memory s) private pure returns (bool) {
        bytes memory suffix = ".testnet";
        if (s.length <= suffix.length) return false;
        for (uint256 i = 0; i < suffix.length; i++) {
            if (s[s.length - suffix.length + i] != suffix[i]) return false;
        }
        return true;
    }

    receive() external payable {
        revert("use lock");
    }
}

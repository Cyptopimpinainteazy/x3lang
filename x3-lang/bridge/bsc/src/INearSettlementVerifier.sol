// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

/**
 * @title INearSettlementVerifier
 * @notice Interface the issue #42 escrow requires from a NEAR finality verifier.
 * @dev NO IMPLEMENTATION OF THIS INTERFACE EXISTS IN THIS REPOSITORY. Building
 *      one is the job of issue #56 (authenticated NEAR light client). Until a
 *      reviewed implementation exists, `NearSettledBnbEscrow` has nothing it
 *      can be safely constructed with outside a local test chain.
 *
 *      `verifySettlement` MUST return `NEAR_SETTLEMENT_VERIFIED`
 *      (= `INearSettlementVerifier.verifySettlement.selector`) if and only if
 *      `proof` authenticates ALL of the following against a NEAR block that the
 *      verifier itself holds as FINAL (BFT-finalized, never optimistic or
 *      merely "executed"), on the NEAR chain whose genesis hash is
 *      `nearGenesisHash`:
 *        1. an ExecutionOutcome included in that block (outcome proof, outcome
 *           root, block Merkle proof and header hash all recomputed on-chain);
 *        2. `executor_id` byte-equal to `nearVault`;
 *        3. execution status SuccessValue or SuccessReceiptId. NEAR keeps the
 *           logs of FAILED receipts, so a matching log alone is not enough;
 *        4. one log entry byte-equal to
 *           "X3_NEAR_SETTLEMENT_V1 " || lowercase_hex(settlementCommitment);
 *        5. the code hash of account `nearVault` in that same block equal to
 *           `nearVaultCodeHash` (state proof), with no full-access key able to
 *           redeploy it (checked at deployment review, see the protocol doc).
 *      Any other input MUST revert or return a different value. The function
 *      MUST be view: the escrow calls it with STATICCALL.
 */
interface INearSettlementVerifier {
    function verifySettlement(
        bytes32 nearGenesisHash,
        string calldata nearVault,
        bytes32 nearVaultCodeHash,
        bytes32 settlementCommitment,
        bytes calldata proof
    ) external view returns (bytes4);

    /// @notice Genesis hash of the single NEAR chain this verifier follows.
    function nearGenesisHash() external view returns (bytes32);
}

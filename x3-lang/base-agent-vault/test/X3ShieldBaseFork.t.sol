// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;
import "../src/X3ShieldBaseVault.sol";

/// @notice Optional opt-in live RPC fork smoke. No public-chain transactions.
/// @dev Without X3_BASE_SEPOLIA_FORK_RPC this suite is skipped rather than
///      silently claiming a real-token compatibility proof.
interface VmFork {
    function envOr(string calldata name, string calldata fallbackValue) external returns (string memory);
    function skip(bool value) external;
    function createSelectFork(string calldata url) external returns (uint256);
    function createSelectFork(string calldata url, uint256 blockNumber) external returns (uint256);
    function prank(address account) external;
    function expectRevert() external;
    function sign(uint256 privateKey, bytes32 digest) external returns (uint8, bytes32, bytes32);
    function addr(uint256 privateKey) external returns (address);
}

contract X3ShieldBaseForkTest {
    VmFork private constant vm = VmFork(address(uint160(uint256(keccak256("hevm cheat code")))));
    address private constant TOKEN = 0x38d81431B9f76d9699AF7517040D5221D9dEA252;
    address private constant PUBLIC_TESTNET_SOURCE = 0x5aF0c1BF95B3A158f6Fc48e3DED35903b2fa0721;
    address private constant RECIPIENT = address(0xBEEF);
    uint256 private constant AMOUNT = 100_000; // 0.1 USDC in 6-decimal test units
    uint256 private constant SIGNER_KEY = 0xA11CE;
    bytes32 private constant NONCE = keccak256("x3-fork-only-fresh-nonce");

    function _assertReplayAndRedirectRefused(X3ShieldBaseVault pilot, uint256 deadline, bytes memory originalSignature)
        internal
    {
        vm.expectRevert();
        pilot.executePayment(NONCE, RECIPIENT, AMOUNT, deadline, originalSignature);

        bytes32 alternateNonce = keccak256("fork-only-unauthorized-recipient");
        (uint8 v, bytes32 r, bytes32 sigS) =
            vm.sign(SIGNER_KEY, pilot.paymentDigest(alternateNonce, address(0xBAD0), AMOUNT, deadline));
        vm.expectRevert();
        pilot.executePayment(alternateNonce, address(0xBAD0), AMOUNT, deadline, abi.encodePacked(r, sigS, v));
    }

    function testLiveForkRealTokenExecutesOnlyExactAgentSignedPayment() public {
        string memory rpc = vm.envOr("X3_BASE_SEPOLIA_FORK_RPC", string(""));
        if (bytes(rpc).length == 0) {
            vm.skip(true);
            return;
        }
        // Independently finalized source-proof checkpoint for the historical
        // real Tidepool trade, used only as read-only fork state.
        vm.createSelectFork(rpc, 55_557_819);
        require(block.chainid == 84532 && block.number == 55_557_819, "fork must be pinned finalized Base Sepolia");
        require(TOKEN.code.length > 0, "pinned test-USDC token not deployed");
        require(
            sha256(TOKEN.code) == 0xdb77a21a647b5973bac5728bf9f6977b56933006bf2f0730a14d109d33072c83,
            "deployed test-USDC bytecode does not match independently pinned SHA-256"
        );
        uint256 sourceBefore = IX3VaultERC20(TOKEN).balanceOf(PUBLIC_TESTNET_SOURCE);
        require(sourceBefore >= AMOUNT, "source account lacks 0.1 test USDC");

        X3ShieldBaseVault pilot;
        address scopedSigner = vm.addr(SIGNER_KEY);
        // Foundry impersonation is strictly confined to this local fork.
        vm.prank(PUBLIC_TESTNET_SOURCE);
        pilot = new X3ShieldBaseVault(
            TOKEN,
            scopedSigner,
            RECIPIENT,
            keccak256("new fork-only owner quote hash - NOT the old Tidepool quote"),
            keccak256("new fork-only program hash"),
            AMOUNT,
            AMOUNT,
            block.timestamp + 3600
        );
        // Actual token approval / transferFrom / transfer code executes under
        // a local fork. No owner EOA private key is requested, loaded, or signed.
        vm.prank(PUBLIC_TESTNET_SOURCE);
        require(IERC20Pilot(TOKEN).approve(address(pilot), AMOUNT), "real token approval failed");
        vm.prank(PUBLIC_TESTNET_SOURCE);
        pilot.deposit(AMOUNT);

        uint256 deadline = block.timestamp + 120;
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(SIGNER_KEY, pilot.paymentDigest(NONCE, RECIPIENT, AMOUNT, deadline));
        bytes memory agentSignature = abi.encodePacked(r, s, v);
        uint256 beneficiaryBefore = IX3VaultERC20(TOKEN).balanceOf(RECIPIENT);
        pilot.executePayment(NONCE, RECIPIENT, AMOUNT, deadline, agentSignature);
        require(
            IX3VaultERC20(TOKEN).balanceOf(RECIPIENT) == beneficiaryBefore + AMOUNT,
            "real forked ERC20 recipient was not credited"
        );
        require(
            IX3VaultERC20(TOKEN).balanceOf(address(pilot)) == 0 && pilot.spent() == AMOUNT,
            "vault escrow/ledger mismatch on local fork"
        );
        require(
            IX3VaultERC20(TOKEN).balanceOf(PUBLIC_TESTNET_SOURCE) == sourceBefore - AMOUNT,
            "simulated local-fork source token debit was not exact"
        );
        _assertReplayAndRedirectRefused(pilot, deadline, agentSignature);
    }
}

interface IERC20Pilot {
    function approve(address spender, uint256 amount) external returns (bool);
}

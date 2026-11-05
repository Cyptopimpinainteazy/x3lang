// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;
import "../src/NativeBnbHTLC.sol";

interface Vm {
    function warp(uint256) external;
    function deal(address, uint256) external;
    function chainId(uint256) external;
    function prank(address) external;
    function expectRevert() external;
}

contract NativeBnbHTLCTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    NativeBnbHTLC vault;
    bytes32 constant SECRET = bytes32(uint256(0xABC123));
    bytes32 constant ID = keccak256("x3-pair-001");
    bytes32 hashlock;
    address constant RECEIVER = address(0xBEEF);
    uint64 constant WAIT = 3600;
    receive() external payable {}

    function setUp() public {
        vm.warp(1_700_000_000);
        vm.deal(address(this), 10 ether);
        vault = new NativeBnbHTLC();
        hashlock = sha256(abi.encodePacked(SECRET));
    }

    function open() internal {
        vault.lock{value: 1 ether}(ID, RECEIVER, hashlock, uint64(block.timestamp) + WAIT);
    }

    function testDepositorReleasesBnbToLPWithCorrectSecret() public {
        open();
        uint256 beforeBal = RECEIVER.balance;
        vault.ownerRelease(ID, SECRET);
        require(RECEIVER.balance == beforeBal + 1 ether, "incorrect native payout");
        vm.expectRevert();
        vault.refund(ID);
    }

    function testWrongSecretNeverClaims() public {
        open();
        vm.expectRevert();
        vault.ownerRelease(ID, bytes32(uint256(7)));
    }

    function testOnlyDepositorCanAuthorizeBnbPayout() public {
        open();
        vm.prank(RECEIVER);
        vm.expectRevert();
        vault.ownerRelease(ID, SECRET);
    }

    function testLeakedPreimageCannotLetLPTakeBnbBeforeNearCredit() public {
        open();
        uint256 beforeBal = RECEIVER.balance;
        // Adversarial LP learns SECRET from a failed NEAR transaction.
        // Receiving that preimage must not authorize source-chain release.
        vm.prank(RECEIVER);
        vm.expectRevert();
        vault.ownerRelease(ID, SECRET);
        require(RECEIVER.balance == beforeBal, "stolen BNB via leaked preimage");
        require(address(vault).balance == 1 ether, "source escrow prematurely released");
    }

    function testRefundAfterExpiry() public {
        open();
        vm.warp(block.timestamp + WAIT);
        vault.refund(ID);
        require(address(vault).balance == 0, "refund escrow not released");
        vm.expectRevert();
        vault.refund(ID);
    }

    function testCannotRefundEarlyOrClaimLate() public {
        open();
        vm.expectRevert();
        vault.refund(ID);
        vm.warp(block.timestamp + WAIT);
        vm.prank(RECEIVER);
        vm.expectRevert();
        vault.ownerRelease(ID, SECRET);
    }

    function testCannotReuseSwapId() public {
        open();
        vm.expectRevert();
        vault.lock{value: 1 ether}(ID, RECEIVER, hashlock, uint64(block.timestamp) + WAIT);
    }

    function testCannotDeployThisPrototypeToBscMainnet() public {
        vm.chainId(56);
        vm.expectRevert();
        new NativeBnbHTLC();
    }

    function testCanDeployOnBscTestnet() public {
        vm.chainId(97);
        NativeBnbHTLC testVault = new NativeBnbHTLC();
        require(address(testVault) != address(0), "testnet contract missing");
    }

    function testCannotLockZeroOrInvalidExpiry() public {
        vm.expectRevert();
        vault.lock{value: 0}(ID, RECEIVER, hashlock, uint64(block.timestamp) + WAIT);
        vm.expectRevert();
        vault.lock{value: 1 ether}(ID, RECEIVER, hashlock, uint64(block.timestamp) + 10);
    }

    /// @notice Regression witness: once NEAR credited the depositor, the source
    /// depositor can still decline to call ownerRelease and reclaim all BNB.
    /// This is a deliberately successful demonstration of a safety violation,
    /// not a passing atomic-settlement proof. NEAR credit is an explicit
    /// scenario precondition; this Solidity-only test cannot verify NEAR state.
    function testCounterexampleNearCreditedButDepositorRefundsBnb() public {
        open();
        bool nearCreditFinalized = true; // Adversarial cross-chain scenario input.
        require(nearCreditFinalized, "scenario must include NEAR payout");
        uint256 lpBefore = RECEIVER.balance;
        uint256 depositorBefore = address(this).balance;

        // No permissionless BNB claim exists, even after the preimage is known.
        vm.prank(RECEIVER);
        vm.expectRevert();
        vault.ownerRelease(ID, SECRET);
        require(RECEIVER.balance == lpBefore, "LP unexpectedly paid");

        // The NEAR payout cannot stop the BSC depositor's independent refund.
        vm.warp(block.timestamp + WAIT);
        vault.refund(ID);
        require(address(this).balance == depositorBefore + 1 ether, "depositor did not recover BNB");
        require(RECEIVER.balance == lpBefore, "LP should still be unpaid");
    }

    /// @notice Once the owner has paid out BNB, neither a refund nor a second
    /// payout may occur, including on the timeout boundary.
    function testOwnerPayoutCannotBeReplayedOrRefunded() public {
        open();
        vault.ownerRelease(ID, SECRET);
        vm.expectRevert();
        vault.ownerRelease(ID, SECRET);
        vm.warp(block.timestamp + WAIT);
        vm.expectRevert();
        vault.refund(ID);
    }
}

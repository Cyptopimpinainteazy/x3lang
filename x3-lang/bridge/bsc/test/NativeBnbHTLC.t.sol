// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;
import "../src/NativeBnbHTLC.sol";
interface Vm {
    function warp(uint256) external;
    function deal(address, uint256) external;
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
    function testRecipientClaimsNativeBnbWithCorrectSecret() public {
        open();
        uint256 beforeBal = RECEIVER.balance;
        vm.prank(RECEIVER);
        vault.claim(ID, SECRET);
        require(RECEIVER.balance == beforeBal + 1 ether, "incorrect native payout");
        vm.expectRevert();
        vault.refund(ID);
    }
    function testWrongSecretNeverClaims() public {
        open();
        vm.prank(RECEIVER);
        vm.expectRevert();
        vault.claim(ID, bytes32(uint256(7)));
    }
    function testOnlyRecipientCanClaim() public {
        open();
        vm.prank(address(0xCAFE));
        vm.expectRevert();
        vault.claim(ID, SECRET);
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
        vault.claim(ID, SECRET);
    }
    function testCannotReuseSwapId() public {
        open();
        vm.expectRevert();
        vault.lock{value: 1 ether}(ID, RECEIVER, hashlock, uint64(block.timestamp) + WAIT);
    }
    function testCannotLockZeroOrInvalidExpiry() public {
        vm.expectRevert();
        vault.lock{value: 0}(ID, RECEIVER, hashlock, uint64(block.timestamp) + WAIT);
        vm.expectRevert();
        vault.lock{value: 1 ether}(ID, RECEIVER, hashlock, uint64(block.timestamp) + 10);
    }
}

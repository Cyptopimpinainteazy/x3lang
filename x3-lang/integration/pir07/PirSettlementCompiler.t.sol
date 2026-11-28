// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;
import "../src/NearSettledBnbEscrow.sol";

interface PirVm {
    function chainId(uint256) external;
    function warp(uint256) external;
    function deal(address,uint256) external;
    function prank(address) external;
    function expectRevert() external;
    function envBytes32(string calldata) external returns (bytes32);
}
contract NeverVerifyPir is INearSettlementVerifier {
    function nearGenesisHash() external pure returns(bytes32) {return bytes32(uint256(5));}
    function verifySettlement(bytes32,string calldata,bytes32,bytes32,bytes calldata)
        external pure returns(bytes4) {return bytes4(0);}
}
contract PirSettlementCompilerTest {
    PirVm constant vm=PirVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    NearSettledBnbEscrow escrow;
    address constant DEPOSITOR=address(0xA11CE);
    address constant LP=address(0xBEEF);
    bytes32 constant SALT=0x1111111111111111111111111111111111111111111111111111111111111111;
    bytes32 constant SECRET=0x0303030303030303030303030303030303030303030303030303030303030303;
    function setUp() public {
        vm.chainId(31337);
        vm.warp(1700000000);
        NeverVerifyPir verifier=new NeverVerifyPir();
        escrow=new NearSettledBnbEscrow(address(verifier),bytes32(uint256(5)),
            "x3-vault.testnet",bytes32(uint256(6)));
    }
    function testRustNativeParity() public {
        bytes32 binding=escrow.sourceBindingFor(31337,address(0xE5C40),DEPOSITOR,LP,1 ether,SALT);
        require(binding==vm.envBytes32("PIR07_BINDING"),"source binding mismatch");
        bytes32 window=escrow.windowIdFor("x3-vault.testnet","lp.testnet","alice.testnet",
            sha256(abi.encodePacked(SECRET)),1e24,1700000600000000000,1700001200000000000,binding);
        require(window==vm.envBytes32("PIR07_WINDOW"),"window mismatch");
        require(escrow.settlementCommitment(window,1)==vm.envBytes32("PIR07_LOCK"),"lock mismatch");
        require(escrow.settlementCommitment(window,2)==vm.envBytes32("PIR07_CLAIM"),"claim mismatch");
        require(escrow.settlementCommitment(window,3)==vm.envBytes32("PIR07_REFUND"),"refund mismatch");
    }
    function testCompilerHashCannotReplaceNativeProof() public {
        NearSettledBnbEscrow.LockTerms memory terms=NearSettledBnbEscrow.LockTerms({
            lp:LP,salt:SALT,nearInitiator:"lp.testnet",nearRecipient:"alice.testnet",
            hashlock:sha256(abi.encodePacked(SECRET)),nearAmountYocto:1e24,
            fundingDeadlineNs:1700000600000000000,nearExpiryNs:1700001200000000000
        });
        bytes32 id=escrow.expectedWindowId(DEPOSITOR,1 ether,terms);
        // A correct native commitment is not a proof, even for this exact escrow.
        bytes memory fakeProof=abi.encodePacked(escrow.settlementCommitment(id,1));
        vm.deal(DEPOSITOR,1 ether);
        vm.expectRevert();
        vm.prank(DEPOSITOR);
        escrow.lock{value:1 ether}(terms,fakeProof);
        require(escrow.lockedBnb()==0 && escrow.owedBnb()==0 && address(escrow).balance==0);
        require(DEPOSITOR.balance==1 ether);
        bytes memory claimProof=abi.encodePacked(escrow.settlementCommitment(id,2));
        bytes memory refundProof=abi.encodePacked(escrow.settlementCommitment(id,3));
        vm.expectRevert();
        escrow.settle(id,2,claimProof);
        vm.expectRevert();
        escrow.settle(id,3,refundProof);
    }
}

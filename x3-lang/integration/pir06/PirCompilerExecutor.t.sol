// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;
import "../src/X3PolicyExecutor.sol";

interface Vm {
    function chainId(uint256) external;
    function warp(uint256) external;
    function envBytes(string calldata) external returns (bytes memory);
    function envBytes32(string calldata) external returns (bytes32);
    function etch(address,bytes calldata) external;
    function prank(address) external;
    function expectRevert(bytes4) external;
    function deal(address,uint256) external;
}
contract FixtureTarget {
    address immutable executor;
    uint256 public number;
    constructor(address e){executor=e;}
    function set(uint256 n) external {
        require(msg.sender==executor,"only executor");
        number=n;
    }
}
contract PirCompilerExecutorTest {
    Vm constant vm=Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    address constant EXEC=0x1111111111111111111111111111111111111111;
    address constant TARGET=0x2222222222222222222222222222222222222222;
    address constant AGENT=0x3333333333333333333333333333333333333333;
    X3PolicyExecutor executor;
    FixtureTarget target;
    X3PolicyExecutor.Action action;
    bytes data;
    function setUp() public {
        vm.chainId(84532);
        vm.warp(100);
        // Fixed fixture addresses ONLY: copy real deployed runtime bytecode
        // including constructor immutables into an isolated Foundry VM.
        X3PolicyExecutor template=new X3PolicyExecutor(address(this));
        FixtureTarget targetTemplate=new FixtureTarget(EXEC);
        vm.etch(EXEC,address(template).code);
        vm.etch(TARGET,address(targetTemplate).code);
        executor=X3PolicyExecutor(EXEC);
        target=FixtureTarget(TARGET);
        action=abi.decode(vm.envBytes("X3_PIR_ACTION"),(X3PolicyExecutor.Action));
        data=abi.encodeCall(target.set,(7));
        require(action.chainId==84532 && action.executor==EXEC && action.target==TARGET);
        require(action.agent==AGENT && action.value==0 && action.expiry==1000);
        require(action.calldataHash==keccak256(data) && action.selector==target.set.selector);
        require(executor.actionId(action)==vm.envBytes32("X3_PIR_ACTION_ID"),"Rust/Solidity action ID mismatch");
        executor.setPolicy(action.policyHash);
        executor.setRule(TARGET,target.set.selector,true);
        require(executor.policyVersion()==action.policyVersion);
        executor.approve(action);
    }
    function testCompilerActionExecutesOnceAndDirectCallDenied() public {
        vm.prank(AGENT);
        (bool ok,)=TARGET.call(data);
        require(!ok && target.number()==0);
        vm.prank(AGENT);
        executor.execute(action,data);
        require(target.number()==7 && executor.usedNonce(action.nonce));
        vm.expectRevert(X3PolicyExecutor.Denied.selector);
        vm.prank(AGENT);
        executor.execute(action,data);
    }
    function testMutatedActionAndCalldataDenied() public {
        vm.expectRevert(X3PolicyExecutor.Denied.selector);
        vm.prank(AGENT);
        executor.execute(action,abi.encodeCall(target.set,(8)));
        vm.expectRevert(X3PolicyExecutor.Denied.selector);
        executor.execute(action,data);
        X3PolicyExecutor.Action memory changed=action;
        changed.policyHash=bytes32(uint256(1));
        vm.expectRevert(X3PolicyExecutor.Denied.selector);
        vm.prank(AGENT);
        executor.execute(changed,data);
        vm.deal(AGENT,1);
        vm.expectRevert(X3PolicyExecutor.Denied.selector);
        vm.prank(AGENT);
        executor.execute{value:1}(action,data);
        require(!executor.usedNonce(action.nonce) && target.number()==0);
    }
    function testExpiryAndRevocation() public {
        executor.revoke(executor.actionId(action));
        vm.expectRevert(X3PolicyExecutor.Denied.selector);
        vm.prank(AGENT);
        executor.execute(action,data);
        executor.approve(action);
        vm.warp(action.expiry);
        vm.expectRevert(X3PolicyExecutor.Denied.selector);
        vm.prank(AGENT);
        executor.execute(action,data);
        require(target.number()==0);
    }
}

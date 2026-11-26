// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import "../src/X3PolicyExecutor.sol";

interface Vm {
    function prank(address) external;
    function expectRevert(bytes4) external;
    function warp(uint256) external;
    function deal(address,uint256) external;
}

contract ProtectedTarget {
    address immutable executor;
    uint256 public amount;
    constructor(address e) { executor = e; }
    function set(uint256 n) external payable {
        require(msg.sender == executor, "only executor");
        require(n != 99, "target failure");
        amount = n;
    }
    function other() external {}
}

contract ReentryTarget {
    X3PolicyExecutor immutable executor;
    bool public blocked;
    constructor(X3PolicyExecutor e) { executor = e; }
    function attempt() external {
        X3PolicyExecutor.Action memory a;
        try executor.execute(a, "") { revert("reentry accepted"); }
        catch (bytes memory reason) {
            require(bytes4(reason) == X3PolicyExecutor.Reentrant.selector);
            blocked = true;
        }
    }
}

contract X3PolicyExecutorTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    address constant AGENT = address(0xA11CE);
    bytes32 constant POLICY = keccak256("fixture policy");
    X3PolicyExecutor e;
    ProtectedTarget t;
    bytes data;

    function setUp() public {
        e = new X3PolicyExecutor(address(this));
        t = new ProtectedTarget(address(e));
        e.setPolicy(POLICY);
        e.setRule(address(t), t.set.selector, true);
        data = abi.encodeCall(t.set, (7));
    }
    function action() internal view returns (X3PolicyExecutor.Action memory a) {
        a = X3PolicyExecutor.Action(block.chainid,address(e),address(t),t.set.selector,
            keccak256(data),0,AGENT,POLICY,e.policyVersion(),1,block.timestamp+100);
    }
    function run(X3PolicyExecutor.Action memory a, bytes memory d) internal {
        vm.prank(AGENT);
        e.execute(a,d);
    }
    function testAuthorizedAndDirectBypassDenied() public {
        vm.prank(AGENT);
        (bool ok,) = address(t).call(data);
        require(!ok && t.amount()==0);
        X3PolicyExecutor.Action memory a=action();
        e.approve(a); run(a,data);
        require(t.amount()==7 && e.usedNonce(a.nonce));
    }
    function testUnknownTarget() public {
        X3PolicyExecutor.Action memory a=action();
        a.target=address(new ProtectedTarget(address(e)));
        vm.expectRevert(X3PolicyExecutor.Denied.selector); e.approve(a);
    }
    function testUnknownSelector() public {
        X3PolicyExecutor.Action memory a=action();
        a.selector=t.other.selector;
        vm.expectRevert(X3PolicyExecutor.Denied.selector); e.approve(a);
    }
    function testModifiedCalldata() public {
        X3PolicyExecutor.Action memory a=action(); e.approve(a);
        vm.expectRevert(X3PolicyExecutor.Denied.selector);
        run(a,abi.encodeCall(t.set,(8)));
        require(t.amount()==0 && !e.usedNonce(a.nonce));
    }
    function testReplay() public {
        X3PolicyExecutor.Action memory a=action(); e.approve(a); run(a,data);
        vm.expectRevert(X3PolicyExecutor.Denied.selector); run(a,data);
    }
    function testExpiry() public {
        X3PolicyExecutor.Action memory a=action(); e.approve(a); vm.warp(a.expiry);
        vm.expectRevert(X3PolicyExecutor.Denied.selector); run(a,data);
    }
    function testWrongChainAndExecutor() public {
        X3PolicyExecutor.Action memory a=action(); a.chainId++;
        vm.expectRevert(X3PolicyExecutor.Denied.selector); e.approve(a);
        a=action(); a.executor=address(t);
        vm.expectRevert(X3PolicyExecutor.Denied.selector); e.approve(a);
    }
    function testWrongAgentAndUnauthorizedAdmin() public {
        X3PolicyExecutor.Action memory a=action(); e.approve(a);
        vm.expectRevert(X3PolicyExecutor.Denied.selector); e.execute(a,data);
        vm.prank(AGENT);
        vm.expectRevert(X3PolicyExecutor.Denied.selector); e.setPolicy(keccak256("evil"));
    }
    function testRevocationAndPolicyEpoch() public {
        X3PolicyExecutor.Action memory a=action(); e.approve(a); e.revoke(e.actionId(a));
        vm.expectRevert(X3PolicyExecutor.Denied.selector); run(a,data);
        e.approve(a); e.setPolicy(POLICY);
        vm.expectRevert(X3PolicyExecutor.Denied.selector); run(a,data);
    }
    function testPauseInvalidatesEvenAfterResume() public {
        X3PolicyExecutor.Action memory a=action(); e.approve(a); e.setPaused(true);
        vm.expectRevert(X3PolicyExecutor.Denied.selector); run(a,data);
        e.setPaused(false);
        vm.expectRevert(X3PolicyExecutor.Denied.selector); run(a,data);
    }
    function test_RevertWhen_TargetCallFailsRollsBack() public {
        data=abi.encodeCall(t.set,(99));
        X3PolicyExecutor.Action memory a=action(); e.approve(a);
        vm.expectRevert(X3PolicyExecutor.CallFailed.selector); run(a,data);
        require(!e.usedNonce(a.nonce) && e.approved(e.actionId(a)) && t.amount()==0);
    }
    function testReentry() public {
        ReentryTarget r=new ReentryTarget(e);
        e.setRule(address(r),r.attempt.selector,true);
        data=abi.encodeCall(r.attempt,());
        X3PolicyExecutor.Action memory a=action(); a.target=address(r); a.selector=r.attempt.selector;
        e.approve(a); run(a,data); require(r.blocked());
    }
    function testValueBinding() public {
        X3PolicyExecutor.Action memory a=action(); e.approve(a);
        vm.deal(AGENT,1 ether);
        vm.prank(AGENT); vm.expectRevert(X3PolicyExecutor.Denied.selector);
        e.execute{value:1}(a,data);
    }
    function testRuleChangeInvalidates() public {
        X3PolicyExecutor.Action memory a=action(); e.approve(a);
        e.setRule(address(t),t.set.selector,false);
        vm.expectRevert(X3PolicyExecutor.Denied.selector); run(a,data);
    }
}

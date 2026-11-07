// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;
import "../src/X3ShieldBaseVault.sol";

interface Vm {
    function warp(uint256) external;
    function getBlockTimestamp() external view returns (uint256);
    function chainId(uint256) external;
    function assume(bool) external;
    function prank(address) external;
    function expectRevert() external;
    function addr(uint256) external returns (address);
    function sign(uint256, bytes32) external returns (uint8, bytes32, bytes32);
}

contract PilotToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    uint256 public transferTaxBps;

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
    }

    function approve(address to, uint256 amount) external returns (bool) {
        allowance[msg.sender][to] = amount;
        return true;
    }

    function setTax(uint256 bps) external {
        transferTaxBps = bps;
    }

    function transfer(address to, uint256 amount) public virtual returns (bool) {
        _move(msg.sender, to, amount);
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        uint256 a = allowance[from][msg.sender];
        require(a >= amount, "allowance");
        allowance[from][msg.sender] = a - amount;
        _move(from, to, amount);
        return true;
    }

    function _move(address from, address to, uint256 amount) internal {
        require(balanceOf[from] >= amount, "funds");
        balanceOf[from] -= amount;
        uint256 delivered = amount - (amount * transferTaxBps / 10_000);
        balanceOf[to] += delivered;
    }
}

/// @dev Disposable test-only token attempts a second valid agent-signed
/// transfer during the first token interaction. Production vault must refuse
/// the reentrant request without destroying the permitted outer payment.
contract ReentrantPilotToken is PilotToken {
    address public reentryTarget;
    bytes public reentryCall;
    bool public attempted;
    bool public reentrySucceeded;

    function configure(address vault_, bytes memory callData) external {
        reentryTarget = vault_;
        reentryCall = callData;
    }

    function transfer(address to, uint256 amount) public override returns (bool) {
        if (!attempted && reentryTarget != address(0)) {
            attempted = true;
            (bool ok,) = reentryTarget.call(reentryCall);
            reentrySucceeded = ok;
        }
        return super.transfer(to, amount);
    }
}

contract X3ShieldBaseVaultTest {
    Vm internal constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    uint256 constant AGENT_KEY = 0xA11CE;
    uint256 constant ATTACKER_KEY = 0xBAD;
    bytes32 constant QUOTE = keccak256("real-quote-commitment");
    bytes32 constant PROGRAM = keccak256("x3-program-commitment");
    bytes32 constant NONCE = keccak256("x3-one-shot-1");
    uint256 constant USDC = 1e6;
    address constant BENEFICIARY = address(0xBEEF);
    address constant ADVERSARY = address(0xBAD0);
    PilotToken token;
    X3ShieldBaseVault vault;
    address agent;
    uint256 expiry;

    function setUp() public {
        vm.warp(1_700_000_000);
        vm.chainId(31337);
        agent = vm.addr(AGENT_KEY);
        expiry = block.timestamp + 3600;
        token = new PilotToken();
        vault = new X3ShieldBaseVault(address(token), agent, BENEFICIARY, QUOTE, PROGRAM, USDC, 3 * USDC, expiry);
        token.mint(address(this), 10 * USDC);
        token.approve(address(vault), type(uint256).max);
        vault.deposit(5 * USDC);
    }

    function signature(
        uint256 key,
        X3ShieldBaseVault target,
        bytes32 nonce,
        address to,
        uint256 amount,
        uint256 deadline
    ) internal returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, target.paymentDigest(nonce, to, amount, deadline));
        return abi.encodePacked(r, s, v);
    }

    function goodSignature(bytes32 nonce, uint256 amount) internal returns (bytes memory) {
        return signature(AGENT_KEY, vault, nonce, BENEFICIARY, amount, block.timestamp + 120);
    }

    function testForgedAgentSignatureCannotMoveVaultFunds() public {
        bytes memory forged = signature(ATTACKER_KEY, vault, NONCE, BENEFICIARY, USDC, block.timestamp + 120);
        vm.prank(ADVERSARY);
        vm.expectRevert();
        vault.executePayment(NONCE, BENEFICIARY, USDC, block.timestamp + 120, forged);
    }

    function testAuthorizedAgentSignaturePermitsExactPayment() public {
        bytes memory signed = goodSignature(NONCE, USDC);
        vm.prank(ADVERSARY);
        vault.executePayment(NONCE, BENEFICIARY, USDC, block.timestamp + 120, signed);
        require(token.balanceOf(BENEFICIARY) == USDC, "recipient did not receive exact 1 USDC");
        require(vault.spent() == USDC && vault.usedNonces(NONCE), "receipt accounting failed");
    }

    function testRecipientRedirectRejectedEvenWhenAgentSignedIt() public {
        bytes memory signed = signature(AGENT_KEY, vault, NONCE, ADVERSARY, USDC, block.timestamp + 120);
        vm.expectRevert();
        vault.executePayment(NONCE, ADVERSARY, USDC, block.timestamp + 120, signed);
    }

    function testNonceReplayNeverExecutesTwice() public {
        bytes memory signed = goodSignature(NONCE, USDC);
        vault.executePayment(NONCE, BENEFICIARY, USDC, block.timestamp + 120, signed);
        vm.expectRevert();
        vault.executePayment(NONCE, BENEFICIARY, USDC, block.timestamp + 120, signed);
    }

    function testSignatureCannotBeReusedWithDifferentAmount() public {
        bytes memory signed = goodSignature(NONCE, USDC);
        vm.expectRevert();
        vault.executePayment(NONCE, BENEFICIARY, USDC / 2, block.timestamp + 120, signed);
        require(!vault.usedNonces(NONCE) && vault.spent() == 0, "failed signature consumed funds");
    }

    function testExpiredSignedPaymentRefused() public {
        uint256 deadline = vm.getBlockTimestamp() + 60;
        bytes memory signed = signature(AGENT_KEY, vault, NONCE, BENEFICIARY, USDC, deadline);
        vm.warp(deadline + 1);
        vm.expectRevert();
        vault.executePayment(NONCE, BENEFICIARY, USDC, deadline, signed);
    }

    function testVaultExpiryRefusesOtherwiseValidPayment() public {
        bytes memory signed = signature(AGENT_KEY, vault, NONCE, BENEFICIARY, USDC, expiry);
        vm.warp(expiry);
        vm.expectRevert();
        vault.executePayment(NONCE, BENEFICIARY, USDC, expiry, signed);
    }

    function testOwnerRevocationStopsAgentAndOnlyOwnerCanRevoke() public {
        bytes memory signed = goodSignature(NONCE, USDC);
        vm.prank(ADVERSARY);
        vm.expectRevert();
        vault.revoke();
        require(!vault.revoked(), "untrusted caller revoked");
        vault.revoke();
        require(vault.revoked(), "owner revocation not stored");
        vm.expectRevert();
        vault.executePayment(NONCE, BENEFICIARY, USDC, block.timestamp + 120, signed);
    }

    function testPerPaymentAndCumulativeBudgetAreBothEnforced() public {
        bytes memory over = signature(AGENT_KEY, vault, NONCE, BENEFICIARY, USDC + 1, block.timestamp + 120);
        vm.expectRevert();
        vault.executePayment(NONCE, BENEFICIARY, USDC + 1, block.timestamp + 120, over);
        for (uint256 i = 1; i <= 3; i++) {
            bytes32 n = keccak256(abi.encodePacked("nonce", i));
            bytes memory sig = goodSignature(n, USDC);
            vault.executePayment(n, BENEFICIARY, USDC, block.timestamp + 120, sig);
        }
        require(vault.spent() == 3 * USDC, "missing bounded spending");
        bytes32 fourth = keccak256("fourth");
        bytes memory last = goodSignature(fourth, USDC);
        vm.expectRevert();
        vault.executePayment(fourth, BENEFICIARY, USDC, block.timestamp + 120, last);
        require(vault.spent() == 3 * USDC && !vault.usedNonces(fourth), "total budget bypassed");
    }

    function testSignedOrderCannotBeReplayedOnDifferentVault() public {
        bytes memory signed = goodSignature(NONCE, USDC);
        X3ShieldBaseVault other =
            new X3ShieldBaseVault(address(token), agent, BENEFICIARY, QUOTE, PROGRAM, USDC, 3 * USDC, expiry);
        token.approve(address(other), type(uint256).max);
        other.deposit(2 * USDC);
        vm.expectRevert();
        other.executePayment(NONCE, BENEFICIARY, USDC, block.timestamp + 120, signed);
    }

    function testSignedOrderCannotBeReplayedOnDifferentChainId() public {
        bytes memory signed = goodSignature(NONCE, USDC);
        vm.chainId(84532);
        vm.expectRevert();
        vault.executePayment(NONCE, BENEFICIARY, USDC, block.timestamp + 120, signed);
    }

    function testPinnedQuoteAndProgramCommitmentCannotBeSwitched() public {
        bytes memory signed = goodSignature(NONCE, USDC);
        X3ShieldBaseVault changedPolicy = new X3ShieldBaseVault(
            address(token), agent, BENEFICIARY, keccak256("another-quote"), PROGRAM, USDC, 3 * USDC, expiry
        );
        token.approve(address(changedPolicy), type(uint256).max);
        changedPolicy.deposit(USDC);
        vm.expectRevert();
        changedPolicy.executePayment(NONCE, BENEFICIARY, USDC, block.timestamp + 120, signed);
        require(changedPolicy.quoteHash() != vault.quoteHash(), "policy binding test broken");
    }

    function testOwnerOnlyDepositsAndOnlyRecoversAfterRevocation() public {
        vm.expectRevert();
        vault.recover(USDC);
        vm.prank(ADVERSARY);
        vm.expectRevert();
        vault.deposit(USDC);
        vm.prank(ADVERSARY);
        vm.expectRevert();
        vault.recover(USDC);
        vault.revoke();
        uint256 before = token.balanceOf(address(this));
        vault.recover(5 * USDC);
        require(token.balanceOf(address(this)) == before + 5 * USDC, "owner did not recover escrow");
        require(token.balanceOf(address(vault)) == 0, "recovery left value");
    }

    function testRecoveryAfterExpiryWithoutRevocation() public {
        vm.warp(expiry);
        vault.recover(2 * USDC);
        require(token.balanceOf(address(vault)) == 3 * USDC, "expired recovery failed");
    }

    function testFeeOnTransferDepositRejectedAndRolledBack() public {
        token.setTax(100);
        uint256 before = token.balanceOf(address(vault));
        vm.expectRevert();
        vault.deposit(USDC);
        require(token.balanceOf(address(vault)) == before, "bad deposit did not roll back");
    }

    function testFeeOnTransferPayoutRejectedWithoutConsumingNonce() public {
        bytes memory signed = goodSignature(NONCE, USDC);
        token.setTax(100);
        uint256 before = token.balanceOf(address(vault));
        vm.expectRevert();
        vault.executePayment(NONCE, BENEFICIARY, USDC, block.timestamp + 120, signed);
        require(vault.spent() == 0 && !vault.usedNonces(NONCE), "taxed payout burned authorization");
        require(token.balanceOf(address(vault)) == before, "taxed payout drained vault");
    }

    function testMalformedAndMalleableSignaturesRejected() public {
        bytes memory valid = goodSignature(NONCE, USDC);
        bytes memory shortSig = new bytes(64);
        vm.expectRevert();
        vault.executePayment(NONCE, BENEFICIARY, USDC, block.timestamp + 120, shortSig);
        bytes memory highS = abi.encodePacked(bytes32(uint256(9)), bytes32(type(uint256).max), bytes1(uint8(27)));
        vm.expectRevert();
        vault.executePayment(NONCE, BENEFICIARY, USDC, block.timestamp + 120, highS);
        valid[64] = bytes1(uint8(29));
        vm.expectRevert();
        vault.executePayment(NONCE, BENEFICIARY, USDC, block.timestamp + 120, valid);
    }

    function testAgentCannotTransferVaultTokensDirectly() public {
        vm.prank(agent);
        vm.expectRevert();
        token.transferFrom(address(vault), ADVERSARY, USDC);
        require(token.balanceOf(address(vault)) == 5 * USDC, "agent bypassed custody");
    }

    function testOwnerMustNotBeAgentAndPolicyMustBeBound() public {
        vm.expectRevert();
        new X3ShieldBaseVault(address(token), address(this), BENEFICIARY, QUOTE, PROGRAM, USDC, 3 * USDC, expiry);
        vm.expectRevert();
        new X3ShieldBaseVault(address(token), agent, BENEFICIARY, bytes32(0), PROGRAM, USDC, 3 * USDC, expiry);
        vm.expectRevert();
        new X3ShieldBaseVault(address(token), agent, BENEFICIARY, QUOTE, bytes32(0), USDC, 3 * USDC, expiry);
        vm.expectRevert();
        new X3ShieldBaseVault(address(token), agent, BENEFICIARY, QUOTE, PROGRAM, 0, 3 * USDC, expiry);
    }

    function testMainnetAndBscMainnetCannotDeployPilot() public {
        vm.chainId(1);
        vm.expectRevert();
        new X3ShieldBaseVault(address(token), agent, BENEFICIARY, QUOTE, PROGRAM, USDC, 3 * USDC, expiry);
        vm.chainId(56);
        vm.expectRevert();
        new X3ShieldBaseVault(address(token), agent, BENEFICIARY, QUOTE, PROGRAM, USDC, 3 * USDC, expiry);
    }

    function testBaseSepoliaChainIdAllowsPilotDeployment() public {
        vm.chainId(84532);
        X3ShieldBaseVault pilot =
            new X3ShieldBaseVault(address(token), agent, BENEFICIARY, QUOTE, PROGRAM, USDC, 3 * USDC, expiry);
        require(address(pilot) != address(0), "testnet constructor refused");
    }

    function testSameNonceCannotBeSubmittedByTwoCompetingRelayers() public {
        bytes memory signed = goodSignature(NONCE, USDC);
        vm.prank(ADVERSARY);
        vault.executePayment(NONCE, BENEFICIARY, USDC, block.timestamp + 120, signed);
        vm.prank(address(0xCAFE));
        vm.expectRevert();
        vault.executePayment(NONCE, BENEFICIARY, USDC, block.timestamp + 120, signed);
        require(token.balanceOf(BENEFICIARY) == USDC, "duplicate relayers changed payment");
    }

    function testFuzzCorrectlySignedUnapprovedRecipientNeverPays(address unwanted) public {
        vm.assume(unwanted != BENEFICIARY && unwanted != address(0));
        bytes memory signed = signature(AGENT_KEY, vault, NONCE, unwanted, USDC, block.timestamp + 120);
        vm.expectRevert();
        vault.executePayment(NONCE, unwanted, USDC, block.timestamp + 120, signed);
        require(vault.spent() == 0 && !vault.usedNonces(NONCE), "unauthorized pay consumed budget");
    }

    function testFuzzCorrectlySignedAmountOverCapNeverPays(uint128 excess) public {
        vm.assume(excess > 0);
        uint256 amount = USDC + uint256(excess);
        bytes memory signed = signature(AGENT_KEY, vault, NONCE, BENEFICIARY, amount, block.timestamp + 120);
        vm.expectRevert();
        vault.executePayment(NONCE, BENEFICIARY, amount, block.timestamp + 120, signed);
        require(vault.spent() == 0 && !vault.usedNonces(NONCE), "over-cap transfer consumed budget");
    }

    function testFuzzWrongNonceSignatureNeverPays(bytes32 alternateNonce) public {
        vm.assume(alternateNonce != NONCE);
        bytes memory signed = goodSignature(NONCE, USDC);
        vm.expectRevert();
        vault.executePayment(alternateNonce, BENEFICIARY, USDC, block.timestamp + 120, signed);
        require(vault.spent() == 0 && !vault.usedNonces(alternateNonce), "mismatched nonce consumed budget");
    }

    function testMaliciousTokenReentryCannotExecuteSecondValidSignedPayment() public {
        ReentrantPilotToken malicious = new ReentrantPilotToken();
        X3ShieldBaseVault guarded =
            new X3ShieldBaseVault(address(malicious), agent, BENEFICIARY, QUOTE, PROGRAM, USDC, 3 * USDC, expiry);
        malicious.mint(address(this), 5 * USDC);
        malicious.approve(address(guarded), type(uint256).max);
        guarded.deposit(3 * USDC);

        bytes32 one = keccak256("approved-one");
        bytes32 two = keccak256("approved-two");
        uint256 deadline = block.timestamp + 120;
        bytes memory first = signature(AGENT_KEY, guarded, one, BENEFICIARY, USDC, deadline);
        bytes memory second = signature(AGENT_KEY, guarded, two, BENEFICIARY, USDC, deadline);

        malicious.configure(
            address(guarded),
            abi.encodeWithSelector(X3ShieldBaseVault.executePayment.selector, two, BENEFICIARY, USDC, deadline, second)
        );
        guarded.executePayment(one, BENEFICIARY, USDC, deadline, first);
        require(malicious.attempted(), "malicious ERC20 failed to attempt reentry");
        require(!malicious.reentrySucceeded(), "reentrant second signed payment succeeded");
        require(
            guarded.spent() == USDC && guarded.usedNonces(one) && !guarded.usedNonces(two), "reentry mutated policy"
        );
        require(malicious.balanceOf(BENEFICIARY) == USDC, "incorrect net payout");
    }
}

// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

/// @notice TESTNET-ONLY, non-upgradeable, single-token EIP-712 scoped-agent vault.
/// @dev Fund the VAULT, never the agent's unrestricted EOA. Owner/admin key
///      must stay outside the agent runtime. No arbitrary target/call/delegatecall.
interface IX3VaultERC20 {
    function balanceOf(address account) external view returns (uint256);
    function transfer(address recipient, uint256 amount) external returns (bool);
    function transferFrom(address from, address recipient, uint256 amount) external returns (bool);
}

contract X3ShieldBaseVault {
    bytes32 private constant DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    bytes32 private constant PAYMENT_TYPEHASH = keccak256(
        "AgentPayment(bytes32 nonce,address recipient,uint256 amount,uint256 deadline,bytes32 quoteHash,bytes32 programHash)"
    );
    bytes32 private constant NAME_HASH = keccak256("X3 Shield Base Vault");
    bytes32 private constant VERSION_HASH = keccak256("1");
    // EIP-2 low-S restriction, to reject malleated ECDSA signatures.
    uint256 private constant MAX_LOW_S = 0x7fffffffffffffffffffffffffffffff5d576e7357a4501ddfe92f46681b20a0;
    // Only the independently reviewed test-USDC ERC-20 at this exact address
    // and exact runtime bytecode may be selected on public Base Sepolia.
    // Foundry local chain 31337 may use disposable adversarial test tokens.
    address private constant BASE_SEPOLIA_USDC = 0x38d81431B9f76d9699AF7517040D5221D9dEA252;
    bytes32 private constant BASE_SEPOLIA_USDC_SHA256 =
        hex"db77a21a647b5973bac5728bf9f6977b56933006bf2f0730a14d109d33072c83";

    address public immutable owner;
    address public immutable asset;
    address public immutable agent;
    address public immutable recipient;
    bytes32 public immutable quoteHash;
    bytes32 public immutable programHash;
    bytes32 public immutable tokenCodeHash;
    uint256 public immutable maxPerPayment;
    uint256 public immutable maxTotal;
    uint256 public immutable expiresAt;

    uint256 public spent;
    bool public revoked;
    bool private _entered;
    mapping(bytes32 => bool) public usedNonces;

    event Deposited(address indexed owner, uint256 amount);
    event PaymentExecuted(bytes32 indexed nonce, address indexed recipient, uint256 amount, bytes32 digest);
    event Revoked(address indexed owner);
    event Recovered(address indexed owner, uint256 amount);

    modifier onlyOwner() {
        require(msg.sender == owner, "OWNER_ONLY");
        _;
    }

    modifier nonReentrant() {
        require(!_entered, "REENTRANT");
        _entered = true;
        _;
        _entered = false;
    }

    /// @dev Chain 84532 is Base Sepolia. Chain 31337 is a local Foundry test.
    ///      Never deploy this experimental prototype on a production chain.
    constructor(
        address asset_,
        address agent_,
        address recipient_,
        bytes32 quoteHash_,
        bytes32 programHash_,
        uint256 maxPerPayment_,
        uint256 maxTotal_,
        uint256 expiresAt_
    ) {
        require(block.chainid == 84532 || block.chainid == 31337, "TESTNET_ONLY");
        require(asset_ != address(0) && asset_.code.length > 0, "INVALID_TOKEN");
        if (block.chainid == 84532) {
            require(asset_ == BASE_SEPOLIA_USDC, "UNAPPROVED_BASE_TEST_TOKEN");
            require(sha256(asset_.code) == BASE_SEPOLIA_USDC_SHA256, "BASE_TOKEN_BYTECODE_MISMATCH");
        }
        require(agent_ != address(0) && recipient_ != address(0), "ZERO_PRINCIPAL");
        require(agent_ != msg.sender && recipient_ != agent_ && recipient_ != address(this), "ROLE_COLLISION");
        require(quoteHash_ != bytes32(0) && programHash_ != bytes32(0), "UNBOUND_POLICY");
        require(maxPerPayment_ > 0 && maxTotal_ >= maxPerPayment_, "INVALID_BUDGET");
        require(expiresAt_ > block.timestamp, "ALREADY_EXPIRED");

        owner = msg.sender;
        asset = asset_;
        agent = agent_;
        recipient = recipient_;
        quoteHash = quoteHash_;
        programHash = programHash_;
        maxPerPayment = maxPerPayment_;
        maxTotal = maxTotal_;
        expiresAt = expiresAt_;
        tokenCodeHash = asset_.codehash;
    }

    function domainSeparator() public view returns (bytes32) {
        return keccak256(abi.encode(DOMAIN_TYPEHASH, NAME_HASH, VERSION_HASH, block.chainid, address(this)));
    }

    function paymentDigest(bytes32 nonce, address to, uint256 amount, uint256 deadline) public view returns (bytes32) {
        bytes32 structHash =
            keccak256(abi.encode(PAYMENT_TYPEHASH, nonce, to, amount, deadline, quoteHash, programHash));
        return keccak256(abi.encodePacked("\x19\x01", domainSeparator(), structHash));
    }

    /// @notice Owner deposits exact ERC-20 units to contract custody.
    /// @dev Check actual balance delta to refuse taxed/deflationary deposits.
    function deposit(uint256 amount) external nonReentrant onlyOwner {
        require(!revoked && block.timestamp < expiresAt, "VAULT_INACTIVE");
        require(amount > 0, "ZERO_DEPOSIT");
        _checkTokenCode();
        uint256 beforeBalance = _balance(address(this));
        // Event is emitted before external interaction; EVM automatically
        // reverts the log if any subsequent check or token call fails.
        emit Deposited(msg.sender, amount);
        _callToken(abi.encodeCall(IX3VaultERC20.transferFrom, (msg.sender, address(this), amount)));
        uint256 afterBalance = _balance(address(this));
        require(afterBalance >= beforeBalance && afterBalance - beforeBalance == amount, "NONEXACT_DEPOSIT");
    }

    /// @notice Anyone can relay an agent-SIGNED order. The agent key cannot
    ///         change the owner's pinned asset, destination, quote or program.
    /// @dev All state is reserved before the ERC-20 external call. On failure
    ///      EVM rollback restores nonce and budget atomically.
    function executePayment(bytes32 nonce, address to, uint256 amount, uint256 deadline, bytes calldata signature)
        external
        nonReentrant
    {
        require(!revoked && block.timestamp < expiresAt, "OWNER_POLICY_INACTIVE");
        require(block.timestamp <= deadline && deadline <= expiresAt, "PAYMENT_EXPIRED");
        require(nonce != bytes32(0) && !usedNonces[nonce], "NONCE_USED");
        require(to == recipient, "RECIPIENT_DENIED");
        require(amount > 0 && amount <= maxPerPayment, "PAYMENT_CAP");
        require(spent <= maxTotal && amount <= maxTotal - spent, "TOTAL_BUDGET");
        _checkTokenCode();

        bytes32 digest = paymentDigest(nonce, to, amount, deadline);
        require(_recoverSigner(digest, signature) == agent, "AGENT_SIGNATURE_INVALID");

        uint256 beforeVault = _balance(address(this));
        uint256 beforeRecipient = _balance(recipient);
        require(beforeVault >= amount, "INSUFFICIENT_ESCROW");

        usedNonces[nonce] = true;
        spent += amount;
        // Reverted token calls erase this event and the spent/nonce mutation.
        emit PaymentExecuted(nonce, to, amount, digest);
        _callToken(abi.encodeCall(IX3VaultERC20.transfer, (to, amount)));

        uint256 afterVault = _balance(address(this));
        uint256 afterRecipient = _balance(recipient);
        require(
            beforeVault >= afterVault && beforeVault - afterVault == amount && afterRecipient >= beforeRecipient
                && afterRecipient - beforeRecipient == amount,
            "NONEXACT_PAYOUT"
        );
    }

    function revoke() external onlyOwner {
        require(!revoked, "ALREADY_REVOKED");
        revoked = true;
        emit Revoked(msg.sender);
    }

    /// @notice Only the separate owner key can recover remaining escrow,
    ///         and only after terminating or expiring the agent mandate.
    function recover(uint256 amount) external nonReentrant onlyOwner {
        require(revoked || block.timestamp >= expiresAt, "POLICY_STILL_ACTIVE");
        require(amount > 0, "ZERO_RECOVERY");
        _checkTokenCode();
        uint256 beforeVault = _balance(address(this));
        uint256 beforeOwner = _balance(owner);
        require(beforeVault >= amount, "INSUFFICIENT_ESCROW");
        emit Recovered(owner, amount);
        _callToken(abi.encodeCall(IX3VaultERC20.transfer, (owner, amount)));
        uint256 afterVault = _balance(address(this));
        uint256 afterOwner = _balance(owner);
        require(
            beforeVault >= afterVault && beforeVault - afterVault == amount && afterOwner >= beforeOwner
                && afterOwner - beforeOwner == amount,
            "NONEXACT_RECOVERY"
        );
    }

    function _checkTokenCode() private view {
        require(asset.codehash == tokenCodeHash, "TOKEN_CODE_CHANGED");
    }

    function _balance(address account) private view returns (uint256) {
        (bool ok, bytes memory data) = asset.staticcall(abi.encodeCall(IX3VaultERC20.balanceOf, (account)));
        require(ok && data.length == 32, "TOKEN_BALANCE_UNAVAILABLE");
        return abi.decode(data, (uint256));
    }

    function _callToken(bytes memory data) private {
        require(_entered, "UNGUARDED_TOKEN_CALL");
        (bool ok, bytes memory result) = asset.call(data);
        require(
            ok && (result.length == 0 || (result.length == 32 && abi.decode(result, (bool)))), "ERC20_TRANSFER_FAILED"
        );
    }

    function _recoverSigner(bytes32 digest, bytes calldata signature) private pure returns (address) {
        if (signature.length != 65) return address(0);
        bytes32 r;
        bytes32 s;
        uint8 v;
        assembly {
            r := calldataload(signature.offset)
            s := calldataload(add(signature.offset, 32))
            v := byte(0, calldataload(add(signature.offset, 64)))
        }
        if ((v != 27 && v != 28) || uint256(s) == 0 || uint256(s) > MAX_LOW_S) return address(0);
        return ecrecover(digest, v, r, s);
    }
}

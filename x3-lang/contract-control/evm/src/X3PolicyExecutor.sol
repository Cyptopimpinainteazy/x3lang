// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

/// @notice Experimental exact-action executor; not audited or approved for funds.
contract X3PolicyExecutor {
    error Denied();
    error Reentrant();
    error CallFailed();

    struct Action {
        uint256 chainId;
        address executor;
        address target;
        bytes4 selector;
        bytes32 calldataHash;
        uint256 value;
        address agent;
        bytes32 policyHash;
        uint256 policyVersion;
        uint256 nonce;
        uint256 expiry;
    }

    bytes32 public constant ACTION_DOMAIN = keccak256("X3ContractActionV1");
    address public immutable owner;
    bytes32 public policyHash;
    uint256 public policyVersion;
    bool public paused;
    bool private entered;
    mapping(address => mapping(bytes4 => bytes32)) public ruleCodeHash;
    mapping(bytes32 => bool) public approved;
    mapping(uint256 => bool) public usedNonce;

    event PolicyChanged(bytes32 indexed policyHash, uint256 version);
    event RuleChanged(address indexed target, bytes4 indexed selector, bytes32 codeHash, uint256 version);
    event PauseChanged(bool paused, uint256 version);
    event ActionApproved(bytes32 indexed id, address indexed agent, uint256 nonce);
    event ActionRevoked(bytes32 indexed id);
    event ActionExecuted(bytes32 indexed id, address indexed agent, address indexed target, uint256 nonce);

    constructor(address initialOwner) {
        if (initialOwner == address(0)) revert Denied();
        owner = initialOwner;
    }

    modifier onlyOwnerUnlocked() {
        if (entered) revert Reentrant();
        if (msg.sender != owner) revert Denied();
        _;
    }

    function setPolicy(bytes32 hash) external onlyOwnerUnlocked {
        if (hash == bytes32(0)) revert Denied();
        policyHash = hash;
        ++policyVersion;
        emit PolicyChanged(hash, policyVersion);
    }

    function setRule(address target, bytes4 selector, bool enabled) external onlyOwnerUnlocked {
        if (target == address(this) || target == address(0)) revert Denied();
        if (enabled && target.code.length == 0) revert Denied();
        bytes32 hash = enabled ? target.codehash : bytes32(0);
        ruleCodeHash[target][selector] = hash;
        ++policyVersion;
        emit RuleChanged(target, selector, hash, policyVersion);
    }

    function setPaused(bool status) external onlyOwnerUnlocked {
        paused = status;
        ++policyVersion;
        emit PauseChanged(status, policyVersion);
    }

    function actionId(Action calldata a) public pure returns (bytes32) {
        return keccak256(abi.encode(ACTION_DOMAIN, a));
    }

    function _check(Action calldata a) private view {
        bytes32 codeHash = ruleCodeHash[a.target][a.selector];
        if (paused || policyHash == bytes32(0) || a.policyHash != policyHash ||
            a.policyVersion != policyVersion || a.chainId != block.chainid ||
            a.executor != address(this) || a.target == address(this) ||
            a.agent == address(0) || block.timestamp >= a.expiry ||
            usedNonce[a.nonce] || codeHash == bytes32(0) ||
            a.target.code.length == 0 || a.target.codehash != codeHash) revert Denied();
    }

    function approve(Action calldata a) external onlyOwnerUnlocked {
        _check(a);
        bytes32 id = actionId(a);
        approved[id] = true;
        emit ActionApproved(id, a.agent, a.nonce);
    }

    function revoke(bytes32 id) external onlyOwnerUnlocked {
        delete approved[id];
        emit ActionRevoked(id);
    }

    function execute(Action calldata a, bytes calldata data) external payable returns (bytes memory result) {
        if (entered) revert Reentrant();
        _check(a);
        bytes32 id = actionId(a);
        if (!approved[id] || msg.sender != a.agent || msg.value != a.value ||
            data.length < 4 || bytes4(data[:4]) != a.selector ||
            keccak256(data) != a.calldataHash) revert Denied();
        entered = true;
        usedNonce[a.nonce] = true;
        delete approved[id];
        (bool ok, bytes memory output) = a.target.call{value: a.value}(data);
        if (!ok) revert CallFailed();
        entered = false;
        emit ActionExecuted(id, a.agent, a.target, a.nonce);
        return output;
    }
}

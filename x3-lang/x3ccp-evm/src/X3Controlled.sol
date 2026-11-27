// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

/// @title X3Controlled — on-chain enforcement of an X3Lang `control` policy (X3CCP v1)
/// @notice A contract inherits this and marks external functions `x3Controlled`. Its owner
///         compiles a `control` block with x3-lang/x3ccp.py and installs the bytes with
///         `installPolicy`. Every marked call is then checked before the function body runs and
///         refused with `X3Refused(clause, code)`, where `clause` is the line of the `.x3` source
///         clause that refused.
/// @dev Authority boundary: this enforces rules only on the inheriting contract, whose owner
///      opted in. It cannot control an arbitrary immutable third-party contract.
///      Apply `x3Controlled` to EXTERNAL functions only: enforcement reads `msg.sig` and
///      `msg.data`, which describe the outer call, not an internal Solidity call.
///      Check order and codes are mirrored exactly by `predict()` in x3-lang/x3ccp.py.
abstract contract X3Controlled {
    // ---- refusal codes (identical to x3ccp.REFUSAL_CODES)
    uint8 internal constant NO_POLICY = 1;
    uint8 internal constant WRONG_CHAIN = 2;
    uint8 internal constant EXPIRED = 3;
    uint8 internal constant CALLER_NOT_ALLOWED = 4;
    uint8 internal constant FUNCTION_NOT_ALLOWED = 5;
    uint8 internal constant BAD_CALLDATA = 6;
    uint8 internal constant ABOVE_MAX = 7;
    uint8 internal constant BELOW_MIN = 8;
    uint8 internal constant NOT_EQUAL = 9;
    uint8 internal constant NOT_IN_SET = 10;
    uint8 internal constant RATE_SUM_EXCEEDED = 11;
    uint8 internal constant RATE_CALLS_EXCEEDED = 12;

    // ---- install rejection reasons
    uint8 internal constant BAD_MAGIC = 1;
    uint8 internal constant BAD_VERSION = 2;
    uint8 internal constant BAD_CHAIN = 3;
    uint8 internal constant BAD_TARGET = 4;
    uint8 internal constant BAD_OWNER = 5;
    uint8 internal constant BAD_EXPIRY = 6;
    uint8 internal constant BAD_COUNT = 7;
    uint8 internal constant DUPLICATE = 8;
    uint8 internal constant BAD_TYPE = 9;
    uint8 internal constant BAD_CHECK = 10;
    uint8 internal constant TRUNCATED = 11;
    uint8 internal constant TRAILING = 12;

    // ---- format constants (identical to x3ccp.py)
    uint256 internal constant MAX_CALLERS = 16;
    uint256 internal constant MAX_FUNCTIONS = 32;
    uint256 internal constant MAX_ARGS = 16;
    uint256 internal constant MAX_CHECKS = 16;
    uint256 internal constant MAX_SET = 16;
    uint256 internal constant MAX_WINDOW = 365 days;
    uint8 internal constant T_ADDRESS = 0x01;
    uint8 internal constant T_BOOL = 0x02;
    uint8 internal constant T_BYTES32 = 0x03;
    uint8 internal constant T_UINT_BASE = 0x10;
    uint8 internal constant K_MAX = 1;
    uint8 internal constant K_MIN = 2;
    uint8 internal constant K_EQ = 3;
    uint8 internal constant K_IN = 4;
    uint8 internal constant K_RATE_SUM = 5;
    uint8 internal constant K_RATE_CALLS = 6;
    uint8 internal constant NO_ARG = 0xFF;

    error X3Refused(uint16 clause, uint8 code);
    error X3NotOwner();
    error X3BadPolicy(uint8 reason);

    event X3PolicyInstalled(uint64 indexed epoch, bytes32 indexed policyHash, uint64 expiresAt);
    event X3PolicyRevoked(uint64 indexed epoch, bytes32 indexed policyHash);
    event X3Allowed(uint64 indexed epoch, bytes4 indexed selector, address indexed caller);

    struct Fn {
        bool allowed;
        uint16 line;
        uint8 nArgs;
        uint8 nChecks;
        bytes32 types; // byte i = type code of argument i
    }

    struct Check {
        uint8 kind;
        uint16 line;
        uint8 arg;
        uint32 window;
        uint256 value;
    }

    address public immutable x3Owner;

    uint64 public x3Epoch;
    bool public x3Active;
    bytes32 public x3PolicyHash;
    uint64 public x3ChainId;
    uint64 public x3ExpiresAt;
    uint16 internal _headerLine;
    uint16 internal _expiresLine;
    uint16 internal _callersLine;

    mapping(uint64 => mapping(address => bool)) internal _callerOk;
    mapping(uint64 => mapping(bytes4 => Fn)) internal _fns;
    mapping(uint64 => mapping(bytes4 => mapping(uint256 => Check))) internal _checks;
    mapping(uint64 => mapping(bytes4 => mapping(uint256 => mapping(uint256 => bool)))) internal _inSet;
    mapping(bytes32 => uint256) internal _usage;

    constructor(address owner_) {
        if (owner_ == address(0)) revert X3NotOwner();
        x3Owner = owner_;
    }

    modifier onlyX3Owner() {
        if (msg.sender != x3Owner) revert X3NotOwner();
        _;
    }

    modifier x3Controlled() {
        _x3Enforce();
        _;
    }

    // ------------------------------------------------------------------ owner

    /// @notice Install compiled X3CCP v1 policy bytes. Starts a new epoch: the previous policy and
    ///         all of its rate usage stop applying in the same transaction.
    function installPolicy(bytes calldata p) external onlyX3Owner {
        uint64 e = x3Epoch + 1;
        x3Epoch = e;
        uint256 pos = 0;
        if (_rd(p, pos, 4) != 0x58334350) revert X3BadPolicy(BAD_MAGIC); // "X3CP"
        pos += 4;
        if (_rd(p, pos, 1) != 1) revert X3BadPolicy(BAD_VERSION);
        pos += 1;
        uint64 chainId = uint64(_rd(p, pos, 8));
        pos += 8;
        if (chainId != block.chainid) revert X3BadPolicy(BAD_CHAIN);
        if (address(uint160(_rd(p, pos, 20))) != address(this)) revert X3BadPolicy(BAD_TARGET);
        pos += 20;
        if (address(uint160(_rd(p, pos, 20))) != x3Owner) revert X3BadPolicy(BAD_OWNER);
        pos += 20;
        uint64 expiresAt = uint64(_rd(p, pos, 8));
        pos += 8;
        if (expiresAt <= block.timestamp) revert X3BadPolicy(BAD_EXPIRY);
        x3ChainId = chainId;
        x3ExpiresAt = expiresAt;
        _headerLine = uint16(_rd(p, pos, 2));
        _expiresLine = uint16(_rd(p, pos + 2, 2));
        _callersLine = uint16(_rd(p, pos + 4, 2));
        pos += 6;

        uint256 nCallers = _rd(p, pos, 1);
        pos += 1;
        if (nCallers == 0 || nCallers > MAX_CALLERS) revert X3BadPolicy(BAD_COUNT);
        for (uint256 i = 0; i < nCallers; i++) {
            address c = address(uint160(_rd(p, pos, 20)));
            pos += 20;
            if (_callerOk[e][c]) revert X3BadPolicy(DUPLICATE);
            _callerOk[e][c] = true;
        }

        uint256 nFns = _rd(p, pos, 1);
        pos += 1;
        if (nFns == 0 || nFns > MAX_FUNCTIONS) revert X3BadPolicy(BAD_COUNT);
        for (uint256 i = 0; i < nFns; i++) {
            pos = _installFn(p, pos, e);
        }
        if (pos != p.length) revert X3BadPolicy(TRAILING);

        bytes32 h = keccak256(p);
        x3PolicyHash = h;
        x3Active = true;
        emit X3PolicyInstalled(e, h, expiresAt);
    }

    /// @notice Stop the installed policy. Every controlled call is refused until a new install.
    function revokePolicy() external onlyX3Owner {
        x3Active = false;
        emit X3PolicyRevoked(x3Epoch, x3PolicyHash);
    }

    function _installFn(bytes calldata p, uint256 pos, uint64 e) private returns (uint256) {
        bytes4 sel = bytes4(uint32(_rd(p, pos, 4)));
        pos += 4;
        Fn storage f = _fns[e][sel];
        if (f.allowed) revert X3BadPolicy(DUPLICATE);
        f.allowed = true;
        f.line = uint16(_rd(p, pos, 2));
        pos += 2;
        uint256 nArgs = _rd(p, pos, 1);
        pos += 1;
        if (nArgs > MAX_ARGS) revert X3BadPolicy(BAD_COUNT);
        // forge-lint: disable-next-line(unsafe-typecast) -- nArgs <= MAX_ARGS (16), checked above
        f.nArgs = uint8(nArgs);
        bytes32 types;
        for (uint256 a = 0; a < nArgs; a++) {
            uint256 t = _rd(p, pos, 1);
            pos += 1;
            if (!(t == T_ADDRESS || t == T_BOOL || t == T_BYTES32 || (t >= T_UINT_BASE && t <= T_UINT_BASE + 31))) {
                revert X3BadPolicy(BAD_TYPE);
            }
            types |= bytes32(t << (248 - 8 * a));
        }
        f.types = types;
        uint256 nChecks = _rd(p, pos, 1);
        pos += 1;
        if (nChecks > MAX_CHECKS) revert X3BadPolicy(BAD_COUNT);
        // forge-lint: disable-next-line(unsafe-typecast) -- nChecks <= MAX_CHECKS (16), checked above
        f.nChecks = uint8(nChecks);
        for (uint256 c = 0; c < nChecks; c++) {
            pos = _installCheck(p, pos, e, sel, c, nArgs, types);
        }
        return pos;
    }

    function _installCheck(
        bytes calldata p,
        uint256 pos,
        uint64 e,
        bytes4 sel,
        uint256 idx,
        uint256 nArgs,
        bytes32 types
    ) private returns (uint256) {
        Check storage c = _checks[e][sel][idx];
        uint8 kind = uint8(_rd(p, pos, 1));
        c.kind = kind;
        c.line = uint16(_rd(p, pos + 1, 2));
        uint8 arg = uint8(_rd(p, pos + 3, 1));
        c.arg = arg;
        pos += 4;
        if (kind == K_RATE_CALLS) {
            if (arg != NO_ARG) revert X3BadPolicy(BAD_CHECK);
        } else if (arg >= nArgs) {
            revert X3BadPolicy(BAD_CHECK);
        }
        if (kind == K_MAX || kind == K_MIN || kind == K_EQ) {
            c.value = _rd(p, pos, 32);
            pos += 32;
        } else if (kind == K_IN) {
            uint256 n = _rd(p, pos, 1);
            pos += 1;
            if (n == 0 || n > MAX_SET) revert X3BadPolicy(BAD_COUNT);
            for (uint256 i = 0; i < n; i++) {
                uint256 w = _rd(p, pos, 32);
                pos += 32;
                if (_inSet[e][sel][idx][w]) revert X3BadPolicy(DUPLICATE);
                _inSet[e][sel][idx][w] = true;
            }
        } else if (kind == K_RATE_SUM || kind == K_RATE_CALLS) {
            uint256 window = _rd(p, pos, 4);
            // forge-lint: disable-next-line(unsafe-typecast) -- read from 4 bytes, so it fits in uint32
            c.window = uint32(window);
            c.value = _rd(p, pos + 4, 32);
            pos += 36;
            if (window == 0 || window > MAX_WINDOW || c.value == 0) revert X3BadPolicy(BAD_CHECK);
        } else {
            revert X3BadPolicy(BAD_CHECK);
        }
        if (kind == K_MAX || kind == K_MIN || kind == K_RATE_SUM) {
            if (uint8(types[arg]) < T_UINT_BASE) revert X3BadPolicy(BAD_CHECK);
        }
        return pos;
    }

    /// @dev Read `n` (1..32) big-endian bytes at `pos`, refusing to read past the end.
    function _rd(bytes calldata p, uint256 pos, uint256 n) private pure returns (uint256 v) {
        if (pos + n > p.length) revert X3BadPolicy(TRUNCATED);
        assembly {
            v := shr(sub(256, mul(n, 8)), calldataload(add(p.offset, pos)))
        }
    }

    // ------------------------------------------------------------------ enforcement

    function _x3Enforce() internal {
        uint64 e = x3Epoch;
        if (!x3Active) revert X3Refused(0, NO_POLICY);
        if (block.chainid != x3ChainId) revert X3Refused(_headerLine, WRONG_CHAIN);
        if (block.timestamp >= x3ExpiresAt) revert X3Refused(_expiresLine, EXPIRED);
        if (!_callerOk[e][msg.sender]) revert X3Refused(_callersLine, CALLER_NOT_ALLOWED);
        if (msg.data.length < 4) revert X3Refused(_headerLine, FUNCTION_NOT_ALLOWED);
        bytes4 sel = msg.sig;
        Fn storage f = _fns[e][sel];
        if (!f.allowed) revert X3Refused(_headerLine, FUNCTION_NOT_ALLOWED);
        uint256 nArgs = f.nArgs;
        if (msg.data.length != 4 + 32 * nArgs) revert X3Refused(f.line, BAD_CALLDATA);
        bytes32 types = f.types;
        for (uint256 a = 0; a < nArgs; a++) {
            if (!_wordOk(_arg(a), uint8(types[a]))) revert X3Refused(f.line, BAD_CALLDATA);
        }
        uint256 nChecks = f.nChecks;
        for (uint256 i = 0; i < nChecks; i++) {
            Check storage c = _checks[e][sel][i];
            uint8 kind = c.kind;
            uint256 w = c.arg == NO_ARG ? 0 : _arg(c.arg);
            if (kind == K_MAX) {
                if (w > c.value) revert X3Refused(c.line, ABOVE_MAX);
            } else if (kind == K_MIN) {
                if (w < c.value) revert X3Refused(c.line, BELOW_MIN);
            } else if (kind == K_EQ) {
                if (w != c.value) revert X3Refused(c.line, NOT_EQUAL);
            } else if (kind == K_IN) {
                if (!_inSet[e][sel][i][w]) revert X3Refused(c.line, NOT_IN_SET);
            } else {
                bytes32 key = keccak256(abi.encode(e, sel, i, block.timestamp / c.window));
                uint256 used = _usage[key];
                uint256 add = kind == K_RATE_SUM ? w : 1;
                // used <= c.value always holds, so this cannot underflow and the sum cannot overflow.
                if (add > c.value - used) {
                    revert X3Refused(c.line, kind == K_RATE_SUM ? RATE_SUM_EXCEEDED : RATE_CALLS_EXCEEDED);
                }
                _usage[key] = used + add;
            }
        }
        emit X3Allowed(e, sel, msg.sender);
    }

    function _arg(uint256 i) private pure returns (uint256 w) {
        assembly {
            w := calldataload(add(4, mul(i, 32)))
        }
    }

    function _wordOk(uint256 w, uint8 t) private pure returns (bool) {
        if (t == T_ADDRESS) return w >> 160 == 0;
        if (t == T_BOOL) return w <= 1;
        if (t == T_BYTES32) return true;
        uint256 bits = (uint256(t) - T_UINT_BASE + 1) * 8;
        return bits == 256 || w >> bits == 0;
    }

    // ------------------------------------------------------------------ views

    /// @notice Rate usage of check `index` of `selector` in the current epoch's window at `timestamp`.
    function x3Usage(bytes4 selector, uint256 index, uint256 timestamp) external view returns (uint256) {
        Check storage c = _checks[x3Epoch][selector][index];
        if (c.window == 0) return 0;
        return _usage[keccak256(abi.encode(x3Epoch, selector, index, timestamp / c.window))];
    }

    function x3IsCaller(address who) external view returns (bool) {
        return _callerOk[x3Epoch][who];
    }
}

// SPDX-License-Identifier: MIT
pragma solidity 0.8.19;

/// @dev Test-only tokens and oracle. Never deployed outside the local test chain.

contract MockERC20 {
    string public name;
    string public symbol;
    uint8 public immutable decimals;
    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    constructor(string memory name_, string memory symbol_, uint8 decimals_) {
        name = name_;
        symbol = symbol_;
        decimals = decimals_;
    }

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
        totalSupply += amount;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        return true;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        uint256 allowed = allowance[from][msg.sender];
        if (allowed != type(uint256).max) allowance[from][msg.sender] = allowed - amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        return true;
    }
}

/// @dev USDT-style token: no return value on transfer/approve, and refuses non-zero to non-zero approvals.
contract MockNoReturnToken {
    uint8 public constant decimals = 6;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
    }

    function approve(address spender, uint256 amount) external {
        require(amount == 0 || allowance[msg.sender][spender] == 0, "USDT approve");
        allowance[msg.sender][spender] = amount;
    }

    function transfer(address to, uint256 amount) external {
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
    }

    function transferFrom(address from, address to, uint256 amount) external {
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
    }
}

interface IMintable {
    function balanceOf(address) external view returns (uint256);
    function transfer(address, uint256) external;
}

/// @dev ERC4626-shaped share with a settable rate and a settable cash buffer.
/// redeem pays from the vault's own balance of the asset and reverts when it runs short,
/// like a vault whose liquidity is deployed elsewhere.
contract MockVault {
    address public immutable asset;
    uint8 public constant decimals = 18;
    uint256 public totalSupply;
    /// @dev Asset units paid per 1e18 share units.
    uint256 public assetsPerShare;
    /// @dev When set, redeem queues instead of paying (an async vault): returns 0 and keeps the shares.
    bool public asyncRedeem;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    constructor(address asset_, uint256 assetsPerShare_) {
        asset = asset_;
        assetsPerShare = assetsPerShare_;
    }

    function setRate(uint256 assetsPerShare_) external {
        assetsPerShare = assetsPerShare_;
    }

    function setAsync(bool on) external {
        asyncRedeem = on;
    }

    function mintShares(address to, uint256 shares) external {
        balanceOf[to] += shares;
        totalSupply += shares;
    }

    function convertToAssets(uint256 shares) public view returns (uint256) {
        return (shares * assetsPerShare) / 1e18;
    }

    function maxWithdraw(address owner) external view returns (uint256) {
        uint256 value = convertToAssets(balanceOf[owner]);
        uint256 cash = IMintable(asset).balanceOf(address(this));
        return value < cash ? value : cash;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        return true;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        uint256 allowed = allowance[from][msg.sender];
        if (allowed != type(uint256).max) allowance[from][msg.sender] = allowed - amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        return true;
    }

    function redeem(uint256 shares, address receiver, address owner) external returns (uint256 assets) {
        require(msg.sender == owner, "owner only");
        if (asyncRedeem) return 0;
        assets = convertToAssets(shares);
        require(IMintable(asset).balanceOf(address(this)) >= assets, "illiquid");
        balanceOf[owner] -= shares;
        totalSupply -= shares;
        IMintable(asset).transfer(receiver, assets);
    }
}

contract MockOracle {
    uint256 public price;

    function setPrice(uint256 price_) external {
        price = price_;
    }
}

interface IERC20Min {
    function transfer(address, uint256) external returns (bool);
    function transferFrom(address, address, uint256) external returns (bool);
}

/// @dev A one-pool DEX with a settable price. `skim` lets a test play a malicious router call.
contract MockSwapRouter {
    /// @dev Output units per input unit, WAD-scaled, raw token units (decimals already folded in).
    mapping(address => mapping(address => uint256)) public outPerInWad;

    function setPrice(address tokenIn, address tokenOut, uint256 outPerInWad_) external {
        outPerInWad[tokenIn][tokenOut] = outPerInWad_;
    }

    function swapExactIn(address tokenIn, address tokenOut, uint256 amountIn, uint256 minOut, address recipient)
        public
        returns (uint256 out)
    {
        require(IERC20Min(tokenIn).transferFrom(msg.sender, address(this), amountIn), "pull failed");
        out = (amountIn * outPerInWad[tokenIn][tokenOut]) / 1e18;
        require(out >= minOut, "too little received");
        require(IERC20Min(tokenOut).transfer(recipient, out), "pay failed");
    }

    /// @dev Pulls `amount` of the caller's approved tokens and gives them to `to`.
    function skim(address token, uint256 amount, address to) public {
        require(IERC20Min(token).transferFrom(msg.sender, to, amount), "skim failed");
    }

    function multicall(bytes[] calldata calls) external {
        for (uint256 i = 0; i < calls.length; i++) {
            (bool ok, bytes memory ret) = address(this).delegatecall(calls[i]);
            if (!ok) {
                assembly {
                    revert(add(ret, 32), mload(ret))
                }
            }
        }
    }
}

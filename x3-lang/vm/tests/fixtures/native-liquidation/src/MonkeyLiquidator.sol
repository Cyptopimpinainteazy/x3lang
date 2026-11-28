// SPDX-License-Identifier: MIT
pragma solidity 0.8.19;

/// @dev Morpho Blue's market key. Field order must match Morpho exactly.
struct MarketParams {
    address loanToken;
    address collateralToken;
    address oracle;
    address irm;
    uint256 lltv;
}

interface IMorphoLiquidate {
    function liquidate(
        MarketParams memory marketParams,
        address borrower,
        uint256 seizedAssets,
        uint256 repaidShares,
        bytes memory data
    ) external returns (uint256, uint256);
}

interface IERC20Like {
    function balanceOf(address account) external view returns (uint256);
}

interface IERC4626Like {
    function asset() external view returns (address);
    function redeem(uint256 shares, address receiver, address owner) external returns (uint256 assets);
}

/// @title MonkeyLiquidator
/// @notice Liquidates a Morpho Blue position whose collateral is an ERC4626 share of the loan token,
/// redeems the seized shares inside Morpho's callback, repays, and sends what is left to the owner.
/// No flashloan: Morpho hands over the collateral before it pulls the repay.
/// @dev Fails closed. If the vault cannot pay, or pays less than repay + minProfit, the whole
/// transaction reverts and the only cost is gas.
contract MonkeyLiquidator {
    address public immutable MORPHO;

    /// @notice Receives every profit and every sweep. Should be a cold wallet.
    address public owner;
    /// @notice Hot key the bot signs with. Can trigger liquidations, cannot move funds anywhere but the owner.
    address public keeper;

    /// @dev Non-zero only between liquidate() calling Morpho and Morpho calling back.
    address private _activeVault;
    uint256 private _sharesBefore;

    error NotOwner();
    error NotAuthorized();
    error NotMorpho();
    error NoActiveLiquidation();
    error NotSameAsset(address vaultAsset, address loanToken);
    error RedeemShort(uint256 redeemed, uint256 needed);
    error TokenCallFailed(address token);
    error ZeroAddress();

    event Liquidated(
        address indexed borrower,
        address indexed collateralToken,
        address indexed loanToken,
        uint256 seized,
        uint256 repaid,
        uint256 profit
    );
    event OwnerChanged(address indexed owner);
    event KeeperChanged(address indexed keeper);

    modifier onlyOwner() {
        if (msg.sender != owner) revert NotOwner();
        _;
    }

    constructor(address morpho, address owner_, address keeper_) {
        if (morpho == address(0) || owner_ == address(0)) revert ZeroAddress();
        MORPHO = morpho;
        owner = owner_;
        keeper = keeper_;
        emit OwnerChanged(owner_);
        emit KeeperChanged(keeper_);
    }

    /// @notice Liquidate `borrower` on `marketParams`.
    /// @param seizedAssets Collateral to seize, or 0 when passing `repaidShares`. Exactly one must be non-zero.
    /// @param repaidShares Borrow shares to repay, or 0 when passing `seizedAssets`.
    /// @param minProfit Loan-token units that must remain after the repay, or the call reverts.
    /// @return profit Loan-token units sent to the owner.
    function liquidate(
        MarketParams calldata marketParams,
        address borrower,
        uint256 seizedAssets,
        uint256 repaidShares,
        uint256 minProfit
    ) external returns (uint256 profit) {
        if (msg.sender != owner && msg.sender != keeper) revert NotAuthorized();

        address vault = marketParams.collateralToken;
        address loan = marketParams.loanToken;
        address vaultAsset = IERC4626Like(vault).asset();
        if (vaultAsset != loan) revert NotSameAsset(vaultAsset, loan);

        uint256 loanBefore = IERC20Like(loan).balanceOf(address(this));
        _activeVault = vault;
        _sharesBefore = IERC20Like(vault).balanceOf(address(this));

        (uint256 seized, uint256 repaid) = IMorphoLiquidate(MORPHO).liquidate(
            marketParams, borrower, seizedAssets, repaidShares, abi.encode(loan, minProfit)
        );

        _activeVault = address(0);
        _sharesBefore = 0;

        profit = IERC20Like(loan).balanceOf(address(this)) - loanBefore;
        if (profit > 0) _transfer(loan, owner, profit);
        emit Liquidated(borrower, vault, loan, seized, repaid, profit);
    }

    /// @notice Morpho's liquidation callback. The seized shares are already here; the repay is pulled after this returns.
    function onMorphoLiquidate(uint256 repaidAssets, bytes calldata data) external {
        if (msg.sender != MORPHO) revert NotMorpho();
        address vault = _activeVault;
        if (vault == address(0)) revert NoActiveLiquidation();

        (address loan, uint256 minProfit) = abi.decode(data, (address, uint256));
        uint256 shares = IERC20Like(vault).balanceOf(address(this)) - _sharesBefore;
        uint256 redeemed = IERC4626Like(vault).redeem(shares, address(this), address(this));

        uint256 needed = repaidAssets + minProfit;
        if (redeemed < needed) revert RedeemShort(redeemed, needed);

        _approve(loan, MORPHO, repaidAssets);
    }

    function setKeeper(address keeper_) external onlyOwner {
        keeper = keeper_;
        emit KeeperChanged(keeper_);
    }

    function setOwner(address owner_) external onlyOwner {
        if (owner_ == address(0)) revert ZeroAddress();
        owner = owner_;
        emit OwnerChanged(owner_);
    }

    /// @notice Send any token balance this contract holds to the owner.
    function sweep(address token) external onlyOwner {
        uint256 balance = IERC20Like(token).balanceOf(address(this));
        if (balance > 0) _transfer(token, owner, balance);
    }

    /// @dev Works with tokens that return nothing (USDT) and tokens that return bool.
    function _call(address token, bytes memory payload) private returns (bool) {
        (bool ok, bytes memory ret) = token.call(payload);
        return ok && (ret.length == 0 || abi.decode(ret, (bool))) && token.code.length > 0;
    }

    function _transfer(address token, address to, uint256 amount) private {
        if (!_call(token, abi.encodeWithSelector(0xa9059cbb, to, amount))) revert TokenCallFailed(token);
    }

    /// @dev Approve exactly `amount`, resetting to zero first for tokens that refuse a non-zero to non-zero change.
    function _approve(address token, address spender, uint256 amount) private {
        if (_call(token, abi.encodeWithSelector(0x095ea7b3, spender, amount))) return;
        if (!_call(token, abi.encodeWithSelector(0x095ea7b3, spender, 0))) revert TokenCallFailed(token);
        if (!_call(token, abi.encodeWithSelector(0x095ea7b3, spender, amount))) revert TokenCallFailed(token);
    }
}

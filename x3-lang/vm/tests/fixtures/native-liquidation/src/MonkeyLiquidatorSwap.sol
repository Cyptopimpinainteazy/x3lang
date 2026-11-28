// SPDX-License-Identifier: MIT
pragma solidity 0.8.19;

import {MarketParams, IMorphoLiquidate, IERC20Like} from "./MonkeyLiquidator.sol";

/// @title MonkeyLiquidatorSwap
/// @notice Liquidates a Morpho Blue position and sells the seized collateral for the loan token on a
/// DEX inside Morpho's callback, then repays and sends what is left to the owner. No flashloan:
/// Morpho hands over the collateral before it pulls the repay.
/// @dev Fails closed. The transaction reverts unless the swap brings in at least
/// repay + max(minProfit, repay × minProfitBps). A lost race or a bad quote costs only gas.
///
/// Trust model. The keeper passes the router call data, so a stolen keeper key could route part of
/// the seized collateral to itself. It can never touch the owner's funds or anything the contract
/// held before the liquidation, and the owner-set `minProfitBps` floor guarantees the owner keeps at
/// least that share of every repay. Routers must be allowlisted by the owner.
contract MonkeyLiquidatorSwap {
    address public immutable MORPHO;

    /// @notice Receives every profit and every sweep. Should be a cold wallet.
    address public owner;
    /// @notice Hot key the bot signs with. Triggers liquidations; cannot withdraw.
    address public keeper;
    /// @notice Minimum profit as a share of the repay, in basis points, enforced on every liquidation.
    uint256 public minProfitBps;
    /// @notice Routers the keeper may send swap call data to.
    mapping(address => bool) public isRouter;

    /// @dev Set only between liquidate() calling Morpho and Morpho calling back.
    address private _activeCollateral;
    uint256 private _collateralBefore;
    uint256 private _loanBefore;

    uint256 private constant MAX_MIN_PROFIT_BPS = 5_000;

    error NotOwner();
    error NotAuthorized();
    error NotMorpho();
    error NoActiveLiquidation();
    error RouterNotAllowed(address router);
    error SwapFailed(bytes reason);
    error SwapShort(uint256 received, uint256 needed);
    error CollateralLeft(uint256 left);
    error TokenCallFailed(address token);
    error ZeroAddress();
    error BadBps(uint256 bps);

    event Liquidated(
        address indexed borrower,
        address indexed collateralToken,
        address indexed loanToken,
        uint256 seized,
        uint256 repaid,
        uint256 profit,
        address router
    );
    event OwnerChanged(address indexed owner);
    event KeeperChanged(address indexed keeper);
    event RouterSet(address indexed router, bool allowed);
    event MinProfitBpsSet(uint256 bps);

    modifier onlyOwner() {
        if (msg.sender != owner) revert NotOwner();
        _;
    }

    constructor(address morpho, address owner_, address keeper_, uint256 minProfitBps_) {
        if (morpho == address(0) || owner_ == address(0)) revert ZeroAddress();
        if (minProfitBps_ > MAX_MIN_PROFIT_BPS) revert BadBps(minProfitBps_);
        MORPHO = morpho;
        owner = owner_;
        keeper = keeper_;
        minProfitBps = minProfitBps_;
        emit OwnerChanged(owner_);
        emit KeeperChanged(keeper_);
        emit MinProfitBpsSet(minProfitBps_);
    }

    /// @param borrower Position to liquidate.
    /// @param seizedAssets Collateral to seize, or 0 when passing `repaidShares`. Exactly one must be non-zero.
    ///        Prefer seizedAssets: the amount sold in `swapData` must equal the seize, or the call reverts.
    /// @param repaidShares Borrow shares to repay, or 0 when passing `seizedAssets`.
    /// @param minProfit Loan-token units that must remain after the repay. The owner's bps floor also applies.
    /// @param router An owner-allowlisted router, approved for exactly the seized collateral and then reset to zero.
    /// @param swapData Call data for `router` that sells the seized collateral and pays the loan token to this contract.
    struct Order {
        address borrower;
        uint256 seizedAssets;
        uint256 repaidShares;
        uint256 minProfit;
        address router;
        bytes swapData;
    }

    /// @notice Liquidate `order.borrower` on `marketParams` and sell the seized collateral through `order.router`.
    /// @return profit Loan-token units sent to the owner.
    function liquidate(MarketParams calldata marketParams, Order calldata order) external returns (uint256 profit) {
        if (msg.sender != owner && msg.sender != keeper) revert NotAuthorized();
        if (!isRouter[order.router]) revert RouterNotAllowed(order.router);

        _begin(marketParams.collateralToken, marketParams.loanToken);
        (uint256 seized, uint256 repaid) = IMorphoLiquidate(MORPHO).liquidate(
            marketParams,
            order.borrower,
            order.seizedAssets,
            order.repaidShares,
            abi.encode(marketParams.loanToken, order.minProfit, order.router, order.swapData)
        );
        profit = _finish(marketParams.loanToken);
        emit Liquidated(
            order.borrower, marketParams.collateralToken, marketParams.loanToken, seized, repaid, profit, order.router
        );
    }

    function _begin(address collateral, address loan) private {
        _activeCollateral = collateral;
        _collateralBefore = IERC20Like(collateral).balanceOf(address(this));
        _loanBefore = IERC20Like(loan).balanceOf(address(this));
    }

    /// @dev Clear the in-flight state and pay the owner exactly what this liquidation made.
    function _finish(address loan) private returns (uint256 profit) {
        uint256 loanBefore = _loanBefore;
        _activeCollateral = address(0);
        _collateralBefore = 0;
        _loanBefore = 0;
        profit = IERC20Like(loan).balanceOf(address(this)) - loanBefore;
        if (profit > 0) _transfer(loan, owner, profit);
    }

    /// @notice Morpho's liquidation callback. The seized collateral is here; the repay is pulled after this returns.
    function onMorphoLiquidate(uint256 repaidAssets, bytes calldata data) external {
        if (msg.sender != MORPHO) revert NotMorpho();
        address collateral = _activeCollateral;
        if (collateral == address(0)) revert NoActiveLiquidation();

        (address loan, uint256 minProfit, address router, bytes memory swapData) =
            abi.decode(data, (address, uint256, address, bytes));
        _sell(collateral, router, swapData);
        _settle(loan, repaidAssets, minProfit);
    }

    /// @dev Approve the router for exactly the seize, run its call, reset the approval, and require every
    /// seized unit to be gone: a leftover means the call data did something other than sell the seize.
    function _sell(address collateral, address router, bytes memory swapData) private {
        uint256 before = _collateralBefore;
        uint256 seized = IERC20Like(collateral).balanceOf(address(this)) - before;
        _approve(collateral, router, seized);
        (bool ok, bytes memory ret) = router.call(swapData);
        if (!ok) revert SwapFailed(ret);
        _approve(collateral, router, 0);
        uint256 left = IERC20Like(collateral).balanceOf(address(this));
        if (left > before) revert CollateralLeft(left - before);
    }

    /// @dev The swap must have brought in repay + max(minProfit, repay × minProfitBps), then approve the repay.
    function _settle(address loan, uint256 repaidAssets, uint256 minProfit) private {
        uint256 floor = (repaidAssets * minProfitBps) / 10_000;
        uint256 needed = repaidAssets + (minProfit > floor ? minProfit : floor);
        uint256 received = IERC20Like(loan).balanceOf(address(this)) - _loanBefore;
        if (received < needed) revert SwapShort(received, needed);
        _approve(loan, MORPHO, repaidAssets);
    }

    function setRouter(address router, bool allowed) external onlyOwner {
        if (router == address(0)) revert ZeroAddress();
        isRouter[router] = allowed;
        emit RouterSet(router, allowed);
    }

    function setMinProfitBps(uint256 bps) external onlyOwner {
        if (bps > MAX_MIN_PROFIT_BPS) revert BadBps(bps);
        minProfitBps = bps;
        emit MinProfitBpsSet(bps);
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

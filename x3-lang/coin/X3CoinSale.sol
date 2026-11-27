// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// The canonical generated X3IssuedCoin exposes these two methods.
interface IX3SaleCoin {
    function declarationHash() external view returns (bytes32);
    function transfer(address recipient, uint256 amount) external returns (bool);
}

/// A steward admits priced orders for a single prefunded X3 coin on this chain.
contract X3CoinSale {
    IX3SaleCoin public immutable coin;
    address payable public immutable treasury;
    address public immutable steward;
    bytes32 public immutable declarationHash;

    struct Order {
        address buyer;
        uint64 amount;
        uint64 expiresAt;
        uint256 priceWei;
        bool filled;
    }

    mapping(bytes32 => Order) public orders;

    error InvalidConfiguration();
    error Unauthorized();
    error InvalidOrder();
    error OrderExists();
    error UnknownOrder();
    error AlreadyFilled();
    error OrderExpired();
    error WrongPrice();
    error DeliveryFailed();
    error PaymentFailed();

    event SaleAuthorized(bytes32 indexed orderKey, address indexed buyer, uint64 amount, uint256 priceWei, uint64 expiresAt);
    event SaleFilled(bytes32 indexed orderKey, address indexed buyer, address indexed token, uint256 amount, uint256 paymentWei);

    constructor(address token, address payable payee, bytes32 expectedDeclaration) {
        if (token.code.length == 0 || payee == address(0) || payee == address(this)
            || payee == token || expectedDeclaration == bytes32(0)) revert InvalidConfiguration();
        coin = IX3SaleCoin(token);
        if (coin.declarationHash() != expectedDeclaration) revert InvalidConfiguration();
        declarationHash = expectedDeclaration;
        steward = msg.sender;
        treasury = payee;
    }

    function authorize(bytes32 orderKey, address buyer, uint64 amount, uint256 priceWei, uint64 expiresAt) external {
        if (msg.sender != steward) revert Unauthorized();
        if (orders[orderKey].buyer != address(0)) revert OrderExists();
        if (orderKey == bytes32(0) || buyer == address(0) || buyer == address(this)
            || buyer == address(coin) || buyer == treasury || amount == 0 || priceWei == 0
            || expiresAt <= block.timestamp) revert InvalidOrder();
        orders[orderKey] = Order(buyer, amount, expiresAt, priceWei, false);
        emit SaleAuthorized(orderKey, buyer, amount, priceWei, expiresAt);
    }

    function isFilled(bytes32 orderKey) external view returns (bool) {
        return orders[orderKey].filled;
    }

    function buy(bytes32 orderKey) external payable {
        Order storage order = orders[orderKey];
        if (order.buyer == address(0)) revert UnknownOrder();
        if (order.filled) revert AlreadyFilled();
        if (msg.sender != order.buyer) revert Unauthorized();
        if (block.timestamp >= order.expiresAt) revert OrderExpired();
        if (msg.value != order.priceWei) revert WrongPrice();

        order.filled = true;
        if (!coin.transfer(msg.sender, order.amount)) revert DeliveryFailed();
        (bool paid,) = treasury.call{value: msg.value}("");
        if (!paid) revert PaymentFailed();
        emit SaleFilled(orderKey, msg.sender, address(coin), order.amount, msg.value);
    }
}

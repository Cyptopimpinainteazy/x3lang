"""Emit a standalone fixed-issuance ERC-20 from a validated coin declaration."""

from pathlib import Path


def render_evm(coin: dict) -> str:
    """The constructor pins the chain and CREATE address before issuing any units."""
    token_library = Path(__file__).with_name("ERC20-5.4.0.sol").read_text(encoding="utf-8")
    target = coin["evm"]
    # Hex literals avoid Solidity source interpolation of user-supplied metadata.
    name = coin["name"].encode("utf-8").hex()
    symbol = coin["symbol"].encode("utf-8").hex()
    address = target["contract"][2:].lower()
    recipient = target["recipient"][2:].lower()
    return token_library + f'''
/// @dev The only issuance is the declared local allocation at construction.
contract X3IssuedCoin is ERC20 {{
    bytes32 public constant declarationHash = hex"{coin["declaration_hash"]}";

    error WrongDeploymentChain();
    error WrongDeploymentAddress();

    constructor() ERC20(string(hex"{name}"), string(hex"{symbol}")) {{
        if (block.chainid != {target["chain_id"]}) revert WrongDeploymentChain();
        if (address(this) != address(bytes20(hex"{address}"))) revert WrongDeploymentAddress();
        _mint(address(bytes20(hex"{recipient}")), {target["allocation"]});
    }}

    function decimals() public pure override returns (uint8) {{
        return {coin["decimals"]};
    }}
}}
'''

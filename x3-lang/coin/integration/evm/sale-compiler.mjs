import { readFileSync } from 'node:fs';
import { Interface } from 'ethers';
import solc from 'solc';

export function compileSaleContracts(tokenSolidity, extraSources = {}) {
  const sources = {
    ...extraSources,
    'Token.sol': { content: tokenSolidity },
    'Sale.sol': { content: readFileSync(new URL('../../X3CoinSale.sol', import.meta.url), 'utf8') },
  };
  const output = JSON.parse(solc.compile(JSON.stringify({
    language: 'Solidity', sources,
    settings: { evmVersion: 'shanghai', outputSelection: { '*': { '*': ['abi', 'evm.bytecode.object'] } } },
  })));
  const errors = (output.errors ?? []).filter((message) => message.severity === 'error');
  if (errors.length > 0) throw new Error(errors.map((m) => m.formattedMessage).join('\n'));
  return (file, name) => {
    const c = output.contracts[file][name];
    return { abi: new Interface(c.abi), bytecode: '0x' + c.evm.bytecode.object };
  };
}

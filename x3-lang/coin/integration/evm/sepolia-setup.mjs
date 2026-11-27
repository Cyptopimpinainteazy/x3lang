// Prepare public wallet requests offline. This process has no provider or signing path.
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Common, Hardfork } from '@ethereumjs/common';
import { Account, Address } from '@ethereumjs/util';
import { VM } from '@ethereumjs/vm';
import { ZeroAddress, concat, getAddress, getBytes, getCreateAddress, keccak256, toQuantity } from 'ethers';
import { compileSaleContracts } from './sale-compiler.mjs';

const chainId = 11155111;
const language = fileURLToPath(new URL('../../../', import.meta.url));
const quantity = (s, max, minimum = 1n) => {
  if (typeof s !== 'string' || !/^(0|[1-9][0-9]{0,77})$/.test(s) || BigInt(s) > max || BigInt(s) < minimum) {
    throw new Error('invalid unsigned decimal quantity');
  }
  return BigInt(s);
};
const address = (s) => {
  const a = getAddress(s).toLowerCase();
  if (a === ZeroAddress) throw new Error('expected a nonzero public EVM address');
  return a;
};

export async function prepareSetup(sourcePath, setup) {
  const expected = ['steward', 'treasury', 'steward_nonce', 'prefund', 'order_id', 'buyer', 'amount', 'expires_at', 'payment_wei'];
  if (Object.keys(setup).sort().join(',') !== expected.sort().join(',')) throw new Error('unexpected or missing setup fields');
  const steward = address(setup.steward), treasury = address(setup.treasury), buyer = address(setup.buyer);
  const max64 = (1n << 64n) - 1n;
  const nonce = quantity(setup.steward_nonce, max64 - 3n, 0n);
  const prefund = quantity(setup.prefund, max64);
  const amount = quantity(setup.amount, prefund);
  quantity(setup.payment_wei, (1n << 256n) - 1n);
  if (typeof setup.order_id !== 'string' || !/^[a-zA-Z0-9][a-zA-Z0-9:._-]{0,127}$/.test(setup.order_id)) {
    throw new Error('invalid order id');
  }
  if (!Number.isSafeInteger(setup.expires_at) || setup.expires_at <= Math.floor(Date.now() / 1000)) {
    throw new Error('expected a future Unix expiry in seconds');
  }
  const compiled = JSON.parse(execFileSync('python3', ['-m', 'coin', 'build', resolve(sourcePath)], { cwd: language }));
  const target = compiled.manifest.evm;
  if (target.chain_id !== String(chainId)) throw new Error('the .x3 declaration must select Ethereum Sepolia chain 11155111');
  const token = getCreateAddress({ from: steward, nonce }).toLowerCase();
  const sale = getCreateAddress({ from: steward, nonce: nonce + 1n }).toLowerCase();
  if (address(target.contract) !== token) throw new Error('declared token differs from the ordinary CREATE address of the steward at the specified nonce');
  const recipient = address(target.recipient);
  if (prefund > BigInt(target.allocation)) throw new Error('prefund exceeds declared local allocation');
  if (recipient === sale || [sale, token, treasury].includes(buyer) || [sale, token].includes(treasury)) {
    throw new Error('buyer, treasury and predicted deployments must be distinct; sale cannot be the genesis recipient');
  }
  const artifact = compileSaleContracts(compiled.evm_solidity);
  const tok = artifact('Token.sol', 'X3IssuedCoin'), controller = artifact('Sale.sol', 'X3CoinSale');
  const declaration = '0x' + compiled.declaration_hash;
  const saleInit = concat([controller.bytecode, controller.abi.encodeDeploy([token, treasury, declaration])]);
  const net = { chainId: toQuantity(chainId), value: '0x0' };
  const calls = {
    deploy_coin: { ...net, from: steward, nonce: toQuantity(nonce), data: tok.bytecode },
    deploy_sale: { ...net, from: steward, nonce: toQuantity(nonce + 1n), data: saleInit },
    prefund_sale: { ...net, from: recipient, to: token, data: tok.abi.encodeFunctionData('transfer', [sale, prefund]) },
  };
  if (recipient === steward) calls.prefund_sale.nonce = toQuantity(nonce + 2n);

  // Execute constructors with the real public arguments. Hashing solc's unlinked
  // runtime template would omit the constructor's stored immutable values.
  const vm = await VM.create({ common: Common.custom({ chainId }, { hardfork: Hardfork.Shanghai }) });
  const caller = Address.fromString(steward);
  await vm.stateManager.putAccount(caller, new Account(nonce, 0n));
  async function runtimeHash(call, expectedAddress) {
    const r = await vm.evm.runCall({ caller, data: getBytes(call.data) });
    if (r.execResult.exceptionError || r.createdAddress?.toString() !== expectedAddress) throw new Error('offline constructor failed');
    const code = await vm.stateManager.getContractCode(r.createdAddress);
    if (!code.length) throw new Error('empty runtime');
    return keccak256(code);
  }
  const tokenHash = await runtimeHash(calls.deploy_coin, token);
  const saleHash = await runtimeHash(calls.deploy_sale, sale);
  const genesis = execFileSync('python3', ['-c', 'from coin.delivery import SEPOLIA_GENESIS;print(SEPOLIA_GENESIS)'],
    { cwd: language, encoding: 'utf8' }).trim();
  return {
    status: 'unsigned_setup', asset_id: compiled.declaration_hash, source_sha256: compiled.source_sha256,
    wallet_calls: calls,
    // Use the public sale deployment receipt's block number to complete this policy.
    partial_policy: { chain_id: chainId, genesis_hash: genesis, sale_contract: sale, sale_code_hash: saleHash,
      token_contract: token, token_code_hash: tokenHash, destination: 'ethereum', treasury, payment_wei: setup.payment_wei },
    reservation: { asset_id: compiled.declaration_hash, order_id: setup.order_id, buyer, destination: 'ethereum',
      amount: amount.toString(), expires_at: setup.expires_at },
  };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length !== 4) throw new Error('usage: node sepolia-setup.mjs <coin.x3> <public-setup.json>');
    console.log(JSON.stringify(await prepareSetup(process.argv[2], JSON.parse(readFileSync(process.argv[3], 'utf8'))), null, 2));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}

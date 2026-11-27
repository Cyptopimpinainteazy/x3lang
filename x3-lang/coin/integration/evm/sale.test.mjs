import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { Block } from '@ethereumjs/block';
import { Common, Hardfork } from '@ethereumjs/common';
import { LegacyTransaction } from '@ethereumjs/tx';
import { Account, Address } from '@ethereumjs/util';
import { VM } from '@ethereumjs/vm';
import { Interface, concat, getBytes, getCreateAddress, hexlify, id, keccak256, toBeHex, toQuantity } from 'ethers';
import { compileSaleContracts } from './sale-compiler.mjs';
import { prepareSetup } from './sepolia-setup.mjs';

const language = fileURLToPath(new URL('../../../', import.meta.url));
const bookDriver = fileURLToPath(new URL('./sale_book.py', import.meta.url));
const chainId = 31337;
const token = '0xf2e246bb76df876cef8b38ae84130f4f55de395b';
const sale = '0x2946259e0334f33a064106302415ad3391bed384';
const firstTimestamp = 1_800_000_000;
const coins = (whole) => BigInt(whole) * 1_000_000n;
const price = 10_000_000_000_000_000n;

// Disposable fixture identities exist solely inside this process.
function actor(index) {
  const signer = getBytes(toBeHex(index, 32));
  return { signer, address: Address.fromPrivateKey(signer).toString() };
}
const owner = actor(1);
const buyer = actor(2);
const treasury = actor(3);

function buildArtifacts() {
  const python = `
import json
from pathlib import Path
from coin import author_coin, compile_coin
spec = json.loads(Path('coin/examples/dual_vm.json').read_text())
spec['cap'] = '1000000000000'
spec['evm']['allocation'] = '100000000000'
spec['svm']['allocation'] = '900000000000'
bundle = compile_coin(author_coin(spec))
print(json.dumps({k: bundle[k] for k in ('evm_solidity', 'declaration_hash')}))
`;
  const bundle = JSON.parse(execFileSync('python3', ['-c', python], { cwd: language }));
  const artifact = compileSaleContracts(bundle.evm_solidity, {
    'Refusal.sol': { content: 'pragma solidity 0.8.28; contract Refusal { receive() external payable { revert(); } }' },
  });
  return {
    token: artifact('Token.sol', 'X3IssuedCoin'), sale: artifact('Sale.sol', 'X3CoinSale'),
    refusal: artifact('Refusal.sol', 'Refusal'), declaration: '0x' + bundle.declaration_hash,
  };
}
const artifacts = buildArtifacts();

const blockWire = (block) => ({
  hash: hexlify(block.hash()), parentHash: hexlify(block.header.parentHash),
  number: toQuantity(block.header.number), timestamp: toQuantity(block.header.timestamp),
  stateRoot: hexlify(block.header.stateRoot), receiptsRoot: hexlify(block.header.receiptTrie),
  transactionsRoot: hexlify(block.header.transactionsTrie), transactions: block.transactions.map((tx) => hexlify(tx.hash())),
});

// Each real signed transaction is executed in its own locally built Ethereum block.
// The RPC transcript labels the head finalized for a deterministic observer test.
class LocalChain {
  static async start(netId = chainId, ownerNonce = 0n) {
    const chain = new LocalChain();
    chain.common = Common.custom({ chainId: netId }, { hardfork: Hardfork.Shanghai });
    chain.vm = await VM.create({ common: chain.common });
    for (const { address } of [owner, buyer, treasury]) {
      await chain.vm.stateManager.putAccount(Address.fromString(address), new Account(address === owner.address ? ownerNonce : 0n, 100n * 10n ** 18n));
    }
    chain.tip = Block.fromBlockData({ header: {
      number: 0n, timestamp: BigInt(firstTimestamp), gasLimit: 12_000_000n, baseFeePerGas: 1n,
      stateRoot: await chain.vm.stateManager.getStateRoot(),
    } }, { common: chain.common });
    chain.blocks = [blockWire(chain.tip)];
    chain.transactions = [];
    chain.receipts = [];
    return chain;
  }

  async send(sender, to, input, value = 0n, time = this.tip.header.timestamp + 1n) {
    const account = await this.vm.stateManager.getAccount(Address.fromString(sender.address));
    const tx = LegacyTransaction.fromTxData({
      nonce: account.nonce, gasLimit: 6_000_000n, gasPrice: 2_000_000_000n,
      to: to ?? undefined, value, data: input,
    }, { common: this.common }).sign(sender.signer);
    assert.equal(tx.getSenderAddress().toString(), sender.address);
    const builder = await this.vm.buildBlock({
      parentBlock: this.tip,
      headerData: { number: this.tip.header.number + 1n, timestamp: time },
      blockOpts: { putBlockIntoBlockchain: false },
    });
    const result = await builder.addTransaction(tx);
    this.tip = await builder.build();
    const block = blockWire(this.tip);
    this.blocks.push(block);
    const loc = { blockNumber: block.number, blockHash: block.hash, transactionIndex: '0x0' };
    const hash = hexlify(tx.hash());
    const from = tx.getSenderAddress().toString();
    this.transactions.push({ ...loc, hash, from, to, input: hexlify(tx.data), value: toQuantity(tx.value), chainId: toQuantity(tx.common.chainId()) });
    const logs = result.receipt.logs.map(([emitter, topics, data], i) => ({
      ...loc, transactionHash: hash, address: hexlify(emitter), topics: topics.map(hexlify), data: hexlify(data),
      logIndex: toQuantity(i), removed: false,
    }));
    const receipt = { ...loc, transactionHash: hash, from, to, status: toQuantity(result.receipt.status), logs };
    this.receipts.push(receipt);
    return { result, receipt, block, created: result.createdAddress?.toString() };
  }

  async call(address, object, method, args = []) {
    const data = object.abi.encodeFunctionData(method, args);
    // eth_call must not persist even the simulated caller nonce.
    await this.vm.stateManager.checkpoint();
    try {
      const result = await this.vm.evm.runCall({ block: this.tip, to: Address.fromString(address), data: getBytes(data) });
      assert.equal(result.execResult.exceptionError, undefined);
      return hexlify(result.execResult.returnValue);
    } finally {
      await this.vm.stateManager.revert();
    }
  }

  async tokenBalance(address) {
    return BigInt(await this.call(token, artifacts.token, 'balanceOf', [address]));
  }

  async code(address) {
    return hexlify(await this.vm.stateManager.getContractCode(Address.fromString(address)));
  }

  async transcript(orderKey) {
    const calls = {};
    for (const [address, artifact, method, args] of [
      [sale, artifacts.sale, 'coin', []], [sale, artifacts.sale, 'treasury', []],
      [sale, artifacts.sale, 'declarationHash', []], [sale, artifacts.sale, 'isFilled', [orderKey]],
      [sale, artifacts.sale, 'orders', [orderKey]],
      [token, artifacts.token, 'declarationHash', []], [token, artifacts.token, 'decimals', []],
    ]) {
      const data = artifact.abi.encodeFunctionData(method, args);
      calls[address + ':' + data] = await this.call(address, artifact, method, args);
    }
    return {
      chain_id: chainId, finalized_hash: hexlify(this.tip.hash()), blocks: this.blocks, transactions: this.transactions,
      receipts: this.receipts, logs: this.receipts.flatMap((r) => r.logs),
      code: { [token]: await this.code(token), [sale]: await this.code(sale) }, calls,
    };
  }
}

function success(record) {
  assert.equal(record.result.execResult.exceptionError, undefined);
  assert.equal(record.receipt.status, '0x1');
  return record;
}
function refuses(record, name) {
  assert.equal(record.result.execResult.exceptionError?.error, 'revert');
  assert.equal(record.receipt.status, '0x0');
  assert.equal(artifacts.sale.abi.parseError(record.result.execResult.returnValue)?.name, name);
}
function saleCreation(payee, declaration = artifacts.declaration) {
  return concat([artifacts.sale.bytecode, artifacts.sale.abi.encodeDeploy([token, payee, declaration])]);
}
async function setup() {
  const chain = await LocalChain.start();
  assert.equal(success(await chain.send(owner, null, artifacts.token.bytecode)).created, token);
  assert.equal(success(await chain.send(owner, null, saleCreation(treasury.address))).created, sale);
  return chain;
}
function authorization(key, expires, amount = coins(100)) {
  return artifacts.sale.abi.encodeFunctionData('authorize', [key, buyer.address, amount, price, expires]);
}

test('sale enforces buyer, exact wei, availability, deadline and one-time delivery', async () => {
  const chain = await setup();
  const expires = firstTimestamp + 600;
  refuses(await chain.send(owner, null, saleCreation(treasury.address, id('other coin'))), 'InvalidConfiguration');
  const key = id('scoped local order');
  const buy = artifacts.sale.abi.encodeFunctionData('buy', [key]);
  refuses(await chain.send(buyer, sale, buy, price), 'UnknownOrder');
  refuses(await chain.send(buyer, sale, authorization(key, expires)), 'Unauthorized');
  success(await chain.send(owner, sale, authorization(key, expires)));
  refuses(await chain.send(owner, sale, authorization(key, expires, 7n)), 'OrderExists');
  refuses(await chain.send(owner, sale, buy, price), 'Unauthorized');
  refuses(await chain.send(buyer, sale, buy, price - 1n), 'WrongPrice');
  refuses(await chain.send(buyer, sale, buy, price + 1n), 'WrongPrice');
  const payee = () => chain.vm.stateManager.getAccount(Address.fromString(treasury.address));
  const originalBalance = (await payee()).balance;
  const unfunded = await chain.send(buyer, sale, buy, price);
  assert.equal(unfunded.receipt.status, '0x0');
  assert.equal(BigInt(await chain.call(sale, artifacts.sale, 'isFilled', [key])), 0n);
  assert.equal((await payee()).balance, originalBalance);
  success(await chain.send(owner, token, artifacts.token.abi.encodeFunctionData('transfer', [sale, coins(100_000)])));
  success(await chain.send(buyer, sale, buy, price));
  assert.equal(BigInt(await chain.call(sale, artifacts.sale, 'isFilled', [key])), 1n);
  assert.equal((await payee()).balance - originalBalance, price);
  assert.equal(await chain.tokenBalance(buyer.address), coins(100));
  assert.equal(await chain.tokenBalance(sale), coins(99_900));
  assert.equal(BigInt(await chain.call(token, artifacts.token, 'totalSupply')), coins(100_000));
  refuses(await chain.send(buyer, sale, buy, price), 'AlreadyFilled');
  assert.equal(await chain.tokenBalance(buyer.address), coins(100));
  assert.equal((await payee()).balance - originalBalance, price);

  const late = id('late local order');
  success(await chain.send(owner, sale, authorization(late, expires)));
  refuses(await chain.send(buyer, sale, artifacts.sale.abi.encodeFunctionData('buy', [late]), price, BigInt(expires)), 'OrderExpired');
  assert.equal(await chain.tokenBalance(buyer.address), coins(100));

  // A failing payee rolls back the token transfer and the consumed flag together.
  const rejectingPayee = success(await chain.send(owner, null, artifacts.refusal.bytecode)).created;
  const secondSale = success(await chain.send(owner, null, saleCreation(rejectingPayee))).created;
  const lateExpiry = expires + 600;
  success(await chain.send(owner, secondSale, authorization(key, lateExpiry)));
  success(await chain.send(buyer, token, artifacts.token.abi.encodeFunctionData('transfer', [secondSale, coins(100)])));
  refuses(await chain.send(buyer, secondSale, buy, price), 'PaymentFailed');
  assert.equal(await chain.tokenBalance(buyer.address), 0n);
  assert.equal(await chain.tokenBalance(secondSale), coins(100));
  assert.equal(BigInt(await chain.call(secondSale, artifacts.sale, 'isFilled', [key])), 0n);
});

function runBook(mode, db, payload) {
  return JSON.parse(execFileSync('python3', [bookDriver, mode, db], {
    input: JSON.stringify(payload), encoding: 'utf8', cwd: language,
  }));
}

test('reopened inventory book finds the paid EthereumJS purchase and accrues each obligation once', async (t) => {
  const chain = await setup();
  success(await chain.send(owner, token, artifacts.token.abi.encodeFunctionData('transfer', [sale, coins(100_000)])));
  const dir = mkdtempSync(join(tmpdir(), 'x3-paid-sale-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const db = join(dir, 'hold.sqlite');
  const policy = {
    chain_id: chainId, genesis_hash: chain.blocks[0].hash, sale_contract: sale,
    sale_code_hash: keccak256(await chain.code(sale)), token_contract: token,
    token_code_hash: keccak256(await chain.code(token)), treasury: treasury.address,
    from_block: Number(chain.tip.header.number), destination: 'ethereum', payment_wei: price.toString(),
  };
  const asset_id = artifacts.declaration.slice(2);
  const expires_at = firstTimestamp + 600;
  const prepared = runBook('prepare', db, { policy, asset_id, buyer: buyer.address, expires_at, now: firstTimestamp });
  const orderKey = prepared.terms.order_key;
  const { authorize, buy } = prepared.wallet_calls;
  assert.equal(authorize.to, sale);
  assert.equal(authorize.data, authorization(orderKey, expires_at));
  assert.equal(buy.to, sale);
  assert.equal(buy.from, buyer.address);
  assert.equal(BigInt(buy.value), price);
  assert.equal(buy.data, artifacts.sale.abi.encodeFunctionData('buy', [orderKey]));
  success(await chain.send(owner, authorize.to, authorize.data, BigInt(authorize.value)));
  runBook('before', db, { asset_id, now: firstTimestamp + 10, chain: await chain.transcript(orderKey) });

  const payee = () => chain.vm.stateManager.getAccount(Address.fromString(treasury.address));
  const beforePayment = (await payee()).balance;
  const filled = success(await chain.send(buyer, buy.to, buy.data, BigInt(buy.value)));
  assert.equal(await chain.tokenBalance(buyer.address), coins(100));
  assert.equal(await chain.tokenBalance(sale), coins(99_900));
  assert.equal((await payee()).balance - beforePayment, price);
  assert.equal(filled.receipt.logs[1].topics[0], artifacts.sale.abi.getEvent('SaleFilled').topicHash);
  // The buy is already in a block, but no transaction hash has been journaled.
  // Two new processes must discover the same transfer through indexed events.
  const observed = { asset_id, now: expires_at + 1, chain: await chain.transcript(orderKey) };
  // Terminate a separate process after its first SQLite obligation insert. The
  // next process must recover from the committed EVM event and an empty journal.
  assert.throws(() => runBook('interrupt', db, observed), (error) => error.status === 75);
  const recovered = runBook('recover', db, observed);
  assert.deepEqual(runBook('recover', db, observed), recovered);
  refuses(await chain.send(buyer, buy.to, buy.data, BigInt(buy.value)), 'AlreadyFilled');
  assert.equal(await chain.tokenBalance(buyer.address), coins(100));
  assert.equal((await payee()).balance - beforePayment, price);
});

test('unsigned Sepolia setup pins deployed immutable runtime and builds executable wallet calls offline', async (t) => {
  const dir = mkdtempSync(join(tmpdir(), 'x3-setup-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const publicSetup = {
    steward: owner.address, treasury: treasury.address, steward_nonce: '7', prefund: coins(100).toString(),
    buyer: buyer.address, order_id: 'local-setup', amount: coins(100).toString(),
    expires_at: Math.floor(Date.now() / 1000) + 600, payment_wei: price.toString(),
  };
  const source = join(dir, 'owner.x3');
  const original = readFileSync(join(language, 'coin/examples/dual_vm.x3'), 'utf8');
  writeFileSync(source, original);
  await assert.rejects(prepareSetup(source, publicSetup), /Sepolia/);
  const expectedToken = getCreateAddress({ from: owner.address, nonce: 7 }).toLowerCase();
  writeFileSync(source, original.replace('chain_id 31337', 'chain_id 11155111').replace(/contract 0x[0-9a-fA-F]{40}/,
    'contract ' + expectedToken));
  await assert.rejects(prepareSetup(source, { ...publicSetup, steward_nonce: '8' }), /ordinary CREATE address/);
  await assert.rejects(prepareSetup(source, { ...publicSetup, prefund: coins(100_000).toString() }), /exceeds declared local allocation/);
  const setup = await prepareSetup(source, publicSetup);
  assert.equal(setup.status, 'unsigned_setup');
  const chain = await LocalChain.start(11155111, 7n);
  const p = setup.partial_policy;
  assert.equal(p.token_contract, expectedToken);
  assert.equal(setup.reservation.asset_id, setup.asset_id);
  const calls = setup.wallet_calls;
  for (const call of [calls.deploy_coin, calls.deploy_sale, calls.prefund_sale]) {
    assert.equal(BigInt(call.chainId), 11155111n);
    assert.equal(call.from, owner.address);
    assert.equal(BigInt(call.nonce), (await chain.vm.stateManager.getAccount(Address.fromString(owner.address))).nonce);
    success(await chain.send(owner, call.to, call.data, BigInt(call.value)));
  }
  assert.equal(keccak256(await chain.code(p.token_contract)), p.token_code_hash);
  assert.equal(keccak256(await chain.code(p.sale_contract)), p.sale_code_hash);
  const inventory = await chain.call(p.token_contract, artifacts.token, 'balanceOf', [p.sale_contract]);
  assert.equal(BigInt(inventory), coins(100));
  assert.equal(await chain.call(p.sale_contract, artifacts.sale, 'declarationHash'), '0x' + setup.asset_id);
});

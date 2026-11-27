import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { Common, Hardfork } from '@ethereumjs/common';
import { Address } from '@ethereumjs/util';
import { VM } from '@ethereumjs/vm';
import { Interface, ZeroAddress, getBytes, hexlify } from 'ethers';
import solc from 'solc';

const languageDirectory = fileURLToPath(new URL('../../../', import.meta.url));
const owner = '0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf';
const contract = '0xF2E246BB76DF876Cef8b38ae84130F4F55De395b';
const spender = '0x0000000000000000000000000000000000000099';
const allocation = 600_000_000n;

function build() {
  const python = `
import json
from pathlib import Path
from coin import compile_coin
source = Path('coin/examples/dual_vm.x3').read_text(encoding='utf-8')
bundle = compile_coin(source)
print(json.dumps({k: bundle[k] for k in ('evm_solidity', 'declaration_hash')}))
`;
  const bundle = JSON.parse(execFileSync('python3', ['-c', python], { cwd: languageDirectory }));
  const output = JSON.parse(solc.compile(JSON.stringify({
    language: 'Solidity',
    sources: { 'Token.sol': { content: bundle.evm_solidity } },
    settings: { evmVersion: 'shanghai', outputSelection: { '*': { '*': ['abi', 'evm.bytecode.object'] } } },
  })));
  assert.deepEqual((output.errors ?? []).filter((message) => message.severity === 'error'), []);
  const compiled = output.contracts['Token.sol'].X3IssuedCoin;
  return {
    abi: new Interface(compiled.abi), bytecode: getBytes(`0x${compiled.evm.bytecode.object}`),
    hash: `0x${bundle.declaration_hash}`,
  };
}

function createVm(chainId) {
  return VM.create({ common: Common.custom({ chainId }, { hardfork: Hardfork.Shanghai }) });
}

function deploy(vm, artifact) {
  return vm.evm.runCall({ caller: Address.fromString(owner), data: artifact.bytecode });
}

function checkRevert(result, abi, expected) {
  assert.equal(result.execResult.exceptionError?.error, 'revert');
  assert.equal(abi.parseError(result.execResult.returnValue).name, expected);
}

function checkLog(result, abi, name, args) {
  const [[emitter, topics, data]] = result.execResult.logs;
  assert.equal(hexlify(emitter), contract.toLowerCase());
  const entry = abi.parseLog({ topics: topics.map(hexlify), data: hexlify(data) });
  assert.equal(entry.name, name);
  assert.deepEqual(entry.args.toArray(), args);
}

test('generated ERC-20 enforces the local genesis allocation and standard token operations', async () => {
  const artifact = build();
  const { abi } = artifact;
  const vm = await createVm(31337);
  const created = await deploy(vm, artifact);
  assert.equal(created.execResult.exceptionError, undefined);
  assert.equal(created.createdAddress.toString(), contract.toLowerCase());
  checkLog(created, abi, 'Transfer', [ZeroAddress, owner, allocation]);

  async function call(method, args = [], caller = owner) {
    return vm.evm.runCall({ to: Address.fromString(contract), caller: Address.fromString(caller),
      data: getBytes(abi.encodeFunctionData(method, args)) });
  }
  async function read(method, args = []) {
    const result = await call(method, args);
    assert.equal(result.execResult.exceptionError, undefined);
    return abi.decodeFunctionResult(method, result.execResult.returnValue)[0];
  }
  assert.equal(await read('declarationHash'), artifact.hash);
  assert.equal(await read('name'), 'X3 Demo Credit');
  assert.equal(await read('symbol'), 'DEMO');
  assert.equal(await read('decimals'), 6n);
  assert.equal(await read('totalSupply'), allocation);
  assert.equal(await read('balanceOf', [owner]), allocation);
  checkLog(await call('transfer', [spender, 7n]), abi, 'Transfer', [owner, spender, 7n]);
  checkLog(await call('approve', [spender, 13n]), abi, 'Approval', [owner, spender, 13n]);
  checkLog(await call('transferFrom', [owner, spender, 13n], spender), abi, 'Transfer', [owner, spender, 13n]);
  assert.equal(await read('allowance', [owner, spender]), 0n);
  checkRevert(await call('transferFrom', [owner, spender, 1n], spender), abi, 'ERC20InsufficientAllowance');
  checkRevert(await call('transfer', [spender, allocation]), abi, 'ERC20InsufficientBalance');
  checkRevert(await call('transfer', [ZeroAddress, 1n]), abi, 'ERC20InvalidReceiver');
  const mint = new Interface(['function mint(address,uint256)']);
  const remint = await vm.evm.runCall({ to: Address.fromString(contract), caller: Address.fromString(owner),
    data: getBytes(mint.encodeFunctionData('mint', [owner, allocation])) });
  assert.equal(remint.execResult.exceptionError?.error, 'revert');
  assert.equal(await read('balanceOf', [owner]), allocation - 20n);
  assert.equal(await read('balanceOf', [spender]), 20n);
  assert.equal(await read('totalSupply'), allocation);

  // CREATE at a subsequent nonce cannot create a second official local allocation.
  checkRevert(await deploy(vm, artifact), abi, 'WrongDeploymentAddress');
  const wrongNetwork = await createVm(31338);
  checkRevert(await deploy(wrongNetwork, artifact), abi, 'WrongDeploymentChain');
});

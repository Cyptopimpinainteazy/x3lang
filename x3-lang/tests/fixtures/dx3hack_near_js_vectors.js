// Generates signed-transaction vectors with the official @near-js libraries (test oracle only).
const { createTransaction, actionCreators, encodeTransaction, SignedTransaction, Signature } = require('@near-js/transactions');
const { KeyPair } = require('@near-js/crypto');
const { baseDecode, baseEncode } = require('@near-js/utils');
const crypto = require('crypto');
const seed = Buffer.alloc(32, 7);                       // fixed test-only seed
const kp = KeyPair.fromString('ed25519:' + baseEncode(Buffer.concat([seed, require('crypto').createPublicKey({key: Buffer.concat([Buffer.from('302e020100300506032b657004220420','hex'), seed]), format:'der', type:'pkcs8'}).export({format:'der', type:'spki'}).subarray(12)])));
const blockHash = Buffer.alloc(32, 9);
const cases = [
  ['faucet.x3lang.near', 'dx3hack.x3lang.near', 12345678901234n, 'add_holder', {account_id: 'j42.judges.x3lang.near'}, 30000000000000n, 0n],
  ['faucet.x3lang.testnet', 'dx3hack.x3lang.testnet', 1n, 'faucet_claim', {receiver_id: 'j1.judges.x3lang.testnet'}, 300000000000000n, 1n],
];
const out = cases.map(([signer, receiver, nonce, method, args, gas, deposit]) => {
  const tx = createTransaction(signer, kp.getPublicKey(), receiver, nonce,
    [actionCreators.functionCall(method, Buffer.from(JSON.stringify(args)), gas, deposit)], blockHash);
  const bytes = encodeTransaction(tx);
  const hash = crypto.createHash('sha256').update(bytes).digest();
  const sig = kp.sign(hash);
  const signed = new SignedTransaction({ transaction: tx, signature: new Signature({ keyType: tx.publicKey.keyType, data: sig.signature }) });
  return { signer, receiver, nonce: nonce.toString(), method, args, gas: gas.toString(), deposit: deposit.toString(),
           public_key: kp.getPublicKey().toString(), block_hash: baseEncode(blockHash),
           tx_hash: baseEncode(hash), signed_tx_base64: Buffer.from(signed.encode()).toString('base64') };
});
console.log(JSON.stringify({ source: '@near-js/transactions 2.5.1, @near-js/crypto', seed_hex: seed.toString('hex'), cases: out }, null, 2));

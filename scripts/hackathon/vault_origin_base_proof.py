#!/usr/bin/env python3
"""Fail-closed READ-ONLY two-RPC Base Sepolia vault-origin transaction verifier.

This module will never sign, broadcast, deploy, or move funds. A *positive*
result requires actual RPC chain responses and a separately reviewed pinned
runtime bytecode hash. It is NOT a NEAR settlement, Tidepool relayer causality,
or cross-chain atomicity proof. Never relabel old EOA ERC20 transfers as vault
payments. For PR review, tests use explicitly synthetic RPC responses ONLY.
"""
from __future__ import annotations

from dataclasses import dataclass
from hashlib import sha256
from typing import Any, Callable
from urllib.parse import urlsplit
import json

from keccak_min import keccak256

CHAIN_ID = 84532
TRANSFER_TOPIC = '0x' + keccak256(b'Transfer(address,address,uint256)').hex()
PAYMENT_TOPIC = '0x' + keccak256(b'PaymentExecuted(bytes32,address,uint256,bytes32)').hex()
PAYMENT_SELECTOR = keccak256(b'executePayment(bytes32,address,uint256,uint256,bytes)')[:4]
ZERO_ADDRESS = '0x' + '0'*40
Rpc = Callable[[str, list[Any]], Any]

class ProofRejected(ValueError):
    pass

def require(ok: bool, reason: str) -> None:
    if not ok:
        raise ProofRejected(reason)

def address(s: Any) -> str:
    require(isinstance(s,str) and len(s)==42 and s.startswith('0x') and all(c in '0123456789abcdefABCDEF' for c in s[2:]),'INVALID_ADDRESS')
    return s.lower()

def hash32(s: Any) -> str:
    require(isinstance(s,str) and len(s)==66 and s.startswith('0x') and all(c in '0123456789abcdefABCDEF' for c in s[2:]),'INVALID_BYTES32')
    return s.lower()

def as_bytes(raw: Any, reason: str) -> bytes:
    require(isinstance(raw,str) and raw.startswith('0x') and len(raw)%2==0,'MALFORMED_'+reason)
    try:
        return bytes.fromhex(raw[2:])
    except ValueError as exc:
        raise ProofRejected('MALFORMED_'+reason) from exc

def number(raw: Any) -> int:
    require(isinstance(raw,str) and raw.startswith('0x') and len(raw)>2, 'MISSING_HEX_NUMBER')
    try:
        n=int(raw,16)
    except ValueError as exc:
        raise ProofRejected('BAD_HEX_NUMBER') from exc
    require(n>=0,'NEGATIVE_NUMBER')
    return n

def uint256(x: int) -> bytes:
    require(type(x) is int and 0<=x<2**256,'INVALID_UINT256')
    return x.to_bytes(32,'big')

def encoded_address(a: str) -> bytes:
    return b'\0'*12 + bytes.fromhex(address(a)[2:])

def _read_word(data: bytes, start: int) -> bytes:
    require(0<=start and start+32<=len(data),'TRUNCATED_CALLDATA')
    return data[start:start+32]

def decode_payment_calldata(data_hex: Any) -> dict[str,Any]:
    b=as_bytes(data_hex,'CALLDATA')
    require(len(b)>=4+160+32,'TRUNCATED_EXECUTE_PAYMENT')
    require(b[:4]==PAYMENT_SELECTOR,'NOT_VAULT_EXECUTE_PAYMENT_SELECTOR')
    words=[_read_word(b,4+32*k) for k in range(5)]
    nonce='0x'+words[0].hex()
    require(nonce!='0x'+'0'*64,'ZERO_NONCE')
    require(words[1][:12]==b'\0'*12,'NONCANONICAL_ADDRESS_PADDING')
    recipient='0x'+words[1][12:].hex()
    amount=int.from_bytes(words[2],'big')
    deadline=int.from_bytes(words[3],'big')
    require(int.from_bytes(words[4],'big')==160,'NONCANONICAL_SIGNATURE_OFFSET')
    length=int.from_bytes(_read_word(b,4+160),'big')
    require(length==65,'SIGNATURE_MUST_BE_65_BYTES')
    pad=(length+31)//32*32
    require(len(b)==4+160+32+pad,'NONCANONICAL_CALLDATA_SIZE')
    sig=b[4+160+32:4+160+32+length]
    require(b[4+160+32+length:]==b'\0'*(pad-length),'NONZERO_SIGNATURE_PADDING')
    require(sig[-1] in (27,28),'INVALID_SIGNATURE_V')
    require(amount>0 and deadline>0,'INVALID_PAYMENT_FIELDS')
    return {'nonce':nonce,'recipient':recipient,'amount':amount,'deadline':deadline,'signature_len':length}

def encode_static_call(signature: str, args: bytes=b'') -> str:
    return '0x' + (keccak256(signature.encode())[:4] + args).hex()

@dataclass(frozen=True)
class Expected:
    vault: str
    owner: str
    agent: str
    token: str
    deposit_address: str
    quote_sha256: str
    program_sha256: str
    token_runtime_sha256: str
    vault_runtime_sha256: str
    nonce: str
    amount: int
    deadline: int
    expires_at: int
    max_per_payment: int
    max_total: int

    def validate(self) -> None:
        vals=[address(a) for a in (self.vault,self.owner,self.agent,self.token,self.deposit_address)]
        require(len(set(vals))==len(vals),'PRINCIPAL_COLLISION')
        for h in (self.quote_sha256,self.program_sha256,self.token_runtime_sha256,self.vault_runtime_sha256):
            require(isinstance(h,str) and len(h)==64 and all(c in '0123456789abcdef' for c in h),'UNPINNED_RUNTIME_OR_POLICY_HASH')
        hash32(self.nonce)
        require(type(self.amount) is int and 0<self.amount<=self.max_per_payment<=self.max_total,'INVALID_BUDGET_POLICY')
        require(type(self.deadline) is int and type(self.expires_at) is int and 0<self.deadline<=self.expires_at,'UNSAFE_EXPIRY')


def _call(rpc: Rpc, contract: str, signature: str, block_hex: str, args: bytes=b'') -> bytes:
    response=as_bytes(rpc('eth_call',[{'to':contract,'data':encode_static_call(signature,args)},block_hex]),'IMMUTABLE_CALL')
    require(len(response)==32,'IMMUTABLE_CALL_NOT_32_BYTES')
    return response

def _snapshot_immutables(rpc: Rpc, e: Expected, block_hex: str) -> None:
    for method, expected in (
        ('owner()',e.owner),('asset()',e.token),('agent()',e.agent),('recipient()',e.deposit_address)
    ):
        raw=_call(rpc,e.vault,method,block_hex)
        require(raw[:12]==b'\0'*12 and '0x'+raw[12:].hex()==address(expected),'VAULT_IMMUTABLE_MISMATCH_'+method)
    for method, expected in (('quoteHash()',e.quote_sha256),('programHash()',e.program_sha256)):
        require(_call(rpc,e.vault,method,block_hex).hex()==expected,'VAULT_IMMUTABLE_MISMATCH_'+method)
    require(_call(rpc,e.vault,'tokenCodeHash()',block_hex)==keccak256(as_bytes(rpc('eth_getCode',[e.token,block_hex]),'TOKEN_CODE')),
            'TOKEN_EVM_CODEHASH_IMMUTABLE_MISMATCH')
    for method, expected in (('maxPerPayment()',e.max_per_payment),('maxTotal()',e.max_total),('expiresAt()',e.expires_at)):
        require(int.from_bytes(_call(rpc,e.vault,method,block_hex),'big')==expected,'VAULT_IMMUTABLE_MISMATCH_'+method)
    nonce_arg=bytes.fromhex(hash32(e.nonce)[2:])
    require(int.from_bytes(_call(rpc,e.vault,'usedNonces(bytes32)',block_hex,nonce_arg),'big')==1,'NONCE_NOT_CONSUMED_IN_CANONICAL_STATE')
    require(int.from_bytes(_call(rpc,e.vault,'spent()',block_hex),'big')>=e.amount,'VAULT_SPENT_LEDGER_TOO_LOW')

def _scan_receipt_logs(receipt: dict[str,Any], e: Expected, digest: str, block_hash: str, txhash: str) -> None:
    logs=receipt.get('logs')
    require(isinstance(logs,list),'LOGS_MISSING')
    payments=[]; transfers=[]; indices=set()
    for log in logs:
        require(isinstance(log,dict),'BAD_LOG')
        require(hash32(log.get('transactionHash'))==txhash,'LOG_TX_HASH_MISMATCH')
        require(hash32(log.get('blockHash'))==block_hash,'LOG_BLOCK_HASH_MISMATCH')
        require(log.get('removed') is not True,'REORGED_LOG')
        idx=number(log.get('logIndex'))
        require(idx not in indices,'DUPLICATE_LOG_INDEX')
        indices.add(idx)
        logaddr=address(log.get('address'))
        topics=log.get('topics')
        require(isinstance(topics,list) and all(isinstance(x,str) for x in topics),'BAD_TOPICS')
        if not topics: continue
        top0=hash32(topics[0])
        if logaddr==address(e.vault) and top0==PAYMENT_TOPIC:
            require(len(topics)==3,'NONCANONICAL_PAYMENT_TOPICS')
            require(hash32(topics[1])==hash32(e.nonce),'PAYMENT_NONCE_MISMATCH')
            require(hash32(topics[2])=='0x'+encoded_address(e.deposit_address).hex(),'PAYMENT_EVENT_RECIPIENT_MISMATCH')
            data=as_bytes(log.get('data'),'PAYMENT_EVENT')
            require(len(data)==64,'BAD_PAYMENT_EVENT_DATA')
            require(int.from_bytes(data[:32],'big')==e.amount,'PAYMENT_EVENT_AMOUNT_MISMATCH')
            require('0x'+data[32:].hex()==digest,'PAYMENT_EIP712_DIGEST_MISMATCH')
            payments.append(idx)
        if logaddr==address(e.token) and top0==TRANSFER_TOPIC:
            require(len(topics)==3,'NONCANONICAL_TRANSFER_TOPICS')
            if hash32(topics[1])=='0x'+encoded_address(e.vault).hex():
                require(hash32(topics[2])=='0x'+encoded_address(e.deposit_address).hex(),'TOKEN_TRANSFER_REDIRECTED')
                data=as_bytes(log.get('data'),'TOKEN_TRANSFER')
                require(len(data)==32 and int.from_bytes(data,'big')==e.amount,'TOKEN_TRANSFER_WRONG_AMOUNT')
                transfers.append(idx)
    require(len(payments)==1,'VAULT_PAYMENT_EVENT_MISSING_OR_DUPLICATE')
    require(len(transfers)==1,'VAULT_TOKEN_TRANSFER_MISSING_OR_DUPLICATE')
    require(payments[0]<transfers[0],'VAULT_EVENT_MUST_PRECEDE_TOKEN_PAYOUT')

def verify_one(rpc: Rpc, e: Expected, txhash: str, *, min_confirmations: int=12) -> dict[str, Any]:
    e.validate()
    txhash=hash32(txhash)
    require(number(rpc('eth_chainId',[]))==CHAIN_ID,'NOT_BASE_SEPOLIA')
    tx=rpc('eth_getTransactionByHash',[txhash])
    receipt=rpc('eth_getTransactionReceipt',[txhash])
    require(isinstance(tx,dict) and isinstance(receipt,dict),'TX_OR_RECEIPT_MISSING')
    require(hash32(tx.get('hash'))==txhash and hash32(receipt.get('transactionHash'))==txhash,'TRANSACTION_HASH_MISMATCH')
    # Critical: the old successful EOA->ERC20 transfer is NOT a vault transfer.
    require(address(tx.get('to'))==address(e.vault),'NOT_VAULT_ORIGIN_TX_TO')
    require(address(receipt.get('to'))==address(e.vault),'NOT_VAULT_ORIGIN_RECEIPT_TO')
    require(address(receipt.get('from'))==address(tx.get('from')),'RELAY_SENDER_MISMATCH')
    require(number(tx.get('value','0x0'))==0,'UNEXPECTED_NATIVE_VALUE')
    require(number(receipt.get('status'))==1,'VAULT_TX_REVERTED')
    bnum=number(receipt.get('blockNumber'))
    bhex=hex(bnum)
    blockhash=hash32(receipt.get('blockHash'))
    require(tx.get('blockNumber')==bhex and hash32(tx.get('blockHash'))==blockhash,'TX_RECEIPT_BLOCK_MISMATCH')
    block=rpc('eth_getBlockByNumber',[bhex,False])
    require(isinstance(block,dict) and hash32(block.get('hash'))==blockhash and number(block.get('number'))==bnum,'NONCANONICAL_INCLUSION')
    timestamp=number(block.get('timestamp'))
    finalized=rpc('eth_getBlockByNumber',['finalized',False])
    require(isinstance(finalized,dict),'FINALIZED_HEAD_UNAVAILABLE')
    finalized_num=number(finalized.get('number'))
    finalized_hash=hash32(finalized.get('hash'))
    require(finalized_num>=bnum,'BASE_NOT_FINALIZED')
    head=number(rpc('eth_blockNumber',[]))
    require(type(min_confirmations) is int and min_confirmations>=12 and head>=bnum+min_confirmations-1,'INSUFFICIENT_CONFIRMATIONS')
    call=decode_payment_calldata(tx.get('input'))
    require(call['nonce']==hash32(e.nonce),'SIGNED_ORDER_NONCE_MISMATCH')
    require(call['recipient']==address(e.deposit_address),'SIGNED_ORDER_DEPOSIT_REDIRECT')
    require(call['amount']==e.amount,'SIGNED_ORDER_AMOUNT_MISMATCH')
    require(call['deadline']==e.deadline,'SIGNED_ORDER_DEADLINE_MISMATCH')
    require(timestamp<=e.deadline and timestamp<e.expires_at,'VAULT_PAYMENT_AFTER_EXPIRY')
    vaultcode=as_bytes(rpc('eth_getCode',[e.vault,bhex]),'VAULT_CODE')
    require(len(vaultcode)>0 and sha256(vaultcode).hexdigest()==e.vault_runtime_sha256,'VAULT_RUNTIME_CODE_MISMATCH')
    token_code=as_bytes(rpc('eth_getCode',[e.token,bhex]),'TOKEN_CODE')
    require(len(token_code)>0 and sha256(token_code).hexdigest()==e.token_runtime_sha256,'TOKEN_RUNTIME_CODE_MISMATCH')
    _snapshot_immutables(rpc,e,bhex)
    args=bytes.fromhex(e.nonce[2:])+encoded_address(e.deposit_address)+uint256(e.amount)+uint256(e.deadline)
    digest='0x'+_call(rpc,e.vault,'paymentDigest(bytes32,address,uint256,uint256)',bhex,args).hex()
    _scan_receipt_logs(receipt,e,digest,blockhash,txhash)
    return {
        'tx_hash':txhash,'vault':address(e.vault),'token':address(e.token),
        'deposit_address':address(e.deposit_address),'amount':e.amount,'nonce':hash32(e.nonce),
        'quote_hash':e.quote_sha256,'program_hash':e.program_sha256,
        'payment_digest':digest,'block_number':bnum,'block_hash':blockhash,
        'inclusion_unix_s':timestamp,'confirmed_count':head-bnum+1,
        'finalized_height':finalized_num,'finalized_hash':finalized_hash,
        'vault_runtime_code_sha256':e.vault_runtime_sha256,
        'token_runtime_code_sha256':e.token_runtime_sha256,
        'relayer':address(tx.get('from')),
    }

def verify_two_providers(
    observations: tuple[tuple[str,Rpc], tuple[str,Rpc]], e: Expected, txhash: str
) -> dict[str,Any]:
    require(len(observations)==2,'TWO_BASE_PROVIDERS_REQUIRED')
    urls=[urlsplit(url) for url,_ in observations]
    require(all(u.scheme=='https' and u.hostname and not u.username and not u.password
                and not u.query and not u.fragment for u in urls),'BAD_BASE_PROVIDER')
    require(urls[0].hostname!=urls[1].hostname,'BASE_PROVIDERS_NOT_INDEPENDENT')
    a=verify_one(observations[0][1],e,txhash)
    b=verify_one(observations[1][1],e,txhash)
    immutable=[x for x in a if x not in ('confirmed_count','finalized_height','finalized_hash')]
    require(all(a[k]==b[k] for k in immutable),'BASE_TWO_RPC_DISAGREEMENT')
    require(abs(a['finalized_height']-b['finalized_height'])<=6,'FINALIZED_HEAD_DRIFT_EXCEEDED')
    if a['finalized_height']==b['finalized_height']:
        require(a['finalized_hash']==b['finalized_hash'],'FINALIZED_HEAD_HASH_DISAGREEMENT')
    require(abs(a['confirmed_count']-b['confirmed_count'])<=6,'CONFIRMATION_HEAD_DRIFT_EXCEEDED')
    return {
        'classification':'READ_ONLY_BASE_VAULT_ORIGIN_FINALIZED_EVIDENCE_NOT_CROSSCHAIN_PROOF',
        'independent_provider_count':2,
        'provider_hosts':[u.hostname for u in urls],
        'observations':[a,b],
        'signed':False,'broadcast':False,'destination_near_verified':False,
        'relayer_atomicity_proven':False,
    }

# Live network calls belong ONLY in a separately authorized read-only caller.
# No private keys or transaction-construction paths are present in this file.

# CLI only: genuine two-provider read-only JSON-RPC transport. Unit test FakeRPC
# responses MUST NOT be reused as on-chain evidence.
def _https_rpc(url: str) -> Rpc:
    from urllib.request import Request, urlopen
    parsed=urlsplit(url)
    require(parsed.scheme=='https' and parsed.hostname and not parsed.username and not parsed.password
            and not parsed.query and not parsed.fragment,'RPC_MUST_USE_CREDENTIAL_FREE_HTTPS')
    allowed={
        'eth_chainId', 'eth_getTransactionByHash', 'eth_getTransactionReceipt',
        'eth_getBlockByNumber', 'eth_blockNumber', 'eth_getCode', 'eth_call',
    }
    def query(method: str, params: list[Any]) -> Any:
        require(method in allowed,'NON_READ_ONLY_RPC_METHOD_FORBIDDEN')
        import urllib.error
        from urllib.request import Request, urlopen
        payload=json.dumps({'jsonrpc':'2.0','id':'x3vault-proof-readonly','method':method,'params':params},
                           separators=(',',':'),allow_nan=False).encode()
        request=Request(url,data=payload,headers={'Content-Type':'application/json','User-Agent':'X3Shield-Readonly-Vault-Proof/1'})
        try:
            with urlopen(request,timeout=15) as response:
                require(response.status==200,'BASE_RPC_NON_200')
                data=json.load(response)
        except (OSError,ValueError) as exc:
            raise ProofRejected('BASE_RPC_UNAVAILABLE_'+method) from exc
        require(isinstance(data,dict) and data.get('jsonrpc')=='2.0' and data.get('id')=='x3vault-proof-readonly'
                and 'error' not in data and 'result' in data,'BASE_RPC_MALFORMED_'+method)
        return data['result']
    return query

def run_readonly_cli(argv: list[str] | None=None) -> int:
    import argparse
    from dataclasses import fields
    from pathlib import Path
    import sys
    p=argparse.ArgumentParser(description='Read-only, fail-closed Base Sepolia VAULT-ORIGIN payment proof; does not verify NEAR.')
    p.add_argument('--review-manifest',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    args=p.parse_args(argv)
    try:
        manifest=json.loads(args.review_manifest.read_text())
        expected_fields={f.name for f in fields(Expected)}
        require(isinstance(manifest,dict) and set(manifest)==expected_fields | {'tx_hash','rpc_urls'},'MANIFEST_FIELD_SET_MISMATCH')
        urls=manifest['rpc_urls']
        require(isinstance(urls,list) and len(urls)==2,'TWO_BASE_RPC_URLS_REQUIRED')
        e=Expected(**{k:manifest[k] for k in expected_fields})
        e.validate()
        report=verify_two_providers(((urls[0],_https_rpc(urls[0])),(urls[1],_https_rpc(urls[1]))),e,manifest['tx_hash'])
        report['schema']='x3.base_sepolia_vault_origin_readonly_receipt.v1'
        report['review_manifest_is_owner_signed']=False
        report['fresh_maker_signature_and_guard_not_verified_by_this_module']=True
        report['near_transfer_finality_and_balance_not_verified_by_this_module']=True
        report['not_trustless_atomic_settlement']=True
        serialized=(json.dumps(report,sort_keys=True,indent=2)+'\n').encode()
        # No overwrite, to avoid silently changing an observed evidence file.
        args.output.parent.mkdir(parents=True,exist_ok=True)
        with args.output.open('xb') as f:
            f.write(serialized)
        print(json.dumps({'classification':report['classification'],'sha256':sha256(serialized).hexdigest(),
                          'output':str(args.output),'broadcast':False,'near_verified':False},sort_keys=True))
        return 0
    except (ProofRejected,OSError,ValueError,KeyError,TypeError) as exc:
        print(json.dumps({'classification':'HARD_DENIED_NO_VAULT_ORIGIN_PROOF','reason':str(exc),
                          'signed':False,'broadcast':False,'near_verified':False}),file=sys.stderr)
        return 2

if __name__=='__main__':
    raise SystemExit(run_readonly_cli())

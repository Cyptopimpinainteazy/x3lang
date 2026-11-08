"""Pure correlation gate for *already independently queried* Base & NEAR reports.

Checks the deterministic #140 X3 owner-nonce -> Base EIP-712 nonce binding,
quote/program hashes, sequence, amounts and source/destination chronology.
DOES NOT validate Tidepool's maker signature, owner signature, or trustless
atomic settlement. Never call this function on arbitrary downloaded JSON and
mistake a returned classification for network-authenticated proof.
"""
from __future__ import annotations
import hashlib
from typing import Any
from vault_origin_base_proof import Expected, ProofRejected, require
from near_delivery_receipt_proof import NearExpected


def x3_single_quote_base_nonce(quote_sha256:str, program_sha256:str, x3_owner_nonce:str)->str:
    require(isinstance(quote_sha256,str) and len(quote_sha256)==64 and
            isinstance(program_sha256,str) and len(program_sha256)==64 and
            isinstance(x3_owner_nonce,str) and len(x3_owner_nonce)>0,
            'MISSING_X3_NONCE_BINDING')
    try:
        content=(b'X3_SHIELD_SINGLE_QUOTE_BASE_VAULT_NONCE_V1\x00'
            +bytes.fromhex(quote_sha256)+bytes.fromhex(program_sha256)+x3_owner_nonce.encode())
    except ValueError as exc:
        raise ProofRejected('INVALID_X3_NONCE_BINDING') from exc
    return '0x'+hashlib.sha256(content).hexdigest()


def correlate(base:dict[str,Any],near:dict[str,Any],
              ebase:Expected,enear:NearExpected)->dict[str,Any]:
    require(base.get('classification')=='READ_ONLY_BASE_VAULT_ORIGIN_FINALIZED_EVIDENCE_NOT_CROSSCHAIN_PROOF'
            and base.get('independent_provider_count')==2,
            'MISSING_BASE_FINALIZED_SOURCE_VERIFIER')
    require(near.get('classification')=='READ_ONLY_TWO_RPC_FINALIZED_NEAR_RECEIPT_BALANCE_AND_GUARD_NOT_ATOMICITY'
            and near.get('provider_count')==2,
            'MISSING_NEAR_FINALIZED_RECEIPT_GUARD_AND_BALANCE_VERIFIER')
    require(ebase.quote_sha256==enear.quote_sha256 and ebase.program_sha256==enear.program_sha256,
            'QUOTE_OR_X3_PROGRAM_HASH_NOT_CORRELATED')
    require(ebase.nonce.lower()==x3_single_quote_base_nonce(enear.quote_sha256,enear.program_sha256,enear.nonce),
            'OWNER_NONCE_NOT_BOUND_TO_BASE_EIP712_NONCE')
    require(ebase.amount==enear.origin_amount,'ORIGIN_AMOUNT_NOT_CORRELATED')
    require(all(x.get('quote_hash')==ebase.quote_sha256 and x.get('program_hash')==ebase.program_sha256
                and x.get('tx_hash')==base['observations'][0]['tx_hash'] for x in base.get('observations',[])),
            'BASE_PROOF_QUOTE_PROGRAM_OR_TRANSACTION_DISAGREEMENT')
    require(all(x.get('quote_hash')==enear.quote_sha256 and x.get('program_hash')==enear.program_sha256
                and x.get('txhash')==enear.txhash for x in near.get('observations',[])),
            'NEAR_PROOF_QUOTE_PROGRAM_OR_TRANSACTION_DISAGREEMENT')
    source_t=base['observations'][0]['inclusion_unix_s']
    require(source_t==enear.source_inclusion_unix_s,'SOURCE_RECEIPT_TIME_NOT_MATCHED_TO_NEAR_INPUT')
    for n in near['observations']:
        require(n['guard_historical']['approved_unix_s']<=source_t,'GUARD_APPROVAL_TOO_LATE')
        require(source_t<=n['ft_receipt']['unix_s']<=source_t+enear.max_settlement_lag_s,
                'NEAR_PAYOUT_OUTSIDE_SETTLEMENT_WINDOW')
        require(n['balance_delta']>=enear.minimum_units,'DESTINATION_BALANCE_BELOW_SIGNED_FLOOR')
    require(base['observations'][0]['deposit_address'].lower()==ebase.deposit_address.lower(),
            'SIGNED_MAKER_DEPOSIT_ADDRESS_NOT_CORRELATED')
    # Do not make the common mistake of deriving proof of relayer causality
    # from temporal proximity and matching amounts on two different chains.
    return {
        'classification':'TWO_CHAIN_READ_ONLY_OBSERVATIONS_PAIRED_MAKER_SIGNER_NOT_VERIFIED_HERE_NOT_ATOMIC',
        'source_base_vault_tx':base['observations'][0]['tx_hash'],
        'destination_near_tx':enear.txhash,
        'quote_sha256':ebase.quote_sha256,
        'program_sha256':ebase.program_sha256,
        'owner_x3_nonce':enear.nonce,
        'base_eip712_nonce':ebase.nonce,
        'base_provider_count':2,'near_provider_count':2,
        'destination_minimum_units':enear.minimum_units,
        'maker_signature_verified_by_this_module':False,
        'owner_signature_verified_by_this_module':False,
        'relayer_causality_cryptographically_proven':False,
        'trustless_atomic_settlement_proven':False,
        'signed':False,'broadcast':False,
    }

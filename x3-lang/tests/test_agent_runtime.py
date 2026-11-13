"""Public-boundary tests against real SQLite and real Ed25519 signatures."""
import tempfile
import unittest
from pathlib import Path
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization
from agent_constitution import AgentManifest
from agent_runtime import Runtime, Adapter, Action, genesis_payload


def raw(key):
    return key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)


class RuntimeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / 'runtime.sqlite'
        self.owner = Ed25519PrivateKey.generate()
        self.agent = Ed25519PrivateKey.generate()
        self.calls = []
        self.adapters = {'compile': Adapter('compile', 'local', 2, False, lambda code: self.calls.append(code) or b'compiled')}
        self.runtime = Runtime(self.path, raw(self.owner), self.adapters, clock=lambda: 10)
        self.manifest = AgentManifest('root', None, frozenset({'compile'}), frozenset({'local'}), 10, 100, (), 2)
        payload = self.runtime.genesis_payload(self.manifest, raw(self.agent), 'birth')
        self.runtime.genesis(self.manifest, raw(self.agent), 'birth', self.owner.sign(payload))

    def test_success_replay_and_restart(self):
        action = Action('root', 'compile', b'intent example', 'one', 50)
        signature = self.agent.sign(self.runtime.action_payload(action))
        self.assertEqual(self.runtime.execute(action, signature), b'compiled')
        self.assertEqual(self.calls, [b'intent example'])
        restarted = Runtime(self.path, raw(self.owner), self.adapters, clock=lambda: 10)
        with self.assertRaises(ValueError):
            restarted.execute(action, signature)
        self.assertEqual(self.calls, [b'intent example'])
        self.assertEqual(restarted.snapshot('root')['remaining'], 8)

    def action(self, nonce='one', adapter='compile', artifact=b'intent example'):
        return Action('root', adapter, artifact, nonce, 50)

    def run_action(self, action):
        return self.runtime.execute(action, self.agent.sign(self.runtime.action_payload(action)))

    def test_wrong_signer_cannot_kill_victim(self):
        action = self.action(adapter='not-authorized')
        attacker = Ed25519PrivateKey.generate()
        with self.assertRaises(ValueError):
            self.runtime.execute(action, attacker.sign(self.runtime.action_payload(action)))
        self.assertEqual(self.runtime.snapshot('root')['status'], 'active')
        self.assertEqual(len(self.runtime.events()), 1)

    def test_authenticated_forbidden_action_retires_and_preserves_evidence(self):
        from agent_runtime import digest
        action = self.action(adapter='not-authorized')
        with self.assertRaisesRegex(ValueError, 'unknown adapter'):
            self.run_action(action)
        self.assertEqual(self.runtime.snapshot('root')['status'], 'retired')
        proof = self.runtime.evidence(digest(self.runtime.action_payload(action)))
        self.assertEqual(proof['artifact'], action.artifact)
        self.assertEqual(proof['status'], 'denied')
        with self.assertRaisesRegex(ValueError, 'inactive'):
            self.run_action(self.action(nonce='later'))
        self.assertEqual(self.calls, [])

    def test_revocation_survives_restart(self):
        self.runtime.revoke('root', 'operator stop')
        runtime = Runtime(self.path, raw(self.owner), self.adapters, clock=lambda: 10)
        action = self.action()
        with self.assertRaisesRegex(ValueError, 'inactive'):
            runtime.execute(action, self.agent.sign(runtime.action_payload(action)))
        self.assertEqual(self.calls, [])

    def test_adapter_policy_change_is_rejected_on_restart(self):
        with self.assertRaisesRegex(ValueError, 'configuration changed'):
            Runtime(self.path, raw(self.owner),
                    {'compile': Adapter('compile', 'local', 0, False, lambda x: x)},
                    clock=lambda: 10)

    def test_other_operator_is_rejected_on_restart(self):
        with self.assertRaisesRegex(ValueError, 'configuration changed'):
            Runtime(self.path, raw(Ed25519PrivateKey.generate()), self.adapters, clock=lambda: 10)

    def test_action_expiry_and_artifact_tampering(self):
        from dataclasses import replace
        action = self.action()
        signature = self.agent.sign(self.runtime.action_payload(action))
        with self.assertRaisesRegex(ValueError, 'signature'):
            self.runtime.execute(replace(action, artifact=b'changed'), signature)
        expired = replace(action, expires_at=10)
        with self.assertRaisesRegex(ValueError, 'expired'):
            self.run_action(expired)
        self.assertEqual(self.runtime.snapshot('root')['remaining'], 10)

    def test_crash_after_reservation_never_retries(self):
        from agent_runtime import digest
        class Interrupted(BaseException):
            pass
        def crash(_):
            raise Interrupted()
        runtime = Runtime(self.path, raw(self.owner),
                          {'compile': Adapter('compile', 'local', 2, False, crash)},
                          clock=lambda: 10)
        action = self.action()
        signature = self.agent.sign(runtime.action_payload(action))
        with self.assertRaises(Interrupted):
            runtime.execute(action, signature)
        restarted = Runtime(self.path, raw(self.owner), self.adapters, clock=lambda: 10)
        with self.assertRaisesRegex(ValueError, 'already reserved'):
            restarted.execute(action, signature)
        self.assertEqual(restarted.evidence(digest(restarted.action_payload(action)))['status'], 'pending')
        self.assertEqual(restarted.snapshot('root')['remaining'], 8)
        self.assertEqual(self.calls, [])

    def test_failure_is_recorded_without_refund_or_retry(self):
        from agent_runtime import digest
        def fail(_):
            raise RuntimeError('provider failed')
        runtime = Runtime(self.path, raw(self.owner),
                          {'compile': Adapter('compile', 'local', 2, False, fail)},
                          clock=lambda: 10)
        action = self.action()
        signature = self.agent.sign(runtime.action_payload(action))
        with self.assertRaises(RuntimeError):
            runtime.execute(action, signature)
        self.assertEqual(runtime.evidence(digest(runtime.action_payload(action)))['status'], 'failed')
        with self.assertRaisesRegex(ValueError, 'already reserved'):
            runtime.execute(action, signature)
        self.assertEqual(runtime.snapshot('root')['remaining'], 8)

    def test_duplicate_concurrent_action_executes_once(self):
        from concurrent.futures import ThreadPoolExecutor
        action = self.action()
        signature = self.agent.sign(self.runtime.action_payload(action))
        def call():
            try:
                self.runtime.execute(action, signature)
                return 'success'
            except ValueError:
                return 'denied'
        with ThreadPoolExecutor(max_workers=2) as pool:
            results = list(pool.map(lambda _: call(), range(2)))
        self.assertCountEqual(results, ['success', 'denied'])
        self.assertEqual(len(self.calls), 1)

    def admit_child(self, child_id='child', budget=4, nonce='child-birth'):
        from agent_admission_signed import SignedApproval, approval_payload
        from agent_runtime import birth_payload
        child = AgentManifest(child_id, 'root', frozenset({'compile'}), frozenset({'local'}),
                              budget, 90, ('root',), 0)
        key = Ed25519PrivateKey.generate()
        approval = SignedApproval(nonce, 80, self.agent.sign(
            approval_payload(child, self.manifest, nonce=nonce, expires_at=80)))
        self.runtime.admit(child, raw(key), approval,
                           self.owner.sign(self.runtime.birth_payload(child, raw(key), approval)))
        return child, key

    def test_child_budget_reserved_and_revoked_ancestor_blocks(self):
        child, key = self.admit_child()
        self.assertEqual(self.runtime.snapshot('root')['remaining'], 6)
        self.assertEqual(self.runtime.snapshot('child')['remaining'], 4)
        self.runtime.revoke('root', 'parent stop')
        action = Action(child.agent_id, 'compile', b'child code', 'use', 50)
        with self.assertRaisesRegex(ValueError, 'ancestor inactive'):
            self.runtime.execute(action, key.sign(self.runtime.action_payload(action)))
        self.assertEqual(self.calls, [])

    def test_duplicate_admission_rolls_back_budget(self):
        child, _ = self.admit_child()
        with self.assertRaises(ValueError):
            self.admit_child()
        self.assertEqual(self.runtime.snapshot('root')['remaining'], 6)

    def test_child_signing_key_is_bound_to_operator_birth_signature(self):
        from agent_admission_signed import SignedApproval, approval_payload
        from agent_runtime import birth_payload
        child = AgentManifest('child', 'root', frozenset({'compile'}), frozenset({'local'}),
                              4, 90, ('root',), 0)
        key = Ed25519PrivateKey.generate()
        other = Ed25519PrivateKey.generate()
        approval = SignedApproval('birth', 80, self.agent.sign(
            approval_payload(child, self.manifest, nonce='birth', expires_at=80)))
        signature = self.owner.sign(self.runtime.birth_payload(child, raw(key), approval))
        with self.assertRaisesRegex(ValueError, 'signature'):
            self.runtime.admit(child, raw(other), approval, signature)
        self.assertEqual(self.runtime.snapshot('root')['remaining'], 10)

    def test_budget_exhaustion_retires(self):
        for i in range(5):
            self.run_action(self.action(nonce=str(i)))
        with self.assertRaisesRegex(ValueError, 'budget exhausted'):
            self.run_action(self.action(nonce='excess'))
        self.assertEqual(len(self.calls), 5)
        self.assertEqual(self.runtime.snapshot('root')['status'], 'retired')

    def test_jury_needs_fixed_majority_and_cannot_replay_other_artifact(self):
        from agent_runtime import JuryVote, digest, vote_payload
        jurors = {f'j{i}': Ed25519PrivateKey.generate() for i in range(3)}
        adapters = {'reviewed': Adapter('compile', 'local', 1, True, lambda x: b'approved')}
        runtime = Runtime(Path(self.temp.name) / 'jury.sqlite', raw(self.owner), adapters,
                          jury_keys={i: raw(k) for i,k in jurors.items()}, clock=lambda: 10)
        runtime.genesis(self.manifest, raw(self.agent), 'birth',
                        self.owner.sign(runtime.genesis_payload(self.manifest, raw(self.agent), 'birth')))
        action = self.action(adapter='reviewed')
        h = digest(runtime.action_payload(action))
        votes = [JuryVote(i, True, 40, k.sign(vote_payload(h, i, True, 40)))
                 for i,k in jurors.items()]
        sig = self.agent.sign(runtime.action_payload(action))
        with self.assertRaisesRegex(ValueError, 'majority'):
            runtime.execute(action, sig, votes[:1])
        self.assertEqual(runtime.execute(action, sig, votes[:2]), b'approved')
        other = self.action(nonce='other', adapter='reviewed', artifact=b'different')
        with self.assertRaisesRegex(ValueError, 'signature'):
            runtime.execute(other, self.agent.sign(runtime.action_payload(other)), votes)

    def test_jury_self_key_alias_and_duplicate_keys_rejected(self):
        keys = {'alias': raw(self.agent), 'j1': raw(Ed25519PrivateKey.generate()),
                'j2': raw(Ed25519PrivateKey.generate())}
        adapters = {'reviewed': Adapter('compile','local',1,True,lambda x:b'ok')}
        runtime = Runtime(Path(self.temp.name) / 'jury-self.sqlite',raw(self.owner),adapters,
                          jury_keys=keys,clock=lambda:10)
        runtime.genesis(self.manifest,raw(self.agent),'birth',
                        self.owner.sign(runtime.genesis_payload(self.manifest,raw(self.agent),'birth')))
        action=self.action(adapter='reviewed')
        with self.assertRaisesRegex(ValueError,'three independent'):
            runtime.execute(action,self.agent.sign(runtime.action_payload(action)))
        with self.assertRaisesRegex(ValueError,'duplicate juror key'):
            Runtime(Path(self.temp.name)/'duplicate.sqlite',raw(self.owner),adapters,
                    jury_keys={'a':raw(self.agent),'b':raw(self.agent)})

    def test_corrupt_audit_blocks_restart(self):
        import sqlite3
        with sqlite3.connect(self.path) as db:
            db.execute("UPDATE events SET payload='tampered' WHERE seq=1")
        with self.assertRaisesRegex(ValueError,'audit log corruption'):
            Runtime(self.path, raw(self.owner), self.adapters, clock=lambda:10)

    def test_strict_numbers_reject_bool_and_empty_artifact(self):
        from dataclasses import replace
        with self.assertRaises(ValueError):
            Runtime(Path(self.temp.name)/'bool.sqlite',raw(self.owner),
                    {'compile':Adapter('compile','local',True,False,lambda x:x)})
        with self.assertRaises(ValueError):
            self.runtime.action_payload(replace(self.action(),expires_at=True))
        with self.assertRaises(ValueError):
            self.runtime.action_payload(replace(self.action(),artifact=b''))

    def test_root_grant_and_action_cannot_cross_deployments(self):
        other = Runtime(Path(self.temp.name) / 'other.sqlite', raw(self.owner), self.adapters, clock=lambda: 10)
        source_grant = self.owner.sign(self.runtime.genesis_payload(self.manifest, raw(self.agent), 'elsewhere'))
        with self.assertRaisesRegex(ValueError, 'signature'):
            other.genesis(self.manifest, raw(self.agent), 'elsewhere', source_grant)
        other.genesis(self.manifest, raw(self.agent), 'elsewhere',
                      self.owner.sign(other.genesis_payload(self.manifest, raw(self.agent), 'elsewhere')))
        action = self.action()
        with self.assertRaisesRegex(ValueError, 'signature'):
            other.execute(action, self.agent.sign(self.runtime.action_payload(action)))
        self.assertEqual(other.snapshot('root')['status'], 'active')

    def test_policy_revision_is_bound_to_action_and_registry(self):
        other = Runtime(Path(self.temp.name)/'revision.sqlite',raw(self.owner),
                        {'compile':Adapter('compile','local',2,False,lambda x:x,policy_id='source-envelope-v2')},
                        clock=lambda:10)
        other.genesis(self.manifest,raw(self.agent),'new',
                      self.owner.sign(other.genesis_payload(self.manifest,raw(self.agent),'new')))
        action=self.action()
        with self.assertRaisesRegex(ValueError,'signature'):
            other.execute(action,self.agent.sign(self.runtime.action_payload(action)))
        with self.assertRaisesRegex(ValueError,'configuration changed'):
            Runtime(self.path,raw(self.owner),
                    {'compile':Adapter('compile','local',2,False,lambda x:x,policy_id='changed')},
                    clock=lambda:10)

    def test_trusted_adapter_policy_violation_retires_and_preserves_source(self):
        from agent_runtime import PolicyViolation, digest
        def reject(_):
            raise PolicyViolation('forbidden recipient')
        runtime=Runtime(self.path,raw(self.owner),
                        {'compile':Adapter('compile','local',2,False,reject)},clock=lambda:10)
        action=self.action(artifact=b'forbidden source')
        with self.assertRaises(PolicyViolation):
            runtime.execute(action,self.agent.sign(runtime.action_payload(action)))
        self.assertEqual(runtime.snapshot('root')['status'],'retired')
        self.assertEqual(runtime.evidence(digest(runtime.action_payload(action)))['artifact'],b'forbidden source')
        self.assertIn('retired',runtime.events()[-1]['payload'])

    def test_concurrent_child_admission_cannot_oversubscribe(self):
        from concurrent.futures import ThreadPoolExecutor
        from agent_admission_signed import SignedApproval, approval_payload
        def birth(i):
            key=Ed25519PrivateKey.generate()
            child=AgentManifest('child-'+str(i),'root',frozenset({'compile'}),frozenset({'local'}),
                                6,90,('root',),0)
            nonce='birth-'+str(i)
            approval=SignedApproval(nonce,80,self.agent.sign(
                approval_payload(child,self.manifest,nonce=nonce,expires_at=80)))
            signature=self.owner.sign(self.runtime.birth_payload(child,raw(key),approval))
            try:
                self.runtime.admit(child,raw(key),approval,signature)
                return 'born'
            except ValueError:
                return 'denied'
        with ThreadPoolExecutor(max_workers=2) as pool:
            results=list(pool.map(birth,range(2)))
        self.assertCountEqual(results,['born','denied'])
        self.assertEqual(self.runtime.snapshot('root')['remaining'],4)

    def test_admission_nonce_consumed_across_distinct_child_manifests(self):
        self.admit_child('child-one',2,'same-approval-nonce')
        with self.assertRaisesRegex(ValueError,'nonce already consumed'):
            self.admit_child('child-two',2,'same-approval-nonce')
        self.assertEqual(self.runtime.snapshot('root')['remaining'],8)

    def test_oversized_artifact_refused_without_state_change(self):
        from agent_runtime import MAX_ARTIFACT_BYTES
        with self.assertRaisesRegex(ValueError,'artifact'):
            self.runtime.action_payload(self.action(artifact=b'x'*(MAX_ARTIFACT_BYTES+1)))
        self.assertEqual(self.runtime.snapshot('root')['remaining'],10)

    def test_birth_evidence_contains_exact_verifiable_payload(self):
        import json
        event = json.loads(self.runtime.events()[0]['payload'])
        signed_payload = bytes.fromhex(event['details']['payload_hex'])
        self.owner.public_key().verify(bytes.fromhex(event['details']['approval']), signed_payload)
        self.assertEqual(json.loads(signed_payload)['runtime_id'], self.runtime.runtime_id)
        self.assertEqual(json.loads(signed_payload)['nonce'], 'birth')
        self.admit_child()
        event = json.loads(self.runtime.events()[-1]['payload'])
        self.owner.public_key().verify(bytes.fromhex(event['details']['owner_signature']),
                                       bytes.fromhex(event['details']['payload_hex']))

    def test_missing_runtime_identity_fails_closed(self):
        import json
        import sqlite3
        with sqlite3.connect(self.path) as db:
            config=json.loads(db.execute('SELECT value FROM config').fetchone()[0])
            config.pop('runtime_id')
            db.execute('UPDATE config SET value=?',(json.dumps(config).encode(),))
        with self.assertRaisesRegex(ValueError,'identity missing'):
            Runtime(self.path,raw(self.owner),self.adapters,clock=lambda:10)

    def test_revoked_registered_juror_cannot_vote_even_under_key_alias(self):
        from dataclasses import replace
        from agent_runtime import JuryVote, digest, vote_payload
        jurors={f'j{i}':Ed25519PrivateKey.generate() for i in range(3)}
        adapters={'reviewed':Adapter('compile','local',1,True,lambda x:b'ok')}
        runtime=Runtime(Path(self.temp.name)/'jury-revocation.sqlite',raw(self.owner),adapters,
                        jury_keys={i:raw(k) for i,k in jurors.items()},clock=lambda:10)
        runtime.genesis(self.manifest,raw(self.agent),'root-birth',
                        self.owner.sign(runtime.genesis_payload(self.manifest,raw(self.agent),'root-birth')))
        juror_manifest=replace(self.manifest,agent_id='registered-reviewer')
        runtime.genesis(juror_manifest,raw(jurors['j2']),'juror-birth',
                        self.owner.sign(runtime.genesis_payload(juror_manifest,raw(jurors['j2']),'juror-birth')))
        runtime.revoke('registered-reviewer','jury misconduct')
        action=self.action(adapter='reviewed')
        h=digest(runtime.action_payload(action))
        votes=[JuryVote(i,True,40,k.sign(vote_payload(h,i,True,40))) for i,k in jurors.items()]
        signature=self.agent.sign(runtime.action_payload(action))
        with self.assertRaisesRegex(ValueError,'inactive'):
            runtime.execute(action,signature,votes[1:])
        self.assertEqual(runtime.execute(action,signature,votes[:2]),b'ok')

    def test_descendant_key_alias_cannot_approve_parent(self):
        from agent_admission_signed import SignedApproval, approval_payload
        from agent_runtime import JuryVote, digest, vote_payload
        child_key = Ed25519PrivateKey.generate()
        reviewers = {'child-alias': child_key,
                     'independent-one': Ed25519PrivateKey.generate(),
                     'independent-two': Ed25519PrivateKey.generate(),
                     'independent-three': Ed25519PrivateKey.generate()}
        adapters = {'reviewed': Adapter('compile', 'local', 1, True, lambda _: b'reviewed')}
        runtime = Runtime(Path(self.temp.name)/'descendant-jury.sqlite', raw(self.owner), adapters,
                          jury_keys={i: raw(k) for i,k in reviewers.items()}, clock=lambda:10)
        runtime.genesis(self.manifest,raw(self.agent),'birth',
                        self.owner.sign(runtime.genesis_payload(self.manifest,raw(self.agent),'birth')))
        child = AgentManifest('child', 'root', frozenset({'compile'}), frozenset({'local'}),
                              2, 90, ('root',), 0)
        approval = SignedApproval('child',80,self.agent.sign(
            approval_payload(child,self.manifest,nonce='child',expires_at=80)))
        runtime.admit(child,raw(child_key),approval,
                      self.owner.sign(runtime.birth_payload(child,raw(child_key),approval)))
        identities, keys = runtime.review_exclusions('root')
        self.assertIn('child', identities)
        self.assertIn(raw(child_key), keys)
        action = self.action(adapter='reviewed')
        h = digest(runtime.action_payload(action))
        votes = [JuryVote(i,True,40,k.sign(vote_payload(h,i,True,40)))
                 for i,k in reviewers.items()]
        signature = self.agent.sign(runtime.action_payload(action))
        with self.assertRaisesRegex(ValueError,'ineligible'):
            runtime.execute(action,signature,votes[:2])
        self.assertEqual(runtime.execute(action,signature,votes[1:3]),b'reviewed')

    def test_process_exit_after_reservation_remains_pending(self):
        import multiprocessing
        import os
        from agent_runtime import digest
        if 'fork' not in multiprocessing.get_all_start_methods():
            self.skipTest('requires fork-capable host')
        action = self.action(nonce='process-crash')
        signature = self.agent.sign(self.runtime.action_payload(action))
        def child():
            def die(_):
                os._exit(71)
            runtime = Runtime(self.path,raw(self.owner),
                              {'compile':Adapter('compile','local',2,False,die)},clock=lambda:10)
            runtime.execute(action,signature)
        process = multiprocessing.get_context('fork').Process(target=child)
        process.start()
        process.join(timeout=10)
        if process.is_alive():
            process.terminate()
            process.join()
            self.fail('crash process did not exit')
        self.assertEqual(process.exitcode,71)
        restarted = Runtime(self.path,raw(self.owner),self.adapters,clock=lambda:10)
        self.assertEqual(restarted.evidence(digest(restarted.action_payload(action)))['status'],'pending')
        self.assertEqual(restarted.snapshot('root')['remaining'],8)
        with self.assertRaisesRegex(ValueError,'already reserved'):
            restarted.execute(action,signature)
        self.assertEqual(self.calls,[])


if __name__ == '__main__':
    unittest.main()

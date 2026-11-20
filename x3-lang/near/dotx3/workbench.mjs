const $ = id => document.getElementById(id);
let original = '', last = null, busy = false;
const controls = ['compile', 'attack', 'invalid', 'reset', 'restore'];
async function request(path, body) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), path.includes('dry') ? 120000 : 15000);
  try {
    const response = await fetch(path, { method: body ? 'POST' : 'GET',
      headers: body ? { 'Content-Type': 'application/json' } : {},
      body: body ? JSON.stringify(body) : undefined, signal: controller.signal });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || 'Request failed');
    return result;
  } finally { clearTimeout(timer); }
}
function source() { return { program: $('program').value, mandate: $('mandate').value }; }
function lock(value) { busy = value; controls.forEach(id => $(id).disabled = value); ['program', 'mandate'].forEach(id => $(id).readOnly = value); }
function clearReport() {
  last = null; $('download').disabled = true; $('decision').textContent = 'Source changed — compile again';
  $('decision').dataset.tone = ''; $('checks').replaceChildren(); $('report').textContent = '{}';
  ['program-hash', 'mandate-hash'].forEach(id => $(id).textContent = '—');
  $('evidence').textContent = 'Unverified';
  $('result-description').textContent = 'Previous decisions do not apply to the edited source.';
  $('dry-report').hidden = true;
}
function show(report) {
  last = { ...report, source: source(), purpose: 'review_only_not_wallet_authorization' };
  $('download').disabled = false;
  const states = {
    preflight_passed: ['Compiles. Local policy passes.', 'pass'],
    denied_by_policy: ['Compiles. Owner policy refuses.', 'fail'],
    approval_required: ['Compiles. Owner approval required.', ''],
    compile_error: ['Compiler refused the program.', 'fail'],
    mandate_error: ['Owner mandate could not be evaluated.', 'fail'],
  };
  const [title, tone] = states[report.decision] || ['Unverified result', 'fail'];
  $('decision').textContent = title; $('decision').dataset.tone = tone;
  $('program-hash').textContent = report.program_sha256;
  $('mandate-hash').textContent = report.mandate_sha256;
  $('evidence').textContent = report.evidence_level;
  $('result-description').textContent = report.error ? `${report.error.code}: ${report.error.message}` :
    'Local evaluation only. No signed quote, on-chain authorization, wallet signature or settlement is established.';
  $('report').textContent = JSON.stringify(report, null, 2);
  $('checks').replaceChildren();
  const rows = [{ guard: 'python_frontend_compiled', ok: report.compiled }, ...(report.policy?.checks || [])];
  for (const row of rows) {
    const div = document.createElement('div'); div.className = 'check';
    const label = document.createElement('span'); label.textContent = row.guard;
    const status = document.createElement('strong'); status.className = row.ok ? 'pass' : 'fail';
    status.textContent = row.ok ? 'PASS' : 'REFUSED'; div.append(label, status); $('checks').append(div);
  }
}
async function compile() {
  if (busy) return;
  lock(true); $('decision').textContent = 'Compiling .x3…'; $('download').disabled = true;
  try { show(await request('/api/compile', source())); }
  catch (error) { clearReport(); $('decision').textContent = 'Compiler unavailable'; $('result-description').textContent = error.message; }
  finally { lock(false); }
}
async function load() {
  lock(true);
  try {
    const example = await request('/api/examples');
    $('program').value = original = example.program; $('mandate').value = example.mandate;
    $('source-label').textContent = example.label; clearReport();
  } catch (error) { $('source-label').textContent = `Examples unavailable: ${error.message}`; }
  finally { lock(false); }
}
$('compile').addEventListener('click', compile);
$('reset').addEventListener('click', load);
$('restore').addEventListener('click', () => { $('program').value = original; clearReport(); });
$('attack').addEventListener('click', () => {
  const text = $('program').value;
  const changed = text.replace(/(to\s+Near\.USDC\s+receiver\s+)\S+/, '$1attacker.near');
  if (changed === text) { $('result-description').textContent = 'No matching NEAR USDC recipient found. Edit the source directly.'; return; }
  $('program').value = changed; clearReport(); compile();
});
$('invalid').addEventListener('click', () => {
  $('program').value = $('program').value.replace(/^\s*require nonce unused[^\n]*\n/m, '\n'); clearReport(); compile();
});
['program', 'mandate'].forEach(id => $(id).addEventListener('input', clearReport));
$('download').addEventListener('click', () => {
  if (!last) return;
  const url = URL.createObjectURL(new Blob([JSON.stringify(last, null, 2) + '\n'], { type: 'application/json' }));
  const a = document.createElement('a'); a.href = url; a.download = 'dotx3-owner-review.json'; a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
});
$('dry').addEventListener('click', async () => {
  $('dry').disabled = true; lock(true); $('dry-status').textContent = 'Checking public mainnet data and dry quote…';
  try {
    const report = await request('/api/dry-preflight', { ...source(), max_input_usdc: $('budget').value.trim(),
      guard: $('guard').value.trim(), expected_guard_code_hash: $('code-hash').value.trim() });
    $('dry-report').hidden = false; $('dry-report').textContent = JSON.stringify(report, null, 2);
    $('dry-status').textContent = 'Read-only report complete. This cannot authorize a wallet or establish settlement.';
  } catch (error) { $('dry-status').textContent = error.message; }
  finally { $('dry').disabled = false; lock(false); }
});
function lawRow(label, value) {
  const div = document.createElement('div');
  const name = document.createElement('span');
  name.textContent = label;
  const status = document.createElement('strong');
  status.textContent = value;
  div.append(name, status);
  return div;
}
function showLaw(report) {
  const status = $('law-status');
  const chain = $('law-chain');
  const results = $('law-results');
  const limits = $('law-limits');
  chain.replaceChildren();
  results.replaceChildren();
  limits.replaceChildren();
  const claimsAuthority = !report
    || report.settlement === true
    || report.settlement_proven === true
    || report.broadcast === true
    || report.wallet_authorized === true
    || (typeof report.solana_custody === 'string' && report.solana_custody !== 'outside_this_path');
  const limited = report
    && report.settlement === false
    && report.settlement_proven === false
    && report.broadcast === false
    && report.wallet_authorized === false
    && report.solana_custody === 'outside_this_path';
  if (claimsAuthority || (report.ok === true && !limited)) {
    status.dataset.tone = 'fail';
    status.textContent = 'This screen will not show a path that claims settlement, a broadcast, or Solana custody.';
    return;
  }
  if (!limited) {
    status.dataset.tone = 'fail';
    status.textContent = 'The local path did not pass. No receipt was authorized from this screen.';
    return;
  }
  status.dataset.tone = report.ok ? 'pass' : 'fail';
  status.textContent = report.ok
    ? 'Local path passed. The receipt was not broadcast, and this intent is still not settled.'
    : 'The local path did not pass. No receipt was authorized from this screen.';
  for (const stage of report.architecture || []) {
    const item = document.createElement('li');
    item.textContent = stage;
    if ((report.stages_seen || []).includes(stage)) item.className = 'seen';
    chain.append(item);
  }
  const rows = [
    ['Permitted X3 call', String(report.permitted)],
    ['Forbidden bypass', report.forbidden_reason],
    ['Outside route', report.outside_reason],
    ['After revocation', report.revoked_reason],
    ['Court receipts', String(report.court_receipts)],
    ['Spent', String(report.spent)],
    ['Receipt', `${report.receipt_method} · ${report.receipt_amount} · ${report.receipt_recipient}`],
    ['Broadcast', String(report.broadcast)],
    ['Solana custody', report.solana_custody],
    ['Agent self-judgment', report.self_judgment],
    ['Jury reversal', report.jury],
    ['Birth mandate', report.commandments_sha256],
  ];
  for (const [label, value] of rows) {
    if (value === undefined || value === 'undefined' || value === 'null') continue;
    results.append(lawRow(label, value));
  }
  for (const item of report.limits || []) {
    const line = document.createElement('li');
    line.textContent = item;
    limits.append(line);
  }
}
const lawPromise = request('/api/agent-law');
try {
  const health = await request('/api/health');
  $('connection').textContent = 'Python .x3 frontend connected · signing disabled';
  $('dry').disabled = !health.mainnet_readonly;
  if (health.mainnet_readonly) $('dry-status').textContent = 'Supply reviewed public inputs. Only dry quotes and read-only chain queries are available.';
  await load();
} catch (error) { $('connection').textContent = `Backend unavailable: ${error.message}`; }
try { showLaw(await lawPromise); }
catch (error) {
  $('law-status').dataset.tone = 'fail';
  $('law-status').textContent = `Local path unavailable: ${error.message}`;
}
function showDoor(report) {
  const status = $('door-status');
  const results = $('door-results');
  const limits = $('door-limits');
  results.replaceChildren();
  limits.replaceChildren();
  const claimsAuthority = !report
    || report.settlement === true
    || report.settlement_proven === true
    || report.broadcast === true
    || report.wallet_authorized === true
    || (report.receipt && report.receipt.broadcast === true)
    || (typeof report.solana_custody === 'string' && report.solana_custody !== 'outside_this_path');
  if (claimsAuthority) {
    status.dataset.tone = 'fail';
    status.textContent = 'This door will not show a session that claims settlement, a broadcast, or Solana custody.';
    return;
  }
  const limited = report.settlement === false
    && report.settlement_proven === false
    && report.broadcast === false
    && report.wallet_authorized === false
    && report.solana_custody === 'outside_this_path';
  if (!limited) {
    status.dataset.tone = 'fail';
    status.textContent = 'The door returned an incomplete result. No receipt was shown.';
    return;
  }
  status.dataset.tone = report.ok ? 'pass' : 'fail';
  const lines = {
    birth: 'Born into a process-local session. The archived invoice_06 run above is unchanged.',
    test: 'This exact source passed the isolated test. That is not a receipt.',
    authorize: 'Local receipt recorded. It was not broadcast, and this intent is still not settled.',
    revoke: 'Revoked. The next call stops, and the archived exhibit stays as it was.',
  };
  const refusals = {
    not_tested: 'Refused: this exact source has not passed the isolated test. No receipt was recorded.',
    mandate_recipient: 'Refused: the mandate does not allow this recipient. No receipt was recorded.',
    revoked: 'Refused: revocation stops this call. The earlier receipt stays.',
    no_birth: 'Birth from this mandate comes first.',
  };
  status.textContent = report.ok
    ? (lines[report.act] || 'The local door accepted this step.')
    : (refusals[report.reason] || `Refused: ${report.reason || 'the door refused this step'}. No settlement was created.`);
  const receipt = report.receipt || {};
  const rows = [
    ['Act', report.act],
    ['Reason', report.reason],
    ['Status', report.status],
    ['Spent', String(report.spent)],
    ['Court receipts', String(report.court_receipts)],
    ['Receipt', receipt.method ? `${receipt.method} · ${receipt.amount} · ${receipt.recipient}` : ''],
    ['Broadcast', String(report.broadcast)],
    ['Solana custody', report.solana_custody],
    ['Agent', report.agent_id],
  ];
  for (const [label, value] of rows) {
    if (value === undefined || value === 'undefined' || value === 'null' || value === '') continue;
    results.append(lawRow(label, value));
  }
  for (const item of report.limits || []) {
    const line = document.createElement('li');
    line.textContent = item;
    limits.append(line);
  }
}
const doorControls = ['law-birth', 'law-test', 'law-authorize', 'law-revoke'];
for (const id of doorControls) {
  $(id).addEventListener('click', async () => {
    const act = id.slice('law-'.length);
    doorControls.forEach(control => $(control).disabled = true);
    try { showDoor(await request('/api/agent-law/act', { act, ...source() })); }
    catch (error) {
      $('door-status').dataset.tone = 'fail';
      $('door-status').textContent = error.message;
    } finally {
      doorControls.forEach(control => $(control).disabled = false);
    }
  });
}

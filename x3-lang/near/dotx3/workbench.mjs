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
try {
  const health = await request('/api/health');
  $('connection').textContent = 'Python .x3 frontend connected · signing disabled';
  $('dry').disabled = !health.mainnet_readonly;
  if (health.mainnet_readonly) $('dry-status').textContent = 'Supply reviewed public inputs. Only dry quotes and read-only chain queries are available.';
  await load();
} catch (error) { $('connection').textContent = `Backend unavailable: ${error.message}`; }

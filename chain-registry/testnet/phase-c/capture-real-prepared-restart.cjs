// Observe an actual prepared round, crash only V1, and restart the same container.
// No journal manipulation, fabricated traffic, or consensus configuration changes.
const fs = require('node:fs');
const { spawnSync } = require('node:child_process');
const [evidencePath, targetHeightRaw, imageManifestPath] = process.argv.slice(2);
if (!evidencePath) throw new Error('Usage: node capture-real-prepared-restart.cjs EVIDENCE_JSON HEIGHT');
const targetHeight = Number(targetHeightRaw);
if (!Number.isSafeInteger(targetHeight) || targetHeight < 1) throw new Error('Actual next height required');
const api = 'http://127.0.0.1:18080';
const container = 'creg-phase-c-r7-v1';
const report = { schema: 'creg-real-prepared-crash-recovery-v1', target_height: targetHeight,
  container, no_journal_manipulation: true, public_cutover: false, passed: false };
const save = () => fs.writeFileSync(evidencePath, JSON.stringify(report, null, 2) + '\n');
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function docker(...args) {
  const result = spawnSync('docker', args, { encoding: 'utf8', timeout: 30_000, windowsHide: true });
  if (result.status !== 0) throw new Error(`Docker ${args[0]} failed: ${result.stderr || result.error}`);
  return result.stdout.trim();
}
async function read(path) {
  const response = await fetch(api + path, { signal: AbortSignal.timeout(3000) });
  if (!response.ok) throw new Error(`HTTP ${response.status}: ${path}`);
  return response.json();
}
let restartRequired = false;
(async () => {
  const image = docker('inspect', container, '--format', '{{.Image}}');
  let acceptedImages = ['sha256:dd07eef4a657707fdad696b1b4cc7397ee2589b82e3511c892ff3a806f3dcb03',
    'sha256:cee77b5210ad67a91f5d6e5ccbf2b5ec914ccf53268f8294b261bc7a37c7f49a'];
  if (imageManifestPath) {
    const manifest = JSON.parse(fs.readFileSync(imageManifestPath, 'utf8').replace(/^\uFEFF/, ''));
    if (manifest.image_tag !== 'creg-node:phase-c-secure-r20-durable-revocation-delivery' ||
        manifest.compilation !== 'passed') throw new Error('Reviewed R20 build manifest required');
    acceptedImages = manifest.accepted_image_ids;
  }
  if (!acceptedImages.includes(image)) {
    throw new Error('Live V1 must be the accepted reviewed image');
  }
  report.image_id = image;
  report.before = await read('/v1/public/chain/stats');
  if (report.before.tip_height !== targetHeight - 1 || report.before.validator_count !== 4 ||
      report.before.chain_id !== 'creg-phase-c-testnet-2026-09-26-r7') throw new Error('Wrong real baseline');
  report.armed_at_utc = new Date().toISOString(); save();
  console.log(`Armed for an actual prepared h${targetHeight} round on V1`);
  const deadline = Date.now() + 360_000;
  while (Date.now() < deadline) {
    const pbft = await read('/v1/consensus/pbft');
    const round = pbft.rounds.find(r => r.block_height === targetHeight && r.phase === 'commit' &&
      r.prepare_count >= r.quorum && r.commit_count >= 1 && r.commit_count < r.quorum);
    if (!round) {
      if ((await read('/v1/public/chain/stats')).tip_height >= targetHeight) throw new Error('Prepared crash window missed; no process stopped');
      await delay(50); continue;
    }
    report.observed_at_utc = new Date().toISOString(); report.prepared_round = round;
    report.pre_crash_pbft = pbft; save();
    restartRequired = true;
    docker('kill', '--signal', 'SIGKILL', container);
    report.crashed_at_utc = new Date().toISOString();
    if (docker('inspect', container, '--format', '{{.State.Status}}') !== 'exited') throw new Error('V1 did not exit');
    report.confirmed_exited = true; save();
    docker('start', container); restartRequired = false;
    report.restarted_at_utc = new Date().toISOString(); save();
    console.log(`V1 restarted after actual h${targetHeight} prepared/COMMIT crash`);
    break;
  }
  if (!report.confirmed_exited) throw new Error('No prepared round observed before deadline');
  const recoveryDeadline = Date.now() + 360_000;
  while (Date.now() < recoveryDeadline) {
    try {
      report.after = await read('/v1/public/chain/stats');
      report.after_pbft = await read('/v1/consensus/pbft');
      if (report.after.tip_height >= targetHeight) break;
    } catch { /* startup is measured against the bounded recovery deadline */ }
    await delay(1000);
  }
  const logs = docker('logs', '--since', report.crashed_at_utc, container);
  // docker logs also uses stderr; collect both without exposing private environment.
  const result = spawnSync('docker', ['logs', '--since', report.crashed_at_utc, container],
    { encoding: 'utf8', timeout: 30_000, windowsHide: true });
  const text = (logs + '\n' + (result.stderr || '')).replace(/\u001b\[[0-9;]*m/g, '');
  const recoveryLines = text.split(/\r?\n/).filter(line =>
    line.includes('Recovered durable PBFT prepared round') || line.includes('Recovered durable local PBFT COMMIT'));
  report.recovery_log_lines = recoveryLines;
  report.checks = {
    recovered_prepared_round: recoveryLines.some(line => line.includes('Recovered durable PBFT prepared round') && new RegExp(`height[= ]${targetHeight}(?:\\D|$)`).test(line)),
    recovered_local_commit: recoveryLines.some(line => line.includes('Recovered durable local PBFT COMMIT') && new RegExp(`height[= ]${targetHeight}(?:\\D|$)`).test(line)),
    original_prepared_hash_finalized: report.after?.tip_height === targetHeight && report.after.tip_hash === report.prepared_round.block_hash,
    same_image: docker('inspect', container, '--format', '{{.Image}}') === image,
    running: docker('inspect', container, '--format', '{{.State.Status}}') === 'running',
  };
  report.passed = Object.values(report.checks).every(Boolean); save();
  if (!report.passed) throw new Error('Prepared crash acceptance remains incomplete; inspect saved evidence');
  console.log('Actual prepared round and local COMMIT recovered; original block finalized');
})().catch(error => { report.error = error.message; save(); console.error(error.message); process.exitCode = 1; })
  .finally(() => { if (restartRequired) { try { docker('start', container); report.emergency_restart = true; save(); } catch (error) { console.error(error.message); } } });

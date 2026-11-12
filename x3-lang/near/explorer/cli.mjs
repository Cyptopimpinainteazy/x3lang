import { createExplorerClient } from './client.mjs';
try {
  const input = new URLSearchParams(process.argv[2] || '');
  console.log(JSON.stringify(await createExplorerClient().transactions(input), null, 2));
} catch (error) {
  console.error(JSON.stringify({ error: error.code || 'EXPLORER_UNAVAILABLE', retryAfter: error.retryAfter || 0 }));
  process.exitCode = 1;
}

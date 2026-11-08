// NEAR JSON-RPC over fetch: timeouts on every call, exponential backoff on 429, 5xx and
// network errors, rotating through the given endpoints.

const DEFAULT_TIMEOUT_MS = 10_000;
const DEFAULT_ATTEMPTS = 5;
const BACKOFF_BASE_MS = 500;

export class RpcError extends Error {
  constructor(message, data) {
    super(message);
    this.data = data;
  }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export function makeRpc(urls, { timeoutMs = DEFAULT_TIMEOUT_MS, attempts = DEFAULT_ATTEMPTS, fetchImpl = fetch } = {}) {
  if (!urls.length) throw new Error('no RPC endpoints');

  async function call(method, params, { timeout = timeoutMs } = {}) {
    let last;
    for (let i = 0; i < attempts; i++) {
      const url = urls[i % urls.length];
      try {
        const res = await fetchImpl(url, {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ jsonrpc: '2.0', id: 'relayer', method, params }),
          signal: AbortSignal.timeout(timeout),
        });
        if (res.status === 429 || res.status >= 500) {
          last = new RpcError(`HTTP ${res.status} from ${new URL(url).host}`);
        } else {
          const body = await res.json();
          if (body.error) {
            // A node-side timeout is worth retrying; anything else is the answer.
            const name = body.error.cause?.name;
            if (name !== 'TIMEOUT_ERROR' && name !== 'TOO_MANY_REQUESTS') {
              throw new RpcError(name ?? body.error.message ?? 'rpc error', body.error);
            }
            last = new RpcError(name, body.error);
          } else {
            return body.result;
          }
        }
      } catch (e) {
        if (e instanceof RpcError && e.data && !['TIMEOUT_ERROR', 'TOO_MANY_REQUESTS'].includes(e.data.cause?.name)) throw e;
        last = e;
      }
      await sleep(BACKOFF_BASE_MS * 2 ** i);
    }
    throw last;
  }

  return {
    call,
    async view(accountId, methodName, args) {
      const r = await call('query', {
        request_type: 'call_function',
        finality: 'final',
        account_id: accountId,
        method_name: methodName,
        args_base64: Buffer.from(JSON.stringify(args)).toString('base64'),
      });
      return JSON.parse(Buffer.from(r.result).toString('utf8'));
    },
    viewAccessKey: (accountId, publicKey) =>
      call('query', { request_type: 'view_access_key', finality: 'final', account_id: accountId, public_key: publicKey }),
    viewAccount: (accountId) => call('query', { request_type: 'view_account', finality: 'final', account_id: accountId }),
    finalBlock: () => call('block', { finality: 'final' }),
    // The same signed bytes on every retry: a resend can only ever land once.
    sendTx: (signedBase64, waitUntil = 'INCLUDED') =>
      call('send_tx', { signed_tx_base64: signedBase64, wait_until: waitUntil }, { timeout: 30_000 }),
  };
}

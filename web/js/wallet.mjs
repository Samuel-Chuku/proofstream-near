// The one third-party library: @hot-labs/near-connect 0.12.0, vendored (vendor/README.md).
// Loaded only when someone connects, so read-only pages never fetch wallet code.
import { NETWORK } from './config.mjs';

let connector;
let account = null;
const listeners = new Set();

async function load() {
  if (connector) return connector;
  const { NearConnector } = await import('../vendor/near-connect-0.12.0.mjs');
  connector = new NearConnector({ network: NETWORK, features: { signAndSendTransactions: true, testnet: true } });
  connector.on('wallet:signIn', (t) => { account = t.accounts?.[0]?.accountId ?? null; listeners.forEach((f) => f(account)); });
  connector.on('wallet:signOut', () => { account = null; listeners.forEach((f) => f(null)); });
  try {
    const w = await connector.wallet();
    const accounts = await w.getAccounts();
    account = accounts?.[0]?.accountId ?? null;
  } catch { /* not signed in yet */ }
  return connector;
}

export function onAccount(f) {
  listeners.add(f);
  f(account);
}

/// Restores a session from an earlier visit, if the wallet kept one.
export async function restore() {
  await load().catch(() => null);
  listeners.forEach((f) => f(account));
  return account;
}

export async function connect() {
  const c = await load();
  await c.connect();
  return account;
}

export async function disconnect() {
  if (!connector) return;
  const w = await connector.wallet().catch(() => null);
  await w?.signOut?.();
  account = null;
  listeners.forEach((f) => f(null));
}

export const current = () => account;

/// Signs and sends transactions, in order, in one wallet prompt where the wallet allows.
export async function send(transactions) {
  const c = await load();
  const w = await c.wallet();
  const shaped = transactions.map(({ receiverId, calls }) => ({
    receiverId,
    actions: calls.map(({ method, args, gas, deposit }) => ({ type: 'FunctionCall', params: { methodName: method, args, gas, deposit } })),
  }));
  if (shaped.length === 1) return [await w.signAndSendTransaction(shaped[0])];
  return w.signAndSendTransactions({ transactions: shaped });
}

/// The hash of a sent transaction, whatever shape the wallet answered in.
export const hashOf = (outcome) => outcome?.transaction?.hash ?? outcome?.transaction_outcome?.id ?? null;

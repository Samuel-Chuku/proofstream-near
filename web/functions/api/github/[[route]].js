// GitHub sign-in for the funder's form, as a Cloudflare Pages Function on the site's own origin.
// It does what the Arc site's /api/github routes do: trade GitHub's code for a user token with
// the App's client secret, keep that token in an AES-GCM sealed cookie (no database), and list
// granted repositories and branches, or create a branch. The secret and the token never reach
// the browser. Web Crypto only; no packages.
//
// Secrets (Pages project settings): GITHUB_APP_CLIENT_ID, GITHUB_APP_CLIENT_SECRET,
// SESSION_SECRET. Plain: GITHUB_APP_SLUG (default "proofstream").

const GITHUB = 'https://api.github.com';
const SESSION = 'ps_gh';
const STATE = 'ps_gh_state';
const NEXT = 'ps_gh_next';
const STATE_TTL_S = 600;
const SESSION_TTL_S = 8 * 3600;
const REPO = /^[A-Za-z0-9-]{1,39}\/[A-Za-z0-9._-]{1,100}$/;
const BRANCH = /^(?!.*\.\.)(?!\/)(?!.*\/$)[A-Za-z0-9._\/-]{1,100}$/;
const NEXT_PATH = /^\/(?!\/)[A-Za-z0-9._\-\/?=&#%]*$/;

const enc = new TextEncoder();
const b64url = (bytes) => btoa(String.fromCharCode(...new Uint8Array(bytes))).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
const unb64url = (s) => Uint8Array.from(atob(s.replace(/-/g, '+').replace(/_/g, '/') + '==='.slice((s.length + 3) % 4)), (c) => c.charCodeAt(0));

async function keys(secret) {
  const raw = await crypto.subtle.digest('SHA-256', enc.encode(secret));
  return {
    aes: await crypto.subtle.importKey('raw', raw, 'AES-GCM', false, ['encrypt', 'decrypt']),
    mac: await crypto.subtle.importKey('raw', raw, { name: 'HMAC', hash: 'SHA-256' }, false, ['sign', 'verify']),
  };
}

export async function seal(k, value) {
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const body = await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, k.aes, enc.encode(JSON.stringify(value)));
  return b64url(new Uint8Array([...iv, ...new Uint8Array(body)]));
}

export async function unseal(k, sealed) {
  if (!sealed) return null;
  try {
    const raw = unb64url(sealed);
    const out = await crypto.subtle.decrypt({ name: 'AES-GCM', iv: raw.slice(0, 12) }, k.aes, raw.slice(12));
    return JSON.parse(new TextDecoder().decode(out));
  } catch { return null; } // wrong key, tampered or garbage: not signed in
}

const sign = async (k, nonce) => b64url(await crypto.subtle.sign('HMAC', k.mac, enc.encode(nonce)));

async function stateValid(k, state, nonceCookie) {
  if (!state || !nonceCookie) return false;
  const [nonce, sig] = state.split('.');
  if (nonce !== nonceCookie || !sig) return false;
  try { return await crypto.subtle.verify('HMAC', k.mac, unb64url(sig), enc.encode(nonce)); } catch { return false; }
}

function cookies(request) {
  const out = {};
  for (const part of (request.headers.get('Cookie') || '').split(';')) {
    const i = part.indexOf('=');
    if (i > 0) out[part.slice(0, i).trim()] = decodeURIComponent(part.slice(i + 1).trim());
  }
  return out;
}

const cookie = (name, value, maxAge) => `${name}=${encodeURIComponent(value)}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=${maxAge}`;
const json = (body, status = 200, headers = {}) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json', 'Cache-Control': 'no-store', ...headers } });
const safeNext = (n) => (n && NEXT_PATH.test(n) ? n : null);

function redirect(location, setCookies = []) {
  const headers = new Headers({ Location: location, 'Cache-Control': 'no-store' });
  for (const c of setCookies) headers.append('Set-Cookie', c);
  return new Response(null, { status: 302, headers });
}

async function gh(path, token, init = {}) {
  const res = await fetch(`${GITHUB}${path}`, {
    ...init,
    headers: { Authorization: `Bearer ${token}`, Accept: 'application/vnd.github+json', 'X-GitHub-Api-Version': '2022-11-28', 'User-Agent': 'proofstream-near', ...(init.body ? { 'Content-Type': 'application/json' } : {}) },
  });
  if (!res.ok) throw Object.assign(new Error(`github ${res.status}`), { status: res.status });
  return res.json();
}

export async function onRequest({ request, env, params }) {
  const route = [].concat(params.route || []).join('/');
  if (!env.GITHUB_APP_CLIENT_ID || !env.GITHUB_APP_CLIENT_SECRET || !env.SESSION_SECRET) return json({ error: 'github_not_configured' }, 503);
  const url = new URL(request.url);
  const k = await keys(env.SESSION_SECRET);
  const jar = cookies(request);
  const session = await unseal(k, jar[SESSION]);
  const live = session && session.exp > Date.now() / 1000 ? session : null;
  const slug = env.GITHUB_APP_SLUG || 'proofstream';

  try {
    switch (`${request.method} ${route}`) {
      case 'GET login': {
        const nonce = b64url(crypto.getRandomValues(new Uint8Array(16)));
        const state = `${nonce}.${await sign(k, nonce)}`;
        const next = safeNext(url.searchParams.get('next')) || '/new.html';
        const target = url.searchParams.get('install') === '1'
          ? `https://github.com/apps/${slug}/installations/new?state=${encodeURIComponent(state)}`
          : `https://github.com/login/oauth/authorize?${new URLSearchParams({ client_id: env.GITHUB_APP_CLIENT_ID, redirect_uri: `${url.origin}/api/github/callback`, state })}`;
        return redirect(target, [cookie(STATE, nonce, STATE_TTL_S), cookie(NEXT, next, STATE_TTL_S)]);
      }
      case 'GET callback': {
        const next = safeNext(jar[NEXT]) || '/new.html';
        const code = url.searchParams.get('code');
        // After a bare install GitHub may return without a code: sign in properly now.
        if (!code) return redirect(`/api/github/login?next=${encodeURIComponent(next)}`);
        if (!(await stateValid(k, url.searchParams.get('state'), jar[STATE]))) return redirect(`${next}${next.includes('?') ? '&' : '?'}github=state_mismatch`);
        const res = await fetch('https://github.com/login/oauth/access_token', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json', Accept: 'application/json', 'User-Agent': 'proofstream-near' },
          body: JSON.stringify({ client_id: env.GITHUB_APP_CLIENT_ID, client_secret: env.GITHUB_APP_CLIENT_SECRET, code, redirect_uri: `${url.origin}/api/github/callback` }),
        });
        const body = await res.json();
        if (!body.access_token) return redirect(`${next}${next.includes('?') ? '&' : '?'}github=denied`);
        const user = await gh('/user', body.access_token);
        const exp = Math.floor(Date.now() / 1000) + Math.min(body.expires_in || SESSION_TTL_S, SESSION_TTL_S);
        const sealed = await seal(k, { token: body.access_token, login: user.login, id: user.id, exp });
        return redirect(next, [cookie(SESSION, sealed, exp - Math.floor(Date.now() / 1000)), cookie(STATE, '', 0), cookie(NEXT, '', 0)]);
      }
      case 'GET logout':
        return redirect(safeNext(url.searchParams.get('next')) || '/', [cookie(SESSION, '', 0)]);
      case 'GET me':
        return live ? json({ login: live.login, id: live.id }) : json({ error: 'not_signed_in' }, 401);
      case 'GET repos': {
        if (!live) return json({ error: 'not_signed_in' }, 401);
        const { installations } = await gh('/user/installations', live.token);
        const repos = [];
        for (const inst of installations) {
          const page = await gh(`/user/installations/${inst.id}/repositories?per_page=100`, live.token);
          for (const r of page.repositories) repos.push({ id: r.id, fullName: r.full_name, private: r.private, defaultBranch: r.default_branch });
        }
        repos.sort((a, b) => a.fullName.localeCompare(b.fullName));
        return json({ repos, installations: installations.map((i) => ({ id: i.id, account: i.account?.login ?? '', settingsUrl: `https://github.com/settings/installations/${i.id}` })) });
      }
      case 'GET branches': {
        if (!live) return json({ error: 'not_signed_in' }, 401);
        const repo = url.searchParams.get('repo') || '';
        if (!REPO.test(repo)) return json({ error: 'bad_repo' }, 400);
        const list = await gh(`/repos/${repo}/branches?per_page=100`, live.token);
        return json({ branches: list.map((b) => b.name).sort((a, b) => a.localeCompare(b)) });
      }
      case 'POST branch': {
        if (!live) return json({ error: 'not_signed_in' }, 401);
        // Only this site may ask: a form or script on another origin can't send this.
        if (request.headers.get('Origin') !== url.origin || !(request.headers.get('Content-Type') || '').startsWith('application/json')) return json({ error: 'forbidden' }, 403);
        const { repo, branch, from } = await request.json().catch(() => ({}));
        if (!REPO.test(repo || '') || !BRANCH.test(branch || '') || !BRANCH.test(from || '')) return json({ error: 'bad_request' }, 400);
        const ref = await gh(`/repos/${repo}/git/ref/heads/${encodeURIComponent(from)}`, live.token);
        await gh(`/repos/${repo}/git/refs`, live.token, { method: 'POST', body: JSON.stringify({ ref: `refs/heads/${branch}`, sha: ref.object.sha }) });
        return json({ ok: true, branch });
      }
      default:
        return json({ error: 'not_found' }, 404);
    }
  } catch (e) {
    return json({ error: e.status === 403 || e.status === 404 ? 'github_denied' : 'github_unavailable' }, e.status === 403 ? 403 : 502);
  }
}

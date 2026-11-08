# Relayer: deploy note

The relayer submits only `bind_payee`, `claim` and `review` to `proofstream-v1.testnet`, so a contributor without NEAR can still claim. It can refuse to send a request, but it can't redirect money: the gist binding fixes where the payout goes. Anyone with NEAR can call the contract directly instead.

## What runs

- `relayer`: Node 22 with no npm packages, from `Dockerfile`. It runs as the non-root `node` user, with a read-only filesystem, no Linux capabilities, `no-new-privileges` and 256 MB of memory. It's reachable only from the proxy.
- `proxy`: Nginx. It terminates TLS on 443 for `RELAYER_DOMAIN`, forwards only `/bind_payee`, `/claim`, `/review` and `/health`, caps bodies at 1 KiB and limits each IP to 30 requests a minute.
- **Limits in the relayer** (in memory, reset on restart):
  - 10 requests per IP per hour
  - 6 sends per milestone per hour
  - 10 sends per GitHub id per hour
  - `DAILY_CAP_MILLINEAR` of NEAR committed per UTC day
  - one send at a time, with 8 waiting at most

## Host (AWS)

1. **Security group:** inbound TCP 443 from anywhere, and TCP 22 from your IP only. Nothing else. Port 80 depends on D41.
2. **SSH:** key only. In `/etc/ssh/sshd_config`, set `PasswordAuthentication no` and `PermitRootLogin no`, then `sudo systemctl reload ssh`.
3. **Docker:** install Docker Engine and the Compose plugin from Docker's apt repository (docs.docker.com/engine/install/ubuntu).
4. **Certificate:** put it at `/etc/letsencrypt/live/$RELAYER_DOMAIN/{fullchain,privkey}.pem`. How it's issued is D41.

## Key (never in env, never in git)

The signing key's secret file holds only `private_key`. Never copy the seed phrase from `~/.near-credentials` to the server. Create the file on your PC and copy it over:

```bash
jq '{private_key}' ~/.near-credentials/testnet/proofstream-relayer.testnet.json > relayer_key.json
scp relayer_key.json <vps>:proofstream-near/relayer/secrets/relayer_key.json
shred -u relayer_key.json
```

Then on the VPS:

```bash
cd proofstream-near/relayer
# Compose mounts file secrets with their host owner and mode; uid 1000 is the container's `node` user.
sudo chown 1000:1000 secrets/relayer_key.json && sudo chmod 600 secrets/relayer_key.json
cp .env.example .env   # fill in ALLOWED_ORIGIN and RELAYER_DOMAIN
```

## Run

```bash
docker compose up -d --build
docker compose logs relayer        # expect one "listening" line
curl https://$RELAYER_DOMAIN/health
```

At startup the relayer checks that its key is a full-access key on `RELAYER_ACCOUNT_ID`. It refuses to start if OutLayer's current price is above the fixed deposits.

**Local run, before the VPS:** this is the same container, on 127.0.0.1:8080 with no proxy.

```bash
docker compose -f compose.yaml -f compose.local.yaml up --build relayer
```

## Before it spends anything (gate)

Run this on at least one real testnet transaction per method; each must match exactly:

```bash
node relayer/scripts/rehash.mjs <bind tx>:<signer> <claim tx>:<signer> <review tx>:<signer>
```

Then make one real `review` through the relayer.

## Logs

Logs are JSON lines: `sent` with the tx hash, `refused` with a reason, and `send_failed` with the tx hash. Never the key, the signed bytes or client IPs. Docker rotates them at 10 MB × 3.

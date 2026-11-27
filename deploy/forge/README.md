# X3 Agent Forge on forge.dotx3.net

Runs the Forge (`x3-lang/forge`) on this machine behind Cloudflare Access. It is kept separate
from `deploy/x3star`: its own tunnel `x3-forge`, its own Access app and owner-only policy
("X3 Agent Forge"), and port 7420. It can't change the x3star.net demos.

## First deploy (on x3star1)

Prerequisites, once:
- `node` 20.19+ or 22.12+ with `npm`, `python3` 3.10+ with `venv`, `curl`
- `cloudflared` logged in (`cloudflared tunnel login`)
- `dotx3.net` as an active zone in the same Cloudflare account
- a Cloudflare API token with **Access: Apps and Policies → Edit**,
  **Access: Organizations, Identity Providers, and Groups → Read**, and
  **Zone → Read** + **DNS → Edit** on `dotx3.net`

```bash
cd ~/x3lang && git fetch origin feat/agent-forge && git checkout feat/agent-forge   # or master once merged
export CF_API_TOKEN=...          # not written to disk
export CF_ACCOUNT_ID=...
export X3_OWNER_EMAIL=you@example.com
deploy/forge/deploy.sh up
sudo loginctl enable-linger "$USER"   # once, so it survives logout
```

`up` stops at the first failure, in this order:
1. checks that `forge.dotx3.net` is in a Cloudflare zone of this account
2. builds a release from one exact commit (the x3-lang modules, the Forge and its built web app)
   in `~/x3-forge/releases/<sha>`, with a venv in `~/x3-forge/venv`
3. creates or updates the Access app and its owner-only policy, then reads them back
4. starts `x3-forge-web.service` on 127.0.0.1:7420. The health check requires `/api/status` to
   answer **403** without a login.
5. creates the `x3-forge` tunnel and starts it
6. verifies the Access app again, then points `forge.dotx3.net` at the tunnel. It uses a
   proxied CNAME through the API and refuses to overwrite a record that isn't a tunnel.

At the end it prints the one-time High Command setup token. Open https://forge.dotx3.net, sign in
with your email, go to **Birth** and claim. Agents, the runtime and High Command live in
`~/x3-forge/data` and survive updates.

## Day to day

```bash
deploy/forge/deploy.sh status
deploy/forge/deploy.sh update [--ref <sha>]   # ship or roll back; no Cloudflare calls
deploy/forge/deploy.sh token                  # setup token, until High Command is claimed
deploy/forge/deploy.sh lock                   # re-verify the Access gate
deploy/forge/deploy.sh down
```

`X3_FORGE_HOST`, `X3_FORGE_PORT`, `X3_FORGE_TUNNEL` and `X3_FORGE_HOME` change the defaults.

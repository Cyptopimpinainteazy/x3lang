# x3star.net hosting for the X3Lang demos

| Hostname | Serves |
|---|---|
| `x3lang.x3star.net` | Hub linking everything below (`deploy/x3star/site/`) |
| `controlroom.x3star.net` | X3 Shield control room (`docs/hackathon/attack-lab.html`, plus the evidence it links) |
| `guard.x3star.net` | X3 Guard live 1Click dry-quote check: `x3-lang/near/demo/` at `/`, with `/judge`, `/bnb`, `/index.md` and its 404 page, as on x3-guard.pages.dev |
| `judge.x3star.net` | Same folder, opening on the judge console |

Login-gated until release: Cloudflare Access lets only the owner's email through, and the
origin server independently refuses any request without a valid Access JWT.

## First deploy (on the x3star box, about 2 minutes)

One-time prerequisites: `node` 20+, `cloudflared` logged in (`cloudflared tunnel login`), and a
Cloudflare API token with **Access: Apps and Policies → Edit** and **Access: Organizations,
Identity Providers, and Groups → Read** on the account that owns `x3star.net`.

```bash
git clone https://github.com/Cyptopimpinainteazy/x3lang && cd x3lang
git checkout feat/x3star-demo-hosting-20261108        # or master once merged
export CF_API_TOKEN=...            # not written to disk
export CF_ACCOUNT_ID=...           # Cloudflare dashboard → right sidebar
export X3_OWNER_EMAIL=you@example.com
deploy/x3star/deploy.sh up
sudo loginctl enable-linger "$USER"   # once, so the services survive logout
```

`up` runs, in this order, and stops at the first failure:

1. **Release**: exports only the files listed in `sites.json` from one exact commit into
   `~/x3lang-demos/releases/<sha>` and points `current` at it.
2. **Lock**: creates or updates the reusable policy *X3Lang demos: owner only* and the Access app
   *X3Lang demos (x3star.net)* covering all four hostnames, then reads the app back to verify it.
3. **Web server**: a systemd user service on `127.0.0.1:7410`. A health check requires every
   hostname to answer **403** without a login.
4. **Tunnel**: a dedicated tunnel `x3lang-demos`, so the existing x3star.net tunnel config is untouched.
5. **DNS**: verifies the Access app again, then points each hostname at the tunnel
   (`--overwrite-dns`, which replaces any old record for these four names).

Your existing `x3star.net` tunnel and its other subdomains are not changed.

## Day to day

```bash
deploy/x3star/deploy.sh status                 # services, release, and what each hostname returns
deploy/x3star/deploy.sh update                 # ship the checked-out HEAD (no Cloudflare calls)
deploy/x3star/deploy.sh update --ref <sha>     # ship or roll back to an exact commit
deploy/x3star/deploy.sh down                   # stop the server and the tunnel
```

## Release day

```bash
deploy/x3star/deploy.sh go-public --yes-publish   # origin serves without login, Access app deleted
deploy/x3star/deploy.sh lock                      # undo: lock it again
```

## What is never served

Anything not listed in `sites.json`, including `judge/evidence.json` (a local record that can
contain file paths), Python sources, dotfiles and `.env` files. Traversal (`..`, `%2e%2e`),
backslashes, NUL and symlinks that point outside the release all return 404 or 400. HTML pages
carry a CSP that allows only their own inline scripts, by SHA-256, and only the outside origins
in `sites.json` (1Click API, FastNEAR RPC, Google Fonts).

## Adding a page

Add the hostname to `sites` and every file it loads to `files` in `sites.json`, then
`deploy.sh up` (new hostnames need the Access app and DNS updated; `update` alone is enough for
changes to existing pages). The server refuses to start if a listed file is missing.

## Tests

```bash
node --test deploy/x3star/*.test.mjs
```

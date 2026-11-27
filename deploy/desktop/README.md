# X3 on your desktop

One script runs the full local stack in `~/Desktop/x3`:

- **Agent Forge** (`forge.server`, local mode, `127.0.0.1:7420`)
- **GPU mesh coordinator** (`gpu_mesh`, `127.0.0.1:8790`)
- **This machine's GPU** as a fleet worker, through your local Ollama

Everything binds to loopback. Nothing touches forge.dotx3.net, x3star1 or any chain.

```bash
git clone https://github.com/Cyptopimpinainteazy/x3lang.git /tmp/x3lang-bootstrap
/tmp/x3lang-bootstrap/deploy/desktop/x3-desktop.sh install --pull-model   # Python 3.10+, Node 20.19+/22.12+, Ollama
~/Desktop/x3/src/x3lang/deploy/desktop/x3-desktop.sh start                # prints the High Command setup token
~/Desktop/x3/src/x3lang/deploy/desktop/x3-desktop.sh smoke                # one real completion through the mesh
```

Then open http://127.0.0.1:7420:

1. Claim High Command with the token.
2. Pick **Mesh Mind** in the Skill Tree.
3. In the Comms Rig, plug in **X3 GPU mesh**:
   - URL: `http://127.0.0.1:8790`
   - Key: `~/Desktop/x3/keys/forge-app.key`
4. Prove, birth, then give the agent a task in its **Mind** panel.

Layout:

- `src/x3lang`: the checkout (`--ref` picks the branch; default `feat/forge-mesh-runner`)
- `venv`: the Python environment
- `keys`: the coordinator, worker and Forge app keys (0600)
- `data`: the Forge store and the mesh database
- `logs`, `run`: process logs and pid files

Other commands: `status`, `stop`, `restart`, `logs`. Settings come from environment variables:

- `X3_HOME`
- `X3_REF`
- `X3_MODEL` (default `qwen3:8b`)
- `X3_OLLAMA_URL`
- `X3_FORGE_PORT`
- `X3_MESH_PORT`

Linux and macOS run the script directly. On Windows, run it inside WSL with Ollama for Windows
(WSL reaches it at `127.0.0.1:11434`). Set `X3_HOME=/mnt/c/Users/<you>/Desktop/x3` to keep it on the
Windows desktop.

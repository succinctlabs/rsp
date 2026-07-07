# ethproofs

This package builds two binaries:

- **`ethproofs`** — the long-running proving service. Subscribes to blocks, executes and proves
  the sampled ones, and reports to the [ethproofs](https://ethproofs.org) API. See `--help`.
- **`ethproofs-cli`** — utility commands for the ethproofs API (cluster management + verification
  key generation).

## `ethproofs-cli`

Credentials are read from `--endpoint` / `--api-token`, defaulting to the `ETH_PROOFS_ENDPOINT`
and `ETH_PROOFS_API_TOKEN` environment variables (a `.env` file is loaded automatically). The
endpoint is the API base URL including the version segment, e.g. `https://ethproofs.org/api/v0`.

### Cluster commands

```bash
# List the clusters owned by your team.
ethproofs-cli cluster list

# Create a cluster (zkvm-version-id comes from https://ethproofs.org/docs/zkvms).
ethproofs-cli cluster create --name ZKnight-01 --zkvm-version-id 1 \
    --num-gpus 8 --hardware-description "8x H100" --deployment-type on-prem

# Update metadata or point the cluster at a new zkVM version.
ethproofs-cli cluster patch --id 3 --is-active false
```

### Updating the cluster verification key

The VK that ethproofs stores for an SP1-hypercube cluster is **not** the on-chain `bytes32` vkey
hash. It is the 32-byte value `bincode(vk.hash_koalabear())` — the input the
`sp1-hypercube` WASM verifier expects as `vk_bytes`. `gen-vk` produces exactly that file for the
`rsp-client` program:

```bash
ethproofs-cli cluster gen-vk --output vk.bin
```

This runs SP1's *light* prover (execute/verify only — no GPU required) to compute the program's
verifying key, writes the 32-byte VK file, and prints the `bytes32` hash for reference.

**Uploading the file is a website action, not an API-key one.** ethproofs' VK-upload endpoint
authenticates with a website (Supabase) session rather than an API key, so it cannot be driven
from this CLI. Upload `vk.bin` through the ethproofs website's admin verification-key form. The
documented `PATCH /clusters/{id}` `vk_path` field only stores a storage *path* to an
already-uploaded key (exposed here as `cluster patch --vk-path`), so it is not a substitute for
the upload.

# Generating Proofs

If you want to actually generate proofs, you can run the CLI using the `--prove` argument, like this:

```bash
rsp --block-number 18884864 --chain-id <chain-id> --prove
```

This will generate proofs locally on your machine. Given how large these programs are, it might take a while for the proof to generate.

## Run with prover network

If you want to run proofs using Succinct's [prover network](https://docs.succinct.xyz/docs/sp1/generating-proofs/prover-network), follow the sign-up instructions, and run the command with the following environment variables prefixed:

```bash
SP1_PROVER=network NETWORK_PRIVATE_KEY=<pk>
```

To specify a custom prover network RPC, you can use the `PROVER_NETWORK_RPC` environment variable.

## Run with GPU

To generate proofs locally on a GPU, run the command with the following environment variable prefixed:

```bash
SP1_PROVER=cuda
```
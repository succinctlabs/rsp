# FAQ

## Building the client programs manually

By default, the `build.rs` in the `bin/host` crate will rebuild the client programs every time they are modified. To manually build the client programs, you can run these commands (ake sure you have the [SP1 toolchain](https://docs.succinct.xyz/docs/sp1/getting-started/install) installed):

```console
cd ./bin/client-eth
cargo prove build --ignore-rust-version
```

To build the Optimism client ELF program:

```console
cd ./bin/client-op
cargo prove build --ignore-rust-version
```

## What are good testing blocks

A good small block to test on for Ethereum mainnet is: `20526624`.

## State root mismatch

This issue can be caused using an RPC provider that returns incorrect results from the `eth_getProof` endpoint. We have empirically observed such issues with many RPC providers. We recommend using Alchemy.

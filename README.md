# Reth Succinct Processor (RSP)

RSP is a minimal implementation of generating zero-knowledge proofs of EVM block execution using [Reth](https://reth.rs). Supports both Ethereum and OP Stack.

[Docs](https://succinctlabs.github.io/rsp/)

## Overview

RSP is designed to generate zero-knowledge proofs of EVM block execution using components from [Reth](https://reth.rs) and [SP1](https://docs.succinct.xyz/docs/sp1/introduction). The system is split between a host CLI that prepares execution data and orchestrates the process, and a client program that runs within a zero-knowledge virtual machine (SP1) to generate proofs.

The repository is organized into the following directories:

* `book`: The documentation for RSP users and developers.
* `bin/client` and `bin/client`: The programs that runs inside the zkVM.
* `bin/host`: The CLI to prepare the proving process.
* `crates`: RSP components like the host and client executors


> [!CAUTION]
>
> This repository is still an active work-in-progress and is not audited or meant for production usage.

## Acknowledgments

This repo would not exist without:

* [Reth](https://reth.rs): Highly modular Ethereum execution layer implementation.
* [SP1](https://github.com/succinctlabs/sp1): The fastest, most feature-complete zkVM for developers.
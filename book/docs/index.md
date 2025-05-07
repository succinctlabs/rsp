---
title: Introduction
sidebar_position: 1
---

# Reth Succinct Processor (RSP)

RSP is minimal implementation of generating zero-knowledge proofs of EVM block execution using [Reth](https://reth.rs/). Supports both Ethereum and OP Stack.

The system is split between a host environment that prepares execution data and orchestrates the process, and a client environment that runs within the [SP1](https://docs.succinct.xyz/docs/sp1/introduction) zero-knowledge virtual machine to generate proofs.

:::danger

RSP is still an active work-in-progress and is not audited or meant for production usage.

:::
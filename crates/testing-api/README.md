# RSP Testing API

A simplified API for working with RSP (Reth in SP1) in testing contexts.

## Features

This crate provides two main functions:

1. **`fetch_block_stdin`** - Fetches and executes a block, generating stdin data
2. **`get_program_elf`** - Retrieves the compiled ELF for RSP client programs (requires `embedded-programs` feature)

## Feature Flags

- `embedded-programs` - Embeds the compiled ELF programs at build time. This enables the `get_program_elf` function but significantly increases build time as it compiles the SP1 client programs.

## Usage

### Basic Usage (without embedded programs)

Add to your `Cargo.toml`:

```toml
[dependencies]
rsp-testing-api = { path = "../rsp/crates/testing-api" }
```

### With Embedded Programs

Add to your `Cargo.toml`:

```toml
[dependencies]
rsp-testing-api = { path = "../rsp/crates/testing-api", features = ["embedded-programs"] }
```

## Example

```rust
use rsp_testing_api::{fetch_block_stdin, get_program_elf, ProgramType};
use std::path::Path;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Fetch stdin for a block
    let stdin_path = fetch_block_stdin(
        21000000,
        "https://eth-mainnet.g.alchemy.com/v2/YOUR_API_KEY",
        Path::new("./cache")
    ).await?;

    println!("Stdin written to: {}", stdin_path.display());

    // Get the ETH program ELF (requires 'embedded-programs' feature)
    #[cfg(feature = "embedded-programs")]
    {
        let eth_elf = get_program_elf(ProgramType::Eth)?;
        std::fs::write("./eth-program.elf", eth_elf)?;

        let op_elf = get_program_elf(ProgramType::Op)?;
        std::fs::write("./op-program.elf", op_elf)?;
    }

    Ok(())
}
```

## API Documentation

### `fetch_block_stdin`

```rust
pub async fn fetch_block_stdin(
    block_number: u64,
    rpc_url: &str,
    output_dir: &Path,
) -> Result<PathBuf, TestingApiError>
```

Fetches block execution data and generates a stdin file. The stdin file will be written to `{output_dir}/input/{chain_id}/{block_number}.bin`.

**Arguments:**
- `block_number` - The block number to fetch and execute
- `rpc_url` - The RPC URL for fetching block data (must be an archive node)
- `output_dir` - The directory where the stdin file will be written

**Returns:** The path to the generated stdin file

### `get_program_elf`

```rust
pub fn get_program_elf(program_type: ProgramType) -> Result<Vec<u8>, TestingApiError>
```

Get the compiled ELF bytes for the specified program type. Requires the `embedded-programs` feature.

**Arguments:**
- `program_type` - Either `ProgramType::Eth` or `ProgramType::Op`

**Returns:** A vector containing the ELF bytes

## Integration with Testing Suite

In your testing suite's `Cargo.toml`:

```toml
[dependencies]
rsp-testing-api = { path = "../rsp/crates/testing-api", features = ["embedded-programs"] }
```

Then in your test code:

```rust
use rsp_testing_api::{fetch_block_stdin, get_program_elf, ProgramType};

// Fetch new test cases dynamically
let stdin = fetch_block_stdin(block_num, rpc_url, output_dir).await?;

// Get program ELFs
let eth_elf = get_program_elf(ProgramType::Eth)?;
let op_elf = get_program_elf(ProgramType::Op)?;
```

## Notes

- The `fetch_block_stdin` function requires an archive node RPC endpoint
- When using `embedded-programs`, build times will be significantly longer (3-5 minutes) due to SP1 program compilation
- The generated stdin files follow RSP's standard directory structure: `{cache-dir}/input/{chain-id}/{block-number}.bin`

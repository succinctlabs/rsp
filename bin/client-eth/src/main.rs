#![no_main]
sp1_zkvm::entrypoint!(main);

use reth_primitives::Genesis;
use rsp_client_executor::{io::ClientExecutorInput, ChainVariant, ClientExecutor};

pub fn main() {
    // Read the input.
    let input = sp1_zkvm::io::read_vec();
    let input = bincode::deserialize::<ClientExecutorInput>(&input).unwrap();

    let variant = if let Some(genesis) = &input.genesis {
        let genesis = serde_json::from_str::<Genesis>(genesis).unwrap();
        ChainVariant::from_genesis(genesis)
    } else {
        ChainVariant::mainnet()
    };

    // Execute the block.
    let executor = ClientExecutor;
    let header = executor.execute(input, &variant).expect("failed to execute client");
    let block_hash = header.hash_slow();

    // Commit the block hash.
    sp1_zkvm::io::commit(&block_hash);
}

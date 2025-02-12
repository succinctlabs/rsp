#![no_main]
sp1_zkvm::entrypoint!(main);

use std::sync::Arc;

use rsp_client_executor::{executor::OpClientExecutor, io::OpClientExecutorInput};

pub fn main() {
    // Read the input.
    let input = sp1_zkvm::io::read_vec();
    let input = bincode::deserialize::<OpClientExecutorInput>(&input).unwrap();
    let genesis = input.genesis().unwrap();

    // Execute the block.
    let executor = OpClientExecutor::optimism(Arc::new(genesis.into()));
    let header = executor.execute(input).expect("failed to execute client");
    let block_hash = header.hash_slow();

    // Commit the block hash.
    sp1_zkvm::io::commit(&block_hash);
}

#![no_main]
sp1_zkvm::entrypoint!(main);

use rsp_client_executor::{io::ClientExecutorInput, ClientExecutor, CliqueShanghaiChainIDVariant};

pub fn main() {
    // Read the input.
    let input = sp1_zkvm::io::read_vec();
    let input = bincode::deserialize::<ClientExecutorInput>(&input).unwrap();
    let chainId = sp1_zkvm::io::read::<u64>();

    // Execute the block.
    let executor = ClientExecutor;
    let header = executor.execute_with_chain_id::<CliqueShanghaiChainIDVariant>(chainId,input).expect("failed to execute client");
    let block_hash = header.hash_slow();

    // Commit the block hash.
    sp1_zkvm::io::commit(&block_hash);
}

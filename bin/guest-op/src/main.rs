#![no_main]
sp1_zkvm::entrypoint!(main);

use rsp_guest_executor::{io::GuestExecutorInput, GuestExecutor, OptimismVariant};

pub fn main() {
    // Read the input.
    let input = sp1_zkvm::io::read_vec();
    let input = bincode::deserialize::<GuestExecutorInput>(&input).unwrap();

    // Execute the block.
    let executor = GuestExecutor;
    let header = executor.execute::<OptimismVariant>(input).expect("failed to execute guest");
    let block_hash = header.hash_slow();

    // Commit the block hash.
    sp1_zkvm::io::commit(&block_hash);
}

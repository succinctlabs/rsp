#![no_main]
sp1_zkvm::entrypoint!(main);

use rsp_guest_executor::{io::GuestExecutorInput, GuestExecutor};

pub fn main() {
    // Read the input.
    let input = sp1_zkvm::io::read_vec();
    let input: GuestExecutorInput = serde_json::from_slice(&input).unwrap();

    // Execute the block.
    let executor = GuestExecutor;
    executor.execute(input).expect("failed to execute guest");
}

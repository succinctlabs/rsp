#![no_main]
sp1_zkvm::entrypoint!(main);

use rsp_client_executor::{
    executor::{EthClientExecutor, DESERIALIZE_INPUTS},
    io::{CommittedHeader, EthClientExecutorInput},
};
use std::sync::Arc;

pub fn main() {
    // Read the input.
    println!("cycle-tracker-report-start: {}", DESERIALIZE_INPUTS);
    let input = sp1_zkvm::io::read_vec();
    let input = bincode::deserialize::<EthClientExecutorInput>(&input).unwrap();
    println!("cycle-tracker-report-end: {}", DESERIALIZE_INPUTS);

    // Execute the block.
    let executor = EthClientExecutor::eth(
        Arc::new((&input.genesis).try_into().unwrap()),
        input.custom_beneficiary,
    );
    let header = executor.execute(input).expect("failed to execute client");

    // Commit the block header.
    sp1_zkvm::io::commit::<CommittedHeader>(&header.into());
}

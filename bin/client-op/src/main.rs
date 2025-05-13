#![no_main]
sp1_zkvm::entrypoint!(main);

use rsp_client_executor::{
    executor::{OpClientExecutor, DESERIALZE_INPUTS},
    io::{CommittedHeader, OpClientExecutorInput},
};
use std::sync::Arc;

pub fn main() {
    // Read the input.
    println!("cycle-tracker-report-start: {}", DESERIALZE_INPUTS);
    let input = sp1_zkvm::io::read_vec();
    let input = bincode::deserialize::<OpClientExecutorInput>(&input).unwrap();
    println!("cycle-tracker-report-end: {}", DESERIALZE_INPUTS);

    // Execute the block.
    let executor = OpClientExecutor::optimism(Arc::new((&input.genesis).try_into().unwrap()));
    let header = executor.execute(input).expect("failed to execute client");

    // Commit the block hash.
    sp1_zkvm::io::commit::<CommittedHeader>(&header.into());
}

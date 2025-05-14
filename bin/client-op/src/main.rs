#![no_main]
sp1_zkvm::entrypoint!(main);

use rsp_client_executor::{
    executor::{OpClientExecutor, DESERIALZE_INPUTS},
    io::{CommittedHeader, OpClientExecutorInput},
    utils::profile_report,
};
use std::sync::Arc;

pub fn main() {
    // Read the input.
    let input = profile_report!(DESERIALZE_INPUTS, {
        let input = sp1_zkvm::io::read_vec();
        bincode::deserialize::<OpClientExecutorInput>(&input).unwrap()
    });

    // Execute the block.
    let executor = OpClientExecutor::optimism(Arc::new((&input.genesis).try_into().unwrap()));
    let header = executor.execute(input).expect("failed to execute client");

    // Commit the block hash.
    sp1_zkvm::io::commit::<CommittedHeader>(&header.into());
}

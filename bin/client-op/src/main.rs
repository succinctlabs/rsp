#![no_main]
sp1_zkvm::entrypoint!(main);

use rsp_client_executor::{
    executor::{OpClientExecutor, DESERIALZE_INPUTS},
    io::{CommittedHeader, OpClientExecutorInput},
    utils::profile_report,
};
use std::sync::Arc;

pub fn main() {
    // Read the input. The witness blob is a *separate* stdin item (parent_state is
    // `#[serde(skip)]`), so the bincode pass only handles the small header and the witness is
    // read straight from the SP1 input region and decoded zero-copy in the executor.
    let input = profile_report!(DESERIALZE_INPUTS, {
        let header_bytes = sp1_zkvm::io::read_vec();
        let mut input: OpClientExecutorInput = bincode::deserialize(&header_bytes).unwrap();
        input.parent_state = sp1_zkvm::io::read_vec();
        input
    });

    // Execute the block.
    let executor = OpClientExecutor::optimism(Arc::new((&input.genesis).try_into().unwrap()));
    let header = executor.execute(input).expect("failed to execute client");

    // Commit the block hash.
    sp1_zkvm::io::commit::<CommittedHeader>(&header.into());
}

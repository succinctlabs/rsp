#![no_main]
sp1_zkvm::entrypoint!(main);

use rsp_client_executor::{
    executor::{EthClientExecutor, DESERIALZE_INPUTS},
    io::{CommittedHeader, EthClientExecutorInput},
    utils::profile_report,
};
use std::sync::Arc;

pub fn main() {
    // Read the input. The witness blob is a *separate* stdin item, so bincode only handles the
    // small header (parent_state is `#[serde(skip)]`); the witness is then read straight from
    // the SP1 input region into `input.parent_state` and decoded zero-copy in the executor.
    let input = profile_report!(DESERIALZE_INPUTS, {
        let header_bytes = sp1_zkvm::io::read_vec();
        let mut input: EthClientExecutorInput = bincode::deserialize(&header_bytes).unwrap();
        input.parent_state = sp1_zkvm::io::read_vec();
        input
    });

    // Execute the block.
    let executor = EthClientExecutor::eth(
        Arc::new((&input.genesis).try_into().unwrap()),
        input.custom_beneficiary,
    );
    let header = executor.execute(input).expect("failed to execute client");

    // Commit the block header.
    sp1_zkvm::io::commit::<CommittedHeader>(&header.into());
}

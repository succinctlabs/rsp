#![no_main]
sp1_zkvm::entrypoint!(main);

use std::sync::Arc;

use reth_evm_ethereum::execute::EthExecutionStrategyFactory;
use reth_primitives::EthPrimitives;
use rsp_client_executor::{
    custom::CustomEthEvmConfig, executor::EthClientExecutor, io::ClientExecutorInput,
};

pub fn main() {
    // Read the input.
    let input = sp1_zkvm::io::read_vec();
    let input = bincode::deserialize::<ClientExecutorInput<EthPrimitives>>(&input).unwrap();
    //let genesis = serde_json::from_str::<Genesis>(&input.genesis_json).unwrap();

    // Execute the block.
    let chain_spec = rsp_primitives::chain_spec::mainnet();
    let executor = EthClientExecutor::eth(chain_spec /*Arc::new(genesis.into())*/);
    let header = executor.execute(input).expect("failed to execute client");
    let block_hash = header.hash_slow();

    // Commit the block hash.
    sp1_zkvm::io::commit(&block_hash);
}

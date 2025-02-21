use crate::{
    cli::{HostArgs, ProviderConfig},
    eth_proofs::EthProofsClient,
    execute::execute,
};
use alloy_chains::Chain;
use op_alloy_network::{Ethereum, Optimism};
use rsp_host_executor::{
    create_eth_block_execution_strategy_factory, create_op_block_execution_strategy_factory,
};
use rsp_primitives::genesis::Genesis;

pub async fn process_single_block(
    args: HostArgs,
    provider_config: ProviderConfig,
    genesis: Genesis,
) -> eyre::Result<()> {
    let block_number = args.block_number.unwrap();
    let is_optimism = Chain::from_id(provider_config.chain_id).is_optimism();

    let eth_proofs_client = setup_eth_proofs(&args, block_number).await?;
    execute_block(args, provider_config, genesis, eth_proofs_client, is_optimism).await?;

    Ok(())
}

async fn setup_eth_proofs(
    args: &HostArgs,
    block_number: u64,
) -> eyre::Result<Option<EthProofsClient>> {
    let eth_proofs_client = EthProofsClient::new(
        args.eth_proofs_cluster_id,
        args.eth_proofs_endpoint.clone(),
        args.eth_proofs_api_token.clone(),
    );

    if let Some(eth_proofs_client) = &eth_proofs_client {
        eth_proofs_client.queued(block_number).await?;
    }

    Ok(eth_proofs_client)
}

async fn execute_block(
    args: HostArgs,
    provider_config: ProviderConfig,
    genesis: Genesis,
    eth_proofs_client: Option<EthProofsClient>,
    is_optimism: bool,
) -> eyre::Result<()> {
    match is_optimism {
        true => {
            let block_execution_strategy_factory =
                create_op_block_execution_strategy_factory(&genesis);
            execute::<Optimism, _, _>(
                args,
                provider_config,
                genesis,
                eth_proofs_client,
                block_execution_strategy_factory,
                true,
            )
            .await
        }
        false => {
            let block_execution_strategy_factory =
                create_eth_block_execution_strategy_factory(&genesis, args.custom_beneficiary);
            execute::<Ethereum, _, _>(
                args,
                provider_config,
                genesis,
                eth_proofs_client,
                block_execution_strategy_factory,
                false,
            )
            .await
        }
    }
}

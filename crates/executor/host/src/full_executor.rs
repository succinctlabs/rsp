use std::{marker::PhantomData, path::PathBuf};

use alloy_provider::{Network, RootProvider};
use alloy_rpc_client::RpcClient;
use alloy_transport::layers::RetryBackoffLayer;
use reth_evm::execute::BlockExecutionStrategyFactory;
use reth_primitives::NodePrimitives;
use revm_primitives::B256;
use rsp_client_executor::{io::ClientExecutorInput, IntoInput, IntoPrimitives};
use rsp_rpc_db::RpcDb;
use serde::de::DeserializeOwned;
use sp1_sdk::{EnvProver, SP1ProvingKey, SP1Stdin, SP1VerifyingKey};

use crate::{Config, ExecutionHooks, HostExecutor};

pub struct FullExecutor<N, NP, F, H>
where
    N: Network,
    NP: NodePrimitives + DeserializeOwned + IntoPrimitives<N> + IntoInput,
    F: BlockExecutionStrategyFactory<Primitives = NP>,
    H: ExecutionHooks,
{
    host_executor: HostExecutor<F>,
    client: EnvProver,
    pk: SP1ProvingKey,
    vk: SP1VerifyingKey,
    hooks: H,
    config: Config,
    phantom: PhantomData<N>,
}

impl<N, NP, F, H> FullExecutor<N, NP, F, H>
where
    N: Network,
    NP: NodePrimitives + DeserializeOwned + IntoPrimitives<N> + IntoInput,
    F: BlockExecutionStrategyFactory<Primitives = NP>,
    H: ExecutionHooks,
{
    pub fn new(
        elf: Vec<u8>,
        block_execution_strategy_factory: F,
        hooks: H,
        config: Config,
    ) -> Self {
        let client = EnvProver::new();

        // Setup the proving key and verification key.
        let (pk, vk) = client.setup(&elf);

        Self {
            host_executor: HostExecutor::new(block_execution_strategy_factory),
            client,
            pk,
            vk,
            hooks,
            config,
            phantom: Default::default(),
        }
    }

    pub async fn execute(&mut self, block_number: u64) -> eyre::Result<()> {
        let client_input_from_cache = try_load_input_from_cache::<NP>(
            self.config.cache_dir.as_ref(),
            self.config.chain.id(),
            block_number,
        )?;

        self.hooks.on_execution_start(block_number).await?;

        let client_input = match (client_input_from_cache, self.config.rpc_url.clone()) {
            (Some(client_input_from_cache), _) => client_input_from_cache,
            (None, Some(rpc_url)) => {
                let retry_layer = RetryBackoffLayer::new(3, 1000, 100);
                let client = RpcClient::builder().layer(retry_layer).http(rpc_url);
                let provider = RootProvider::<N>::new(client);

                let rpc_db = RpcDb::new(provider.clone(), block_number - 1);

                // Execute the host.
                let client_input = self
                    .host_executor
                    .execute(
                        block_number,
                        &rpc_db,
                        &provider,
                        self.config.genesis.clone(),
                        self.config.custom_beneficiary,
                    )
                    .await?;

                if let Some(ref cache_dir) = self.config.cache_dir {
                    let input_folder = cache_dir.join(format!("input/{}", self.config.chain.id()));
                    if !input_folder.exists() {
                        std::fs::create_dir_all(&input_folder)?;
                    }

                    let input_path = input_folder.join(format!("{}.bin", block_number));
                    let mut cache_file = std::fs::File::create(input_path)?;

                    bincode::serialize_into(&mut cache_file, &client_input)?;
                }

                client_input
            }
            (None, None) => {
                eyre::bail!("cache not found and RPC URL not provided")
            }
        };

        // Generate the proof.
        // Execute the block inside the zkVM.
        let mut stdin = SP1Stdin::new();
        let buffer = bincode::serialize(&client_input).unwrap();

        stdin.write_vec(buffer);

        // Only execute the program.
        let (mut public_values, execution_report) =
            self.client.execute(&self.pk.elf, &stdin).run().unwrap();

        // Read the block hash.
        let block_hash = public_values.read::<B256>();
        println!("success: block_hash={block_hash}");

        self.hooks.on_execution_end(block_number, &client_input, &execution_report).await?;

        if self.config.prove {
            println!("Starting proof generation.");

            self.hooks.on_proving_start(block_number).await?;

            let proof = self
                .client
                .prove(&self.pk, &stdin)
                .compressed()
                .run()
                .expect("Proving should work.");
            let proof_bytes = bincode::serialize(&proof.proof).unwrap();

            self.hooks
                .on_proving_end(block_number, &proof_bytes, &self.vk, &execution_report)
                .await?;
        }

        Ok(())
    }
}

fn try_load_input_from_cache<P: NodePrimitives + DeserializeOwned>(
    cache_dir: Option<&PathBuf>,
    chain_id: u64,
    block_number: u64,
) -> eyre::Result<Option<ClientExecutorInput<P>>> {
    Ok(if let Some(cache_dir) = cache_dir {
        let cache_path = cache_dir.join(format!("input/{}/{}.bin", chain_id, block_number));

        if cache_path.exists() {
            // TODO: prune the cache if invalid instead
            let mut cache_file = std::fs::File::open(cache_path)?;
            let client_input: ClientExecutorInput<P> = bincode::deserialize_from(&mut cache_file)?;

            Some(client_input)
        } else {
            None
        }
    } else {
        None
    })
}

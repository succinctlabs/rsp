use std::{
    fmt::{Debug, Formatter},
    marker::PhantomData,
    path::{Path, PathBuf},
};

use alloy_provider::{Network, Provider};
use either::Either;
use reth_evm::execute::BlockExecutionStrategyFactory;
use reth_primitives::NodePrimitives;
use revm_primitives::B256;
use rsp_client_executor::{io::ClientExecutorInput, IntoInput, IntoPrimitives};
use rsp_rpc_db::RpcDb;
use serde::de::DeserializeOwned;
use sp1_sdk::{EnvProver, SP1ProvingKey, SP1Stdin, SP1VerifyingKey};
use tracing::warn;

use crate::{Config, ExecutionHooks, HostExecutor};

pub type EitherExecutor<P, N, NP, F, H> =
    Either<FullExecutor<P, N, NP, F, H>, CachedExecutor<NP, H>>;

pub fn build_executor<P, N, NP, F, H>(
    elf: Vec<u8>,
    provider: Option<P>,
    block_execution_strategy_factory: F,
    hooks: H,
    config: Config,
) -> eyre::Result<EitherExecutor<P, N, NP, F, H>>
where
    P: Provider<N> + Clone,
    N: Network,
    NP: NodePrimitives + DeserializeOwned + IntoPrimitives<N> + IntoInput,
    F: BlockExecutionStrategyFactory<Primitives = NP>,
    H: ExecutionHooks,
{
    if let Some(provider) = provider {
        return Ok(Either::Left(FullExecutor::new(
            provider,
            elf,
            block_execution_strategy_factory,
            hooks,
            config,
        )));
    }

    if let Some(cache_dir) = config.cache_dir {
        return Ok(Either::Right(CachedExecutor::new(
            elf,
            hooks,
            cache_dir,
            config.chain.id(),
            config.prove,
        )));
    }

    todo!()
}

pub trait BlockExecutor {
    #[allow(async_fn_in_trait)]
    async fn execute(&mut self, block_number: u64) -> eyre::Result<()>;
}

impl<P, N, NP, F, H> BlockExecutor for EitherExecutor<P, N, NP, F, H>
where
    P: Provider<N> + Clone,
    N: Network,
    NP: NodePrimitives + DeserializeOwned + IntoPrimitives<N> + IntoInput,
    F: BlockExecutionStrategyFactory<Primitives = NP>,
    H: ExecutionHooks,
{
    async fn execute(&mut self, block_number: u64) -> eyre::Result<()> {
        match self {
            Either::Left(ref mut executor) => executor.execute(block_number).await,
            Either::Right(ref mut executor) => executor.execute(block_number).await,
        }
    }
}

pub struct FullExecutor<P, N, NP, F, H>
where
    P: Provider<N> + Clone,
    N: Network,
    NP: NodePrimitives + DeserializeOwned + IntoPrimitives<N> + IntoInput,
    F: BlockExecutionStrategyFactory<Primitives = NP>,
    H: ExecutionHooks,
{
    provider: P,
    host_executor: HostExecutor<F>,
    client: EnvProver,
    pk: SP1ProvingKey,
    vk: SP1VerifyingKey,
    hooks: H,
    config: Config,
    phantom: PhantomData<N>,
}

impl<P, N, NP, F, H> FullExecutor<P, N, NP, F, H>
where
    P: Provider<N> + Clone,
    N: Network,
    NP: NodePrimitives + DeserializeOwned + IntoPrimitives<N> + IntoInput,
    F: BlockExecutionStrategyFactory<Primitives = NP>,
    H: ExecutionHooks,
{
    pub fn new(
        provider: P,
        elf: Vec<u8>,
        block_execution_strategy_factory: F,
        hooks: H,
        config: Config,
    ) -> Self {
        let client = EnvProver::new();

        // Setup the proving key and verification key.
        let (pk, vk) = client.setup(&elf);

        Self {
            provider,
            host_executor: HostExecutor::new(block_execution_strategy_factory),
            client,
            pk,
            vk,
            hooks,
            config,
            phantom: Default::default(),
        }
    }
}

impl<P, N, NP, F, H> BlockExecutor for FullExecutor<P, N, NP, F, H>
where
    P: Provider<N> + Clone,
    N: Network,
    NP: NodePrimitives + DeserializeOwned + IntoPrimitives<N> + IntoInput,
    F: BlockExecutionStrategyFactory<Primitives = NP>,
    H: ExecutionHooks,
{
    async fn execute(&mut self, block_number: u64) -> eyre::Result<()> {
        self.hooks.on_execution_start(block_number).await?;

        let client_input_from_cache = self.config.cache_dir.as_ref().and_then(|cache_dir| {
            match try_load_input_from_cache::<NP>(cache_dir, self.config.chain.id(), block_number) {
                Ok(client_input) => client_input,
                Err(e) => {
                    warn!("Failed to load input from cache: {}", e);
                    None
                }
            }
        });

        let client_input = match client_input_from_cache {
            Some(client_input_from_cache) => client_input_from_cache,
            None => {
                let rpc_db = RpcDb::new(self.provider.clone(), block_number - 1);

                // Execute the host.
                let client_input = self
                    .host_executor
                    .execute(
                        block_number,
                        &rpc_db,
                        &self.provider,
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
        };

        execute_client(
            client_input,
            &self.client,
            &self.pk,
            &self.vk,
            &mut self.hooks,
            self.config.prove,
        )
        .await?;

        Ok(())
    }
}

impl<P, N, NP, F, H> Debug for FullExecutor<P, N, NP, F, H>
where
    P: Provider<N> + Clone,
    N: Network,
    NP: NodePrimitives + DeserializeOwned + IntoPrimitives<N> + IntoInput,
    F: BlockExecutionStrategyFactory<Primitives = NP>,
    H: ExecutionHooks,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FullExecutor").field("config", &self.config).finish()
    }
}

pub struct CachedExecutor<NP: NodePrimitives, H: ExecutionHooks> {
    cache_dir: PathBuf,
    chain_id: u64,
    client: EnvProver,
    pk: SP1ProvingKey,
    vk: SP1VerifyingKey,
    hooks: H,
    prove: bool,
    phantom: PhantomData<NP>,
}

impl<NP, H> CachedExecutor<NP, H>
where
    NP: NodePrimitives + DeserializeOwned,
    H: ExecutionHooks,
{
    pub fn new(elf: Vec<u8>, hooks: H, cache_dir: PathBuf, chain_id: u64, prove: bool) -> Self {
        let client = EnvProver::new();

        // Setup the proving key and verification key.
        let (pk, vk) = client.setup(&elf);

        Self { cache_dir, chain_id, client, pk, vk, hooks, prove, phantom: Default::default() }
    }
}

impl<NP, H> BlockExecutor for CachedExecutor<NP, H>
where
    NP: NodePrimitives + DeserializeOwned,
    H: ExecutionHooks,
{
    async fn execute(&mut self, block_number: u64) -> eyre::Result<()> {
        let client_input =
            try_load_input_from_cache::<NP>(&self.cache_dir, self.chain_id, block_number)?
                .ok_or(eyre::eyre!("No cached input found"))?;

        execute_client(client_input, &self.client, &self.pk, &self.vk, &mut self.hooks, self.prove)
            .await
    }
}

impl<NP: NodePrimitives, H: ExecutionHooks> Debug for CachedExecutor<NP, H> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachedExecutor").field("cache_dir", &self.cache_dir).finish()
    }
}

async fn execute_client<P: NodePrimitives, H: ExecutionHooks>(
    client_input: ClientExecutorInput<P>,
    client: &EnvProver,
    pk: &SP1ProvingKey,
    vk: &SP1VerifyingKey,
    hooks: &mut H,
    prove: bool,
) -> eyre::Result<()> {
    // Generate the proof.
    // Execute the block inside the zkVM.
    let mut stdin = SP1Stdin::new();
    let buffer = bincode::serialize(&client_input).unwrap();

    stdin.write_vec(buffer);

    // Only execute the program.
    let (mut public_values, execution_report) = client.execute(&pk.elf, &stdin).run().unwrap();

    // Read the block hash.
    let block_hash = public_values.read::<B256>();
    println!("success: block_hash={block_hash}");

    hooks
        .on_execution_end(client_input.current_block.number, &client_input, &execution_report)
        .await?;

    if prove {
        println!("Starting proof generation.");

        hooks.on_proving_start(client_input.current_block.number).await?;

        let proof = client.prove(pk, &stdin).compressed().run().expect("Proving should work.");
        let proof_bytes = bincode::serialize(&proof.proof).unwrap();

        hooks
            .on_proving_end(client_input.current_block.number, &proof_bytes, vk, &execution_report)
            .await?;
    }

    Ok(())
}

fn try_load_input_from_cache<P: NodePrimitives + DeserializeOwned>(
    cache_dir: &Path,
    chain_id: u64,
    block_number: u64,
) -> eyre::Result<Option<ClientExecutorInput<P>>> {
    let cache_path = cache_dir.join(format!("input/{}/{}.bin", chain_id, block_number));

    if cache_path.exists() {
        // TODO: prune the cache if invalid instead
        let mut cache_file = std::fs::File::open(cache_path)?;
        let client_input = bincode::deserialize_from(&mut cache_file)?;

        Ok(Some(client_input))
    } else {
        Ok(None)
    }
}

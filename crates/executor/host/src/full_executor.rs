use std::{
    fmt::{Debug, Formatter},
    marker::PhantomData,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use alloy_provider::{Network, Provider};
use either::Either;
use eyre::bail;
use reth_evm::execute::BlockExecutionStrategyFactory;
use reth_primitives::NodePrimitives;
use revm_primitives::B256;
use rsp_client_executor::{
    io::ClientExecutorInput, IntoInput, IntoPrimitives, ValidateBlockPostExecution,
};
use rsp_rpc_db::RpcDb;
use serde::de::DeserializeOwned;
use sp1_sdk::{
    EnvProver, ExecutionReport, SP1ProvingKey, SP1PublicValues, SP1Stdin, SP1VerifyingKey,
};
use tokio::{task, time::sleep};
use tracing::{info_span, warn};

use crate::{Config, ExecutionHooks, HostExecutor};

pub type EitherExecutor<P, N, NP, F, H> =
    Either<FullExecutor<P, N, NP, F, H>, CachedExecutor<NP, H>>;

pub async fn build_executor<P, N, NP, F, H>(
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
        return Ok(Either::Left(
            FullExecutor::try_new(provider, elf, block_execution_strategy_factory, hooks, config)
                .await?,
        ));
    }

    if let Some(cache_dir) = config.cache_dir {
        return Ok(Either::Right(
            CachedExecutor::try_new(elf, hooks, cache_dir, config.chain.id(), config.prove).await?,
        ));
    }

    bail!("Either a RPC URL or a cache dir must be provided")
}

pub trait BlockExecutor {
    #[allow(async_fn_in_trait)]
    async fn execute(&self, block_number: u64) -> eyre::Result<()>;

    fn client(&self) -> Arc<EnvProver>;

    fn pk(&self) -> Arc<SP1ProvingKey>;

    fn vk(&self) -> Arc<SP1VerifyingKey>;

    #[allow(async_fn_in_trait)]
    async fn process_client<P: NodePrimitives, H: ExecutionHooks>(
        &self,
        client_input: ClientExecutorInput<P>,
        hooks: &H,
        prove: bool,
    ) -> eyre::Result<()> {
        // Generate the proof.
        // Execute the block inside the zkVM.
        let mut stdin = SP1Stdin::new();
        let buffer = bincode::serialize(&client_input).unwrap();

        stdin.write_vec(buffer);

        // Only execute the program.
        let (stdin, execute_result) =
            execute_client(client_input.current_block.number, self.client(), self.pk(), stdin)
                .await?;
        let (mut public_values, execution_report) = execute_result?;

        // Read the block hash.
        let block_hash = public_values.read::<B256>();
        println!("success: block_hash={block_hash}");

        hooks.on_execution_end::<P>(&client_input.current_block, &execution_report).await?;

        if prove {
            println!("Starting proof generation.");

            let proving_start = Instant::now();
            hooks.on_proving_start(client_input.current_block.number).await?;
            let client = self.client();
            let pk = self.pk();

            let proof = task::spawn_blocking(move || {
                client
                    .prove(pk.as_ref(), &stdin)
                    .compressed()
                    .run()
                    .map_err(|err| eyre::eyre!("{err}"))
            })
            .await
            .map_err(|err| eyre::eyre!("{err}"))??;

            let proving_duration = proving_start.elapsed();
            let proof_bytes = bincode::serialize(&proof.proof).unwrap();

            hooks
                .on_proving_end(
                    client_input.current_block.number,
                    &proof_bytes,
                    self.vk().as_ref(),
                    &execution_report,
                    proving_duration,
                )
                .await?;
        }

        Ok(())
    }
}

impl<P, N, NP, F, H> BlockExecutor for EitherExecutor<P, N, NP, F, H>
where
    P: Provider<N> + Clone,
    N: Network,
    NP: NodePrimitives
        + DeserializeOwned
        + IntoPrimitives<N>
        + IntoInput
        + ValidateBlockPostExecution,
    F: BlockExecutionStrategyFactory<Primitives = NP>,
    H: ExecutionHooks,
{
    async fn execute(&self, block_number: u64) -> eyre::Result<()> {
        match self {
            Either::Left(ref executor) => executor.execute(block_number).await,
            Either::Right(ref executor) => executor.execute(block_number).await,
        }
    }

    fn client(&self) -> Arc<EnvProver> {
        match self {
            Either::Left(ref executor) => executor.client.clone(),
            Either::Right(ref executor) => executor.client.clone(),
        }
    }

    fn pk(&self) -> Arc<SP1ProvingKey> {
        match self {
            Either::Left(ref executor) => executor.pk.clone(),
            Either::Right(ref executor) => executor.pk.clone(),
        }
    }

    fn vk(&self) -> Arc<SP1VerifyingKey> {
        match self {
            Either::Left(ref executor) => executor.vk.clone(),
            Either::Right(ref executor) => executor.vk.clone(),
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
    client: Arc<EnvProver>,
    pk: Arc<SP1ProvingKey>,
    vk: Arc<SP1VerifyingKey>,
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
    pub async fn try_new(
        provider: P,
        elf: Vec<u8>,
        block_execution_strategy_factory: F,
        hooks: H,
        config: Config,
    ) -> eyre::Result<Self> {
        let client = Arc::new(EnvProver::new());
        let cloned_client = client.clone();

        // Setup the proving key and verification key.
        let (pk, vk) = task::spawn_blocking(move || {
            let (pk, vk) = cloned_client.setup(&elf);
            (pk, vk)
        })
        .await?;

        Ok(Self {
            provider,
            host_executor: HostExecutor::new(block_execution_strategy_factory),
            client,
            pk: Arc::new(pk),
            vk: Arc::new(vk),
            hooks,
            config,
            phantom: Default::default(),
        })
    }

    pub async fn wait_for_block(&self, block_number: u64) -> eyre::Result<()> {
        while self.provider.get_block_number().await? < block_number {
            sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }
}

impl<P, N, NP, F, H> BlockExecutor for FullExecutor<P, N, NP, F, H>
where
    P: Provider<N> + Clone,
    N: Network,
    NP: NodePrimitives
        + DeserializeOwned
        + IntoPrimitives<N>
        + IntoInput
        + ValidateBlockPostExecution,
    F: BlockExecutionStrategyFactory<Primitives = NP>,
    H: ExecutionHooks,
{
    async fn execute(&self, block_number: u64) -> eyre::Result<()> {
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
                        self.config.opcode_tracking,
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

        self.process_client(client_input, &self.hooks, self.config.prove).await?;

        Ok(())
    }

    fn client(&self) -> Arc<EnvProver> {
        self.client.clone()
    }

    fn pk(&self) -> Arc<SP1ProvingKey> {
        self.pk.clone()
    }

    fn vk(&self) -> Arc<SP1VerifyingKey> {
        self.vk.clone()
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
    client: Arc<EnvProver>,
    pk: Arc<SP1ProvingKey>,
    vk: Arc<SP1VerifyingKey>,
    hooks: H,
    prove: bool,
    phantom: PhantomData<NP>,
}

impl<NP, H> CachedExecutor<NP, H>
where
    NP: NodePrimitives + DeserializeOwned,
    H: ExecutionHooks,
{
    pub async fn try_new(
        elf: Vec<u8>,
        hooks: H,
        cache_dir: PathBuf,
        chain_id: u64,
        prove: bool,
    ) -> eyre::Result<Self> {
        let client = Arc::new(EnvProver::new());
        let cloned_client = client.clone();

        // Setup the proving key and verification key.
        let (pk, vk) = task::spawn_blocking(move || {
            let (pk, vk) = cloned_client.setup(&elf);
            (pk, vk)
        })
        .await?;

        Ok(Self {
            cache_dir,
            chain_id,
            client,
            pk: Arc::new(pk),
            vk: Arc::new(vk),
            hooks,
            prove,
            phantom: Default::default(),
        })
    }
}

impl<NP, H> BlockExecutor for CachedExecutor<NP, H>
where
    NP: NodePrimitives + DeserializeOwned,
    H: ExecutionHooks,
{
    async fn execute(&self, block_number: u64) -> eyre::Result<()> {
        let client_input =
            try_load_input_from_cache::<NP>(&self.cache_dir, self.chain_id, block_number)?
                .ok_or(eyre::eyre!("No cached input found"))?;

        self.process_client(client_input, &self.hooks, self.prove).await
    }

    fn client(&self) -> Arc<EnvProver> {
        self.client.clone()
    }

    fn pk(&self) -> Arc<SP1ProvingKey> {
        self.pk.clone()
    }

    fn vk(&self) -> Arc<SP1VerifyingKey> {
        self.vk.clone()
    }
}

impl<NP: NodePrimitives, H: ExecutionHooks> Debug for CachedExecutor<NP, H> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachedExecutor").field("cache_dir", &self.cache_dir).finish()
    }
}

// As the block execution in the zkVM is a long-running, blocking task, we need to run it in a
// separate thread.
async fn execute_client(
    number: u64,
    client: Arc<EnvProver>,
    pk: Arc<SP1ProvingKey>,
    stdin: SP1Stdin,
) -> eyre::Result<(SP1Stdin, eyre::Result<(SP1PublicValues, ExecutionReport)>)> {
    task::spawn_blocking(move || {
        info_span!("execute_client", number).in_scope(|| {
            let result = client.execute(&pk.elf, &stdin).run();
            (stdin, result.map_err(|err| eyre::eyre!("{err}")))
        })
    })
    .await
    .map_err(|err| eyre::eyre!("{err}"))
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

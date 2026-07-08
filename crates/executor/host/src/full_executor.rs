use std::{
    fmt::{Debug, Formatter},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use alloy_provider::Provider;
use either::Either;
use eyre::bail;
use reth_primitives_traits::NodePrimitives;
use rsp_client_executor::io::{ClientExecutorInput, CommittedHeader};
use serde::de::DeserializeOwned;
use sp1_sdk::{Elf, ProveRequest, Prover, ProvingKey, SP1ProofMode, SP1Stdin, SP1VerifyingKey};
use tokio::time::sleep;
use tracing::{info, warn, Instrument};

use crate::{Config, ExecutionHooks, ExecutorComponents, HostError, HostExecutor};

pub type EitherExecutor<C, P> = Either<FullExecutor<C, P>, CachedExecutor<C>>;

pub async fn build_executor<C, P>(
    elf: Vec<u8>,
    provider: Option<P>,
    evm_config: C::EvmConfig,
    client: Arc<C::Prover>,
    hooks: C::Hooks,
    config: Config,
) -> eyre::Result<EitherExecutor<C, P>>
where
    C: ExecutorComponents,
    P: Provider<C::Network> + Clone + std::fmt::Debug,
{
    if let Some(provider) = provider {
        return Ok(Either::Left(
            FullExecutor::try_new(provider, elf, evm_config, client, hooks, config).await?,
        ));
    }

    if let Some(cache_dir) = &config.cache_dir {
        return Ok(Either::Right(
            CachedExecutor::try_new(elf, client, hooks, cache_dir.clone(), config).await?,
        ));
    }

    bail!("Either a RPC URL or a cache dir must be provided")
}

pub trait BlockExecutor<C: ExecutorComponents> {
    #[allow(async_fn_in_trait)]
    async fn execute(&self, block_number: u64) -> eyre::Result<()>;

    fn client(&self) -> Arc<C::Prover>;

    fn pk(&self) -> Arc<<C::Prover as Prover>::ProvingKey>;

    fn vk(&self) -> Arc<SP1VerifyingKey>;

    fn config(&self) -> &Config;

    /// Serialize a client input into zkVM stdin.
    fn build_stdin(
        &self,
        client_input: &ClientExecutorInput<C::Primitives>,
    ) -> eyre::Result<SP1Stdin> {
        let mut stdin = SP1Stdin::new();
        stdin.write_vec(bincode::serialize(client_input)?);
        Ok(stdin)
    }

    /// If `stdin_dir` is configured, persist a block's zkVM stdin as `{stdin_dir}/{block}.bin`
    /// (bincode). This builds up a reproducible, prover-ready test corpus of real blocks; a no-op
    /// when `stdin_dir` is unset.
    fn save_stdin(&self, block_number: u64, stdin: &SP1Stdin) -> eyre::Result<()> {
        let Some(dir) = self.config().stdin_dir.as_ref() else { return Ok(()) };

        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("{block_number}.bin"));
        let mut file = std::fs::File::create(&path)?;
        bincode::serialize_into(&mut file, stdin)?;

        info!("Saved stdin to {}", path.display());
        Ok(())
    }

    /// Execute the program in the zkVM to validate it, firing `on_execution_end` with the
    /// measured execution duration.
    ///
    /// Returns the cycle count from the execution report. This is the only source of the cycle
    /// count: the local prover does not expose it from `prove` in SP1 v6.
    ///
    /// The execution runs on a dedicated task, so when the caller drives this concurrently with
    /// proving (see the ethproofs pipeline) the execution runs in parallel with the prover.
    #[allow(async_fn_in_trait)]
    async fn execute_input(
        &self,
        client_input: &ClientExecutorInput<C::Primitives>,
        stdin: SP1Stdin,
        hooks: &C::Hooks,
    ) -> eyre::Result<u64> {
        let elf = self.pk().elf().clone();
        let client = self.client();
        let execution_start = Instant::now();
        // `in_current_span` keeps the caller's span (e.g. the pipeline's per-block span with
        // its block number) on the logs of the spawned execution task.
        let (mut public_values, execution_report) =
            tokio::spawn(async move { client.execute(elf, stdin).await }.in_current_span())
                .await
                .map_err(|err| eyre::eyre!("execution task failed: {err}"))?
                .map_err(|err| eyre::eyre!("{err}"))?;
        let execution_duration = execution_start.elapsed();

        // Read the block header and check it matches the input.
        let header = public_values.read::<CommittedHeader>().header;
        let executed_block_hash = header.hash_slow();
        let input_block_hash = client_input.current_block.header.hash_slow();

        if input_block_hash != executed_block_hash {
            return Err(HostError::HeaderMismatch(executed_block_hash, input_block_hash))?;
        }

        info!(?executed_block_hash, duration = ?execution_duration, "Execution successful");

        hooks
            .on_execution_end::<C::Primitives>(
                &client_input.current_block,
                &execution_report,
                execution_duration,
            )
            .await?;

        Ok(execution_report.total_instruction_count())
    }

    /// Generate a proof for a prepared stdin, firing `on_proving_start` and returning the
    /// serialized proof together with how long proving took.
    ///
    /// This does not fire `on_proving_end` — the caller does, once it also has the cycle count
    /// (which comes from execution). Splitting it out lets execution and proving run
    /// concurrently: proving does not depend on the validation-execute, and they use different
    /// resources (GPU vs CPU).
    #[allow(async_fn_in_trait)]
    async fn prove_only(
        &self,
        block_number: u64,
        stdin: SP1Stdin,
        prove_mode: SP1ProofMode,
        hooks: &C::Hooks,
    ) -> eyre::Result<(Vec<u8>, Duration)> {
        info!("Starting proof generation");

        let proving_start = Instant::now();
        hooks.on_proving_start(block_number).await?;
        let client = self.client();
        let pk = self.pk();

        let proof = client
            .prove(pk.as_ref(), stdin)
            .mode(prove_mode)
            .await
            .map_err(|err| eyre::eyre!("{err}"))?;

        let proving_duration = proving_start.elapsed();
        let proof_bytes = bincode::serialize(&proof.proof)?;

        info!(duration = ?proving_duration, "Proof successfully generated!");

        Ok((proof_bytes, proving_duration))
    }

    #[allow(async_fn_in_trait)]
    async fn process_client(
        &self,
        client_input: ClientExecutorInput<C::Primitives>,
        hooks: &C::Hooks,
    ) -> eyre::Result<()> {
        let stdin = self.build_stdin(&client_input)?;
        self.save_stdin(client_input.current_block.number, &stdin)?;

        match self.config().prove_mode {
            Some(prove_mode) => {
                let cycle_count = if self.config().skip_client_execution {
                    info!("Client execution skipped");
                    None
                } else {
                    Some(self.execute_input(&client_input, stdin.clone(), hooks).await?)
                };

                let block_number = client_input.current_block.number;
                let (proof_bytes, proving_duration) =
                    self.prove_only(block_number, stdin, prove_mode, hooks).await?;

                hooks
                    .on_proving_end(
                        block_number,
                        &proof_bytes,
                        self.vk().as_ref(),
                        cycle_count,
                        proving_duration,
                    )
                    .await?;
            }
            None => {
                if self.config().skip_client_execution {
                    info!("Client execution skipped");
                } else {
                    self.execute_input(&client_input, stdin, hooks).await?;
                }
            }
        }

        Ok(())
    }

    /// Like [`Self::process_client`], but runs the validation-execute and proving
    /// *concurrently*: proving does not depend on the execute, and they use different resources
    /// (GPU vs CPU), so this keeps the execute off the proving critical path. Used by
    /// long-running proving services (the ethproofs pipeline); the sequential
    /// [`Self::process_client`] is preferable for one-shot runs, where an execution failure
    /// should prevent spending GPU time on a doomed proof.
    ///
    /// Both sides are always run to completion even if the other fails, so a fast failure on
    /// one side still yields the other's metrics; every failure is surfaced rather than letting
    /// one error mask the other.
    #[allow(async_fn_in_trait)]
    async fn process_client_concurrent(
        &self,
        client_input: ClientExecutorInput<C::Primitives>,
        hooks: &C::Hooks,
    ) -> eyre::Result<()> {
        let stdin = self.build_stdin(&client_input)?;
        let block_number = client_input.current_block.number;
        self.save_stdin(block_number, &stdin)?;

        let Some(prove_mode) = self.config().prove_mode else {
            // Execute-only (proving disabled) — still runs for validation and metrics.
            if self.config().skip_client_execution {
                info!("Client execution skipped");
            } else {
                self.execute_input(&client_input, stdin, hooks).await?;
            }
            return Ok(());
        };

        if self.config().skip_client_execution {
            info!("Client execution skipped");
            let (proof_bytes, proving_duration) =
                self.prove_only(block_number, stdin, prove_mode, hooks).await?;
            hooks
                .on_proving_end(
                    block_number,
                    &proof_bytes,
                    self.vk().as_ref(),
                    None,
                    proving_duration,
                )
                .await?;
            return Ok(());
        }

        let (execution, proof) = tokio::join!(
            self.execute_input(&client_input, stdin.clone(), hooks),
            self.prove_only(block_number, stdin, prove_mode, hooks),
        );

        match (execution, proof) {
            (Ok(cycle_count), Ok((proof_bytes, proving_duration))) => {
                hooks
                    .on_proving_end(
                        block_number,
                        &proof_bytes,
                        self.vk().as_ref(),
                        Some(cycle_count),
                        proving_duration,
                    )
                    .await?;

                Ok(())
            }
            // At least one side failed: surface every failure instead of letting one error
            // mask the other.
            (execution, proof) => {
                let mut errors = Vec::new();

                if let Err(err) = execution {
                    errors.push(format!("execution failed: {err}"));
                }
                match proof {
                    Err(err) => errors.push(format!("proving failed: {err}")),
                    // The proof completed but the execution didn't, so the cycle count
                    // required for submission is missing and the proof cannot be used.
                    Ok(_) => warn!(
                        block_number,
                        "discarding a completed proof because the validation-execute failed"
                    ),
                }

                bail!(errors.join("; "));
            }
        }
    }
}

impl<C, P> BlockExecutor<C> for EitherExecutor<C, P>
where
    C: ExecutorComponents,
    P: Provider<C::Network> + Clone + std::fmt::Debug,
{
    async fn execute(&self, block_number: u64) -> eyre::Result<()> {
        match self {
            Either::Left(ref executor) => executor.execute(block_number).await,
            Either::Right(ref executor) => executor.execute(block_number).await,
        }
    }

    fn client(&self) -> Arc<C::Prover> {
        match self {
            Either::Left(ref executor) => executor.client.clone(),
            Either::Right(ref executor) => executor.client.clone(),
        }
    }

    fn pk(&self) -> Arc<<C::Prover as Prover>::ProvingKey> {
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

    fn config(&self) -> &Config {
        match self {
            Either::Left(executor) => executor.config(),
            Either::Right(executor) => executor.config(),
        }
    }
}

pub struct FullExecutor<C, P>
where
    C: ExecutorComponents,
    P: Provider<C::Network> + Clone + std::fmt::Debug,
{
    provider: P,
    host_executor: HostExecutor<C::EvmConfig, C::ChainSpec>,
    client: Arc<C::Prover>,
    pk: Arc<<C::Prover as Prover>::ProvingKey>,
    vk: Arc<SP1VerifyingKey>,
    hooks: C::Hooks,
    config: Config,
}

impl<C, P> FullExecutor<C, P>
where
    C: ExecutorComponents,
    P: Provider<C::Network> + Clone + std::fmt::Debug,
{
    pub async fn try_new(
        provider: P,
        elf: Vec<u8>,
        evm_config: C::EvmConfig,
        client: Arc<C::Prover>,
        hooks: C::Hooks,
        config: Config,
    ) -> eyre::Result<Self> {
        // Setup the proving key.
        let pk =
            client.setup(Elf::from(elf.as_slice())).await.map_err(|err| eyre::eyre!("{err}"))?;
        let vk = pk.verifying_key().clone();

        Ok(Self {
            provider,
            host_executor: HostExecutor::new(
                evm_config,
                Arc::new(C::try_into_chain_spec(&config.genesis)?),
            ),
            client,
            pk: Arc::new(pk),
            vk: Arc::new(vk),
            hooks,
            config,
        })
    }

    pub async fn wait_for_block(&self, block_number: u64) -> eyre::Result<()> {
        let block_number = block_number.into();

        while self.provider.get_block_by_number(block_number).await?.is_none() {
            sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }

    /// The hooks attached to this executor.
    pub fn hooks(&self) -> &C::Hooks {
        &self.hooks
    }

    /// Fetch the client input for a block, either from the on-disk cache or by executing the
    /// block on the host against the RPC provider (caching the result when a cache dir is set).
    pub async fn fetch_client_input(
        &self,
        block_number: u64,
    ) -> eyre::Result<ClientExecutorInput<C::Primitives>> {
        let client_input_from_cache = self.config.cache_dir.as_ref().and_then(|cache_dir| {
            match try_load_input_from_cache::<C::Primitives>(
                cache_dir,
                self.config.chain.id(),
                block_number,
            ) {
                Ok(client_input) => client_input,
                Err(e) => {
                    warn!("Failed to load input from cache: {}", e);
                    None
                }
            }
        });

        let client_input = match client_input_from_cache {
            Some(mut client_input_from_cache) => {
                // Override opcode tracking from cache by the setting provided by the user
                client_input_from_cache.opcode_tracking = self.config.opcode_tracking;
                client_input_from_cache
            }
            None => {
                // Execute the host.
                let client_input = self
                    .host_executor
                    .execute(
                        block_number,
                        &self.provider,
                        self.config.genesis.clone(),
                        self.config.custom_beneficiary,
                        self.config.opcode_tracking,
                        self.config.state_backend,
                    )
                    .await?;

                if let Some(ref cache_dir) = self.config.cache_dir {
                    let input_folder = cache_dir.join(format!("input/{}", self.config.chain.id()));
                    if !input_folder.exists() {
                        std::fs::create_dir_all(&input_folder)?;
                    }

                    let input_path = input_folder.join(format!("{block_number}.bin"));
                    let mut cache_file = std::fs::File::create(input_path)?;

                    bincode::serialize_into(&mut cache_file, &client_input)?;
                }

                client_input
            }
        };

        Ok(client_input)
    }
}

impl<C, P> BlockExecutor<C> for FullExecutor<C, P>
where
    C: ExecutorComponents,
    P: Provider<C::Network> + Clone + std::fmt::Debug,
{
    async fn execute(&self, block_number: u64) -> eyre::Result<()> {
        self.hooks.on_execution_start(block_number).await?;

        let client_input = self.fetch_client_input(block_number).await?;

        self.process_client(client_input, &self.hooks).await?;

        Ok(())
    }

    fn client(&self) -> Arc<C::Prover> {
        self.client.clone()
    }

    fn pk(&self) -> Arc<<C::Prover as Prover>::ProvingKey> {
        self.pk.clone()
    }

    fn vk(&self) -> Arc<SP1VerifyingKey> {
        self.vk.clone()
    }

    fn config(&self) -> &Config {
        &self.config
    }
}

impl<C, P> Debug for FullExecutor<C, P>
where
    C: ExecutorComponents,
    P: Provider<C::Network> + Clone + std::fmt::Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FullExecutor").field("config", &self.config).finish()
    }
}

pub struct CachedExecutor<C>
where
    C: ExecutorComponents,
{
    cache_dir: PathBuf,
    client: Arc<C::Prover>,
    pk: Arc<<C::Prover as Prover>::ProvingKey>,
    vk: Arc<SP1VerifyingKey>,
    hooks: C::Hooks,
    config: Config,
}

impl<C> CachedExecutor<C>
where
    C: ExecutorComponents,
{
    pub async fn try_new(
        elf: Vec<u8>,
        client: Arc<C::Prover>,
        hooks: C::Hooks,
        cache_dir: PathBuf,
        config: Config,
    ) -> eyre::Result<Self> {
        // Setup the proving key.
        let pk =
            client.setup(Elf::from(elf.as_slice())).await.map_err(|err| eyre::eyre!("{err}"))?;
        let vk = pk.verifying_key().clone();

        Ok(Self { cache_dir, client, pk: Arc::new(pk), vk: Arc::new(vk), hooks, config })
    }
}

impl<C> BlockExecutor<C> for CachedExecutor<C>
where
    C: ExecutorComponents,
{
    async fn execute(&self, block_number: u64) -> eyre::Result<()> {
        let client_input = try_load_input_from_cache::<C::Primitives>(
            &self.cache_dir,
            self.config.chain.id(),
            block_number,
        )?
        .ok_or(eyre::eyre!("No cached input found"))?;

        self.process_client(client_input, &self.hooks).await
    }

    fn client(&self) -> Arc<C::Prover> {
        self.client.clone()
    }

    fn pk(&self) -> Arc<<C::Prover as Prover>::ProvingKey> {
        self.pk.clone()
    }

    fn vk(&self) -> Arc<SP1VerifyingKey> {
        self.vk.clone()
    }

    fn config(&self) -> &Config {
        &self.config
    }
}

impl<C> Debug for CachedExecutor<C>
where
    C: ExecutorComponents,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachedExecutor").field("cache_dir", &self.cache_dir).finish()
    }
}

fn try_load_input_from_cache<P: NodePrimitives + DeserializeOwned>(
    cache_dir: &Path,
    chain_id: u64,
    block_number: u64,
) -> eyre::Result<Option<ClientExecutorInput<P>>> {
    let cache_path = cache_dir.join(format!("input/{chain_id}/{block_number}.bin"));

    if cache_path.exists() {
        // TODO: prune the cache if invalid instead
        let mut cache_file = std::fs::File::open(cache_path)?;
        let client_input = bincode::deserialize_from(&mut cache_file)?;

        Ok(Some(client_input))
    } else {
        Ok(None)
    }
}

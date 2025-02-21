use csv::WriterBuilder;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{fs::OpenOptions, path::PathBuf, time::SystemTime};

use alloy_consensus::BlockHeader;
use alloy_provider::{Network, RootProvider};
use reth_evm::execute::BlockExecutionStrategyFactory;
use reth_primitives::NodePrimitives;
use reth_primitives_traits::BlockBody;
use rsp_primitives::genesis::Genesis;
use rsp_rpc_db::RpcDb;

use rsp_client_executor::{io::ClientExecutorInput, IntoInput, IntoPrimitives};
use rsp_host_executor::HostExecutor;
use sp1_sdk::{include_elf, network::B256, ExecutionReport, ProverClient, SP1Stdin};

use crate::{
    cli::{HostArgs, ProviderConfig},
    db::{self, ProvableBlock},
    eth_proofs::EthProofsClient,
};

#[derive(Serialize, Deserialize)]
struct ExecutionReportData {
    chain_id: u64,
    block_number: u64,
    gas_used: u64,
    tx_count: usize,
    number_cycles: u64,
    number_syscalls: u64,
    bn_add_cycles: u64,
    bn_mul_cycles: u64,
    bn_pair_cycles: u64,
    kzg_point_eval_cycles: u64,
}

pub async fn execute<N, NP, F>(
    args: HostArgs,
    provider_config: ProviderConfig,
    genesis: Genesis,
    eth_proofs_client: Option<EthProofsClient>,
    block_execution_strategy_factory: F,
    is_optimism: bool,
) -> eyre::Result<()>
where
    N: Network,
    NP: NodePrimitives + DeserializeOwned,
    F: BlockExecutionStrategyFactory<Primitives = NP>,
    F::Primitives: IntoPrimitives<N> + IntoInput,
{
    // Initialize PostgreSQL connection pool
    let pool = db::init_db_pool(&args.db_url).await?;

    // Initialize database schema
    db::init_db_schema(&pool).await?;

    let start_time = db::system_time_to_timestamp(SystemTime::now());

    // Create new block record
    let block = ProvableBlock {
        block_number: args.block_number.unwrap() as i64,
        status: "queued".to_string(),
        gas_used: 0,
        tx_count: 0,
        num_cycles: 0,
        start_time: Some(start_time),
        end_time: None,
    };
    db::insert_block(&pool, &block).await?;

    let client_input_from_cache = try_load_input_from_cache::<NP>(
        args.cache_dir.as_ref(),
        provider_config.chain_id,
        args.block_number.unwrap(),
    )?;

    let client_input = match (client_input_from_cache, provider_config.rpc_url) {
        (Some(client_input_from_cache), _) => client_input_from_cache,
        (None, Some(rpc_url)) => {
            let provider = RootProvider::<N>::new_http(rpc_url);

            // Setup the host executor.
            let host_executor = HostExecutor::new(block_execution_strategy_factory);

            let rpc_db = RpcDb::new(provider.clone(), args.block_number.unwrap() - 1);

            // Execute the host.
            let client_input = host_executor
                .execute(
                    args.block_number.unwrap(),
                    &rpc_db,
                    &provider,
                    genesis,
                    args.custom_beneficiary,
                )
                .await?;

            if let Some(ref cache_dir) = args.cache_dir {
                let input_folder = cache_dir.join(format!("input/{}", provider_config.chain_id));
                if !input_folder.exists() {
                    std::fs::create_dir_all(&input_folder)?;
                }

                let input_path = input_folder.join(format!("{}.bin", args.block_number.unwrap()));
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
    let client = ProverClient::from_env();

    // Setup the proving key and verification key.
    let (pk, vk) = client.setup(if is_optimism {
        include_elf!("rsp-client-op")
    } else {
        include_elf!("rsp-client")
    });

    // Execute the block inside the zkVM.
    let mut stdin = SP1Stdin::new();
    let buffer = bincode::serialize(&client_input).unwrap();

    stdin.write_vec(buffer);

    // Only execute the program.
    let (mut public_values, execution_report) = client.execute(&pk.elf, &stdin).run().unwrap();

    // Read the block hash.
    let block_hash = public_values.read::<B256>();
    println!("success: block_hash={block_hash}");

    let executed_block = client_input.clone().current_block;

    if eth_proofs_client.is_none() {
        // Process the execute report, print it out, and save data to a CSV specified by
        // report_path.
        process_execution_report(
            provider_config.chain_id,
            client_input,
            &execution_report,
            args.report_path.clone(),
        )?;
    }

    let end_time = db::system_time_to_timestamp(SystemTime::now());

    // Update the block status in PostgreSQL
    db::update_block_status(
        &pool,
        args.block_number.unwrap() as i64,
        executed_block.header.gas_used() as i64,
        executed_block.body.transaction_count() as i64,
        execution_report.total_instruction_count() as i64,
        end_time,
    )
    .await?;

    if args.prove {
        println!("Starting proof generation.");

        if let Some(eth_proofs_client) = &eth_proofs_client {
            eth_proofs_client.proving(args.block_number.unwrap()).await?;
        }

        let start = std::time::Instant::now();
        let proof = client.prove(&pk, &stdin).compressed().run().expect("Proving should work.");
        let proof_bytes = bincode::serialize(&proof.proof).unwrap();
        let elapsed = start.elapsed().as_secs_f32();

        if let Some(eth_proofs_client) = &eth_proofs_client {
            eth_proofs_client
                .proved(&proof_bytes, args.block_number.unwrap(), &execution_report, elapsed, &vk)
                .await?;
        }
    }

    Ok(())
}

pub fn try_load_input_from_cache<P: NodePrimitives + DeserializeOwned>(
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

/// Given an execution report, print it out and write it to a CSV specified by report_path.
pub fn process_execution_report<P: NodePrimitives>(
    chain_id: u64,
    client_input: ClientExecutorInput<P>,
    execution_report: &ExecutionReport,
    report_path: PathBuf,
) -> eyre::Result<()> {
    println!("\nExecution report:\n{}", execution_report);

    let executed_block = client_input.current_block;
    let block_number = executed_block.header.number();
    let gas_used = executed_block.header.gas_used();
    let tx_count = executed_block.body.transaction_count();
    let number_cycles = execution_report.total_instruction_count();
    let number_syscalls = execution_report.total_syscall_count();

    let bn_add_cycles = *execution_report.cycle_tracker.get("precompile-bn-add").unwrap_or(&0);
    let bn_mul_cycles = *execution_report.cycle_tracker.get("precompile-bn-mul").unwrap_or(&0);
    let bn_pair_cycles = *execution_report.cycle_tracker.get("precompile-bn-pair").unwrap_or(&0);
    let kzg_point_eval_cycles =
        *execution_report.cycle_tracker.get("precompile-kzg-point-evaluation").unwrap_or(&0);

    // TODO: we can track individual syscalls in our CSV once we have sp1-core as a dependency
    // let keccak_count = execution_report.syscall_counts.get(SyscallCode::KECCAK_PERMUTE);
    // let secp256k1_decompress_count =
    //     execution_report.syscall_counts.get(SyscallCode::SECP256K1_DECOMPRESS);

    let report_data = ExecutionReportData {
        chain_id,
        block_number,
        gas_used,
        tx_count,
        number_cycles,
        number_syscalls,
        bn_add_cycles,
        bn_mul_cycles,
        bn_pair_cycles,
        kzg_point_eval_cycles,
    };

    // Open the file for appending or create it if it doesn't exist
    let file = OpenOptions::new().append(true).create(true).open(report_path)?;

    // Check if the file is empty
    let file_is_empty = file.metadata()?.len() == 0;

    let mut writer = WriterBuilder::new().has_headers(file_is_empty).from_writer(file);
    writer.serialize(report_data)?;
    writer.flush()?;

    Ok(())
}

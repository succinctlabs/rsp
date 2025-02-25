use alloy_consensus::{Block, BlockHeader};
use reth_primitives::NodePrimitives;
use rsp_host_executor::ExecutionHooks;
use serde::{Deserialize, Serialize};
use sp1_sdk::ExecutionReport;
use sqlx::{postgres::PgPoolOptions, Pool, Postgres};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct PersistToPostgres {
    pub db_pool: Pool<Postgres>,
}

impl PersistToPostgres {
    pub fn new(db_pool: Pool<Postgres>) -> Self {
        Self { db_pool }
    }
}

impl ExecutionHooks for PersistToPostgres {
    async fn on_execution_start(&self, block_number: u64) -> eyre::Result<()> {
        insert_block(&self.db_pool, block_number, SystemTime::now()).await?;
        Ok(())
    }

    async fn on_execution_end<P: NodePrimitives>(
        &self,
        executed_block: &Block<P::SignedTx>,
        execution_report: &ExecutionReport,
    ) -> eyre::Result<()> {
        // Update the block status in PostgreSQL
        update_block_status(
            &self.db_pool,
            executed_block.number(),
            executed_block.header.gas_used(),
            executed_block.body.transactions.len(),
            execution_report.total_instruction_count(),
            SystemTime::now(),
        )
        .await?;

        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProvableBlock {
    pub block_number: i64,
    pub status: String,
    pub gas_used: i64,
    pub tx_count: i64,
    pub num_cycles: i64,
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
}

pub async fn build_db_pool(database_url: &str) -> Result<Pool<Postgres>, sqlx::Error> {
    PgPoolOptions::new().max_connections(64).connect(database_url).await
}

pub async fn insert_block(
    pool: &Pool<Postgres>,
    block_number: u64,
    start_time: SystemTime,
) -> Result<(), sqlx::Error> {
    let block = ProvableBlock {
        block_number: block_number as i64,
        status: "queued".to_string(),
        gas_used: 0,
        tx_count: 0,
        num_cycles: 0,
        start_time: Some(system_time_to_timestamp(start_time)),
        end_time: None,
    };

    sqlx::query(
        r#"
        INSERT INTO rsp_blocks
        (block_number, status, gas_used, tx_count, num_cycles, start_time, end_time)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (block_number) 
        DO UPDATE SET
            status = EXCLUDED.status,
            gas_used = EXCLUDED.gas_used,
            tx_count = EXCLUDED.tx_count,
            num_cycles = EXCLUDED.num_cycles,
            start_time = EXCLUDED.start_time,
            end_time = EXCLUDED.end_time
        "#,
    )
    .bind(block.block_number)
    .bind(&block.status)
    .bind(block.gas_used)
    .bind(block.tx_count)
    .bind(block.num_cycles)
    .bind(block.start_time)
    .bind(block.end_time)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn update_block_status(
    pool: &Pool<Postgres>,
    block_number: u64,
    gas_used: u64,
    tx_count: usize,
    num_cycles: u64,
    end_time: SystemTime,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE rsp_blocks
        SET status = 'executed',
            gas_used = $2,
            tx_count = $3,
            num_cycles = $4,
            end_time = $5
        WHERE block_number = $1
        "#,
    )
    .bind(block_number as i64)
    .bind(gas_used as i64)
    .bind(tx_count as i64)
    .bind(num_cycles as i64)
    .bind(system_time_to_timestamp(end_time))
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn update_block_status_as_failed(
    pool: &Pool<Postgres>,
    block_number: u64,
    end_time: SystemTime,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE rsp_blocks
        SET status = 'failed',
            end_time = $2
        WHERE block_number = $1
        "#,
    )
    .bind(block_number as i64)
    .bind(system_time_to_timestamp(end_time))
    .execute(pool)
    .await?;

    Ok(())
}

fn system_time_to_timestamp(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

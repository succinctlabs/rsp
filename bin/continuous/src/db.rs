use std::fmt::Display;

use alloy_consensus::{Block, BlockHeader};
use async_trait::async_trait;
use reth_primitives_traits::NodePrimitives;
use rsp_host_executor::ExecutionHooks;
use sp1_sdk::ExecutionReport;
use sqlx::{
    postgres::PgPoolOptions,
    types::chrono::{NaiveDateTime, Utc},
    Pool, Postgres,
};

pub struct PersistToPostgres<P: NodePrimitives> {
    pub db_pool: Pool<Postgres>,
    _phantom: std::marker::PhantomData<P>,
}

impl<P: NodePrimitives> PersistToPostgres<P> {
    pub fn new(db_pool: Pool<Postgres>) -> Self {
        Self { db_pool, _phantom: std::marker::PhantomData }
    }
}

#[async_trait]
impl<P: NodePrimitives + 'static> ExecutionHooks for PersistToPostgres<P> {
    type Primitives = P;

    async fn on_execution_start(&self, block_number: u64) -> eyre::Result<()> {
        insert_block(&self.db_pool, block_number).await?;
        Ok(())
    }

    async fn on_execution_end(
        &self,
        executed_block: &Block<<Self::Primitives as NodePrimitives>::SignedTx>,
        execution_report: &ExecutionReport,
    ) -> eyre::Result<()> {
        // Update the block status in PostgreSQL
        update_block_status(
            &self.db_pool,
            executed_block.number(),
            executed_block.header.gas_used(),
            executed_block.body.transactions.len(),
            execution_report.total_instruction_count(),
            execution_report.gas.unwrap_or_default(),
        )
        .await?;

        Ok(())
    }
}

#[derive(Debug)]
pub struct ProvableBlock {
    pub block_number: i64,
    pub status: ProvableBlockStatus,
    pub gas_used: i64,
    pub tx_count: i64,
    pub num_cycles: i64,
    pub start_time: Option<NaiveDateTime>,
    pub end_time: Option<NaiveDateTime>,
}

#[derive(Debug)]
pub enum ProvableBlockStatus {
    Queued,
    Executed,
    Failed,
}

impl Display for ProvableBlockStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProvableBlockStatus::Queued => write!(f, "queued"),
            ProvableBlockStatus::Executed => write!(f, "executed"),
            ProvableBlockStatus::Failed => write!(f, "failed"),
        }
    }
}

pub async fn build_db_pool(database_url: &str) -> Result<Pool<Postgres>, sqlx::Error> {
    PgPoolOptions::new().max_connections(64).connect(database_url).await
}

pub async fn insert_block(pool: &Pool<Postgres>, block_number: u64) -> Result<(), sqlx::Error> {
    let now = Utc::now().naive_utc();
    let block = ProvableBlock {
        block_number: block_number as i64,
        status: ProvableBlockStatus::Queued,
        gas_used: 0,
        tx_count: 0,
        num_cycles: 0,
        start_time: Some(now),
        end_time: None,
    };

    sqlx::query!(
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
        block.block_number,
        block.status.to_string(),
        block.gas_used,
        block.tx_count,
        block.num_cycles,
        block.start_time,
        block.end_time
    )
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
    sp1_gas: u64,
) -> Result<(), sqlx::Error> {
    let now = Utc::now().naive_utc();

    sqlx::query!(
        r#"
        UPDATE rsp_blocks
        SET status = $5,
            gas_used = $2,
            tx_count = $3,
            num_cycles = $4,
            end_time = $6,
            sp1_gas = $7
        WHERE block_number = $1
        "#,
        block_number as i64,
        gas_used as i64,
        tx_count as i64,
        num_cycles as i64,
        ProvableBlockStatus::Executed.to_string(),
        now,
        sp1_gas as i64
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn update_block_status_as_failed(
    pool: &Pool<Postgres>,
    block_number: u64,
) -> Result<(), sqlx::Error> {
    let now = Utc::now().naive_utc();
    sqlx::query!(
        r#"
        UPDATE rsp_blocks
        SET status = $2,
            end_time = $3
        WHERE block_number = $1
        "#,
        block_number as i64,
        ProvableBlockStatus::Failed.to_string(),
        now
    )
    .execute(pool)
    .await?;

    Ok(())
}

use serde::{Deserialize, Serialize};
use sqlx::{postgres::PgPoolOptions, Pool, Postgres};
use std::time::{SystemTime, UNIX_EPOCH};

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

pub async fn init_db_pool(db_url: &str) -> Result<Pool<Postgres>, sqlx::Error> {
    let database_url = db_url;
    PgPoolOptions::new().max_connections(64).connect(database_url).await
}

pub async fn init_db_schema(pool: &Pool<Postgres>) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS rsp_blocks (
            block_number BIGINT PRIMARY KEY,
            status VARCHAR(50) NOT NULL,
            gas_used BIGINT NOT NULL,
            tx_count BIGINT NOT NULL,
            num_cycles BIGINT NOT NULL,
            start_time BIGINT,
            end_time BIGINT
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn insert_block(pool: &Pool<Postgres>, block: &ProvableBlock) -> Result<(), sqlx::Error> {
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
    block_number: i64,
    gas_used: i64,
    tx_count: i64,
    num_cycles: i64,
    end_time: i64,
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
    .bind(block_number)
    .bind(gas_used)
    .bind(tx_count)
    .bind(num_cycles)
    .bind(end_time)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn update_block_status_as_failed(
    pool: &Pool<Postgres>,
    block_number: i64,
    end_time: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE rsp_blocks
        SET status = 'failed',
            end_time = $2
        WHERE block_number = $1
        "#,
    )
    .bind(block_number)
    .bind(end_time)
    .execute(pool)
    .await?;

    Ok(())
}

pub fn system_time_to_timestamp(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

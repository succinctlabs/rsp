CREATE TABLE IF NOT EXISTS rsp_blocks (
    block_number BIGINT PRIMARY KEY,
    status VARCHAR(50) NOT NULL,
    gas_used BIGINT NOT NULL,
    tx_count BIGINT NOT NULL,
    num_cycles BIGINT NOT NULL,
    start_time BIGINT,
    end_time BIGINT
);
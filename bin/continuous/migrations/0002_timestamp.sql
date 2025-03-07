ALTER TABLE rsp_blocks
    ALTER COLUMN start_time SET DATA TYPE timestamp 
    USING to_timestamp(start_time);

ALTER TABLE rsp_blocks
    ALTER COLUMN end_time SET DATA TYPE timestamp 
    USING to_timestamp(end_time);
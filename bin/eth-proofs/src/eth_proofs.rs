use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine};
use eyre::eyre;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use rsp_host_executor::ExecutionHooks;
use sp1_sdk::{ExecutionReport, HashableKey, SP1VerifyingKey};
use tracing::error;

#[derive(Debug, Clone)]
pub struct EthProofsClient {
    cluster_id: u64,
    endpoint: String,
    api_token: String,
    client: ClientWithMiddleware,
}

impl EthProofsClient {
    pub fn new(cluster_id: u64, endpoint: String, api_token: String) -> Self {
        let retry_policy = ExponentialBackoff::builder().build_with_max_retries(3);
        let client = ClientBuilder::new(reqwest::Client::new())
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build();

        Self { cluster_id, endpoint, api_token, client }
    }

    pub async fn queued(&self, block_number: u64) {
        let json = serde_json::json!({
            "block_number": block_number,
            "cluster_id": self.cluster_id,
        });

        let this = self.clone();

        // Spawn another task to avoid retries to impact block execution
        tokio::spawn(async move {
            let response = this
                .client
                .post(format!("{}/proofs/queued", this.endpoint))
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", this.api_token))
                .json(&json)
                .send()
                .await
                .map_err(|e| eyre!(e))
                .and_then(|r| r.error_for_status().map_err(|e| eyre!(e)));

            if let Err(err) = response {
                error!("Failed to report proof queuing: {}", err)
            }
        });
    }

    pub async fn proving(&self, block_number: u64) {
        let json = serde_json::json!({
            "block_number": block_number,
            "cluster_id": self.cluster_id,
        });
        let this = self.clone();

        // Spawn another task to avoid retries to impact block execution
        tokio::spawn(async move {
            let response = this
                .client
                .post(format!("{}/proofs/proving", this.endpoint))
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", this.api_token))
                .json(&json)
                .send()
                .await
                .map_err(|e| eyre!(e))
                .and_then(|r| r.error_for_status().map_err(|e| eyre!(e)));

            if let Err(err) = response {
                error!("Failed to report proof proving: {}", err)
            }
        });
    }

    pub async fn proved(
        &self,
        proof_bytes: &[u8],
        block_number: u64,
        execution_report: &ExecutionReport,
        elapsed: f32,
        vk: &SP1VerifyingKey,
    ) {
        let json = serde_json::json!({
            "proof": STANDARD.encode(proof_bytes),
            "block_number": block_number,
            "proving_cycles": execution_report.total_instruction_count(),
            "proving_time": (elapsed * 1000.0) as u64,
            "verifier_id": vk.bytes32(),
            "cluster_id": self.cluster_id,
        });

        let this = self.clone();

        // Spawn another task to avoid retries to impact block execution
        tokio::spawn(async move {
            // Submit proof to the API

            let response = this
                .client
                .post(format!("{}/proofs/proved", this.endpoint))
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", this.api_token))
                .json(&json)
                .send()
                .await
                .map_err(|e| eyre!(e))
                .and_then(|r| r.error_for_status().map_err(|e| eyre!(e)));

            if let Err(err) = response {
                error!("Failed to report proof proving: {}", err)
            }
        });
    }
}

impl ExecutionHooks for EthProofsClient {
    async fn on_execution_start(&self, block_number: u64) -> eyre::Result<()> {
        self.queued(block_number).await;

        Ok(())
    }

    async fn on_proving_start(&self, block_number: u64) -> eyre::Result<()> {
        self.proving(block_number).await;

        Ok(())
    }

    async fn on_proving_end(
        &self,
        block_number: u64,
        proof_bytes: &[u8],
        vk: &SP1VerifyingKey,
        execution_report: &ExecutionReport,
        proving_duration: Duration,
    ) -> eyre::Result<()> {
        self.proved(
            proof_bytes,
            block_number,
            execution_report,
            proving_duration.as_secs_f32(),
            vk,
        )
        .await;

        Ok(())
    }
}

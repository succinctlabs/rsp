use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine};
use eyre::eyre;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use rsp_host_executor::ExecutionHooks;
use sp1_sdk::{HashableKey, SP1VerifyingKey};
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

    /// Fire a `POST {endpoint}/{path}` with the given JSON body in a detached task, so retries
    /// never block block processing. Failures are logged, not propagated.
    fn post(&self, path: &'static str, json: serde_json::Value) {
        let this = self.clone();
        let url = format!("{}/{path}", self.endpoint);

        tokio::spawn(async move {
            let response = this
                .client
                .post(url)
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", this.api_token))
                .json(&json)
                .send()
                .await
                .map_err(|e| eyre!(e))
                .and_then(|r| r.error_for_status().map_err(|e| eyre!(e)));

            if let Err(err) = response {
                error!("Failed to POST to eth-proofs {path}: {err}");
            }
        });
    }

    pub fn queued(&self, block_number: u64) {
        self.post(
            "proofs/queued",
            serde_json::json!({
                "block_number": block_number,
                "cluster_id": self.cluster_id,
            }),
        );
    }

    pub fn proving(&self, block_number: u64) {
        self.post(
            "proofs/proving",
            serde_json::json!({
                "block_number": block_number,
                "cluster_id": self.cluster_id,
            }),
        );
    }

    pub fn proved(
        &self,
        proof_bytes: &[u8],
        block_number: u64,
        cycle_count: u64,
        elapsed: f32,
        vk: &SP1VerifyingKey,
    ) {
        self.post(
            "proofs/proved",
            serde_json::json!({
                "proof": STANDARD.encode(proof_bytes),
                "block_number": block_number,
                "proving_cycles": cycle_count,
                "proving_time": (elapsed * 1000.0) as u64,
                "verifier_id": vk.bytes32(),
                "cluster_id": self.cluster_id,
            }),
        );
    }
}

impl ExecutionHooks for EthProofsClient {
    async fn on_execution_start(&self, block_number: u64) -> eyre::Result<()> {
        self.queued(block_number);

        Ok(())
    }

    async fn on_proving_start(&self, block_number: u64) -> eyre::Result<()> {
        self.proving(block_number);

        Ok(())
    }

    async fn on_proving_end(
        &self,
        block_number: u64,
        proof_bytes: &[u8],
        vk: &SP1VerifyingKey,
        cycle_count: Option<u64>,
        proving_duration: Duration,
    ) -> eyre::Result<()> {
        self.proved(
            proof_bytes,
            block_number,
            cycle_count.ok_or_else(|| eyre!("The cycle count is required"))?,
            proving_duration.as_secs_f32(),
            vk,
        );

        Ok(())
    }
}

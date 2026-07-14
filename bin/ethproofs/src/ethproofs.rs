use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD, Engine};
use eyre::eyre;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use rsp_host_executor::ExecutionHooks;
use sp1_sdk::{HashableKey, SP1VerifyingKey};
use tokio::task::JoinSet;
use tracing::{error, warn, Instrument};

#[derive(Debug, Clone)]
pub struct EthproofsClient {
    cluster_id: u64,
    /// `None` disables submission entirely (no requests are sent). This lets the service run
    /// execution, proving and metrics locally without ethproofs credentials.
    submit: Option<Submit>,
    /// In-flight submission tasks, so shutdown can drain them (see
    /// [`Self::drain_submissions`]) instead of the runtime teardown aborting a half-sent
    /// proof. Shared across clones.
    inflight: Arc<Mutex<JoinSet<()>>>,
}

#[derive(Debug, Clone)]
struct Submit {
    endpoint: String,
    api_token: String,
    client: ClientWithMiddleware,
}

impl EthproofsClient {
    /// Create a client. Submission is enabled only when both `endpoint` and `api_token` are
    /// provided; otherwise the client is a no-op (useful for local testing without credentials).
    pub fn new(cluster_id: u64, endpoint: Option<String>, api_token: Option<String>) -> Self {
        let submit = match (endpoint, api_token) {
            (Some(endpoint), Some(api_token)) => {
                let retry_policy = ExponentialBackoff::builder().build_with_max_retries(3);
                let client = ClientBuilder::new(reqwest::Client::new())
                    .with(RetryTransientMiddleware::new_with_policy(retry_policy))
                    .build();
                Some(Submit { endpoint, api_token, client })
            }
            _ => None,
        };

        Self { cluster_id, submit, inflight: Arc::new(Mutex::new(JoinSet::new())) }
    }

    /// Whether this client will actually submit to ethproofs.
    pub fn is_enabled(&self) -> bool {
        self.submit.is_some()
    }

    /// Wait for in-flight submissions to finish, bounded by `timeout`. Call before exiting so
    /// the runtime teardown doesn't abort a proof submission mid-request.
    pub async fn drain_submissions(&self, timeout: Duration) {
        // Take the set out of the mutex so it isn't held across awaits.
        let mut inflight = std::mem::take(&mut *self.inflight.lock().unwrap());

        if inflight.is_empty() {
            return;
        }

        let drain = async { while inflight.join_next().await.is_some() {} };
        if tokio::time::timeout(timeout, drain).await.is_err() {
            warn!("timed out draining in-flight ethproofs submissions after {timeout:?}");
        }
    }

    /// Fire a `POST {endpoint}/{path}` with the given JSON body in a tracked background task,
    /// so retries never block block processing. A no-op when submission is disabled; failures
    /// are logged, not propagated.
    fn post(&self, path: &'static str, json: serde_json::Value) {
        let Some(submit) = self.submit.clone() else { return };
        let url = format!("{}/{path}", submit.endpoint);

        let mut inflight = self.inflight.lock().unwrap();
        // Reap completed tasks so the set doesn't grow over the service's lifetime.
        while inflight.try_join_next().is_some() {}

        // `in_current_span` keeps the caller's span (the pipeline's per-block span with its
        // block number) on this task's logs, so submission failures are attributable to a block.
        inflight.spawn(
            async move {
                let response = submit
                    .client
                    .post(url)
                    .header("Content-Type", "application/json")
                    .header("Authorization", format!("Bearer {}", submit.api_token))
                    .json(&json)
                    .send()
                    .await
                    .map_err(|e| eyre!(e))
                    .and_then(|r| r.error_for_status().map_err(|e| eyre!(e)));

                if let Err(err) = response {
                    error!("Failed to POST to ethproofs {path}: {err}");
                }
            }
            .in_current_span(),
        );
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

impl ExecutionHooks for EthproofsClient {
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

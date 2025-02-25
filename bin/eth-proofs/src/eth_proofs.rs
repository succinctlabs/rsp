use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine};
use rsp_host_executor::ExecutionHooks;
use sp1_sdk::{ExecutionReport, HashableKey, SP1VerifyingKey};

pub struct EthProofsClient {
    cluster_id: u64,
    endpoint: String,
    api_token: String,
    client: reqwest::Client,
}

impl EthProofsClient {
    pub fn new(cluster_id: u64, endpoint: String, api_token: String) -> Self {
        Self { cluster_id, endpoint, api_token, client: reqwest::Client::new() }
    }

    pub async fn queued(&self, block_number: u64) -> eyre::Result<()> {
        let json = &serde_json::json!({
            "block_number": block_number,
            "cluster_id": self.cluster_id,
        });

        let response = self
            .client
            .post(format!("{}/proofs/queued", self.endpoint))
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", self.api_token))
            .json(json)
            .send()
            .await?;

        println!("Queued submission status: {}", response.status());
        if !response.status().is_success() {
            println!("Error response: {}", response.text().await?);
        }

        Ok(())
    }

    pub async fn proving(&self, block_number: u64) -> eyre::Result<()> {
        let json = &serde_json::json!({
            "block_number": block_number,
            "cluster_id": self.cluster_id,
        });

        let response = self
            .client
            .post(format!("{}/proofs/proving", self.endpoint))
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", self.api_token))
            .json(json)
            .send()
            .await?;

        println!("Proving submission status: {}", response.status());
        if !response.status().is_success() {
            println!("Error response: {}", response.text().await?);
        }

        Ok(())
    }

    pub async fn proved(
        &self,
        proof_bytes: &[u8],
        block_number: u64,
        execution_report: &ExecutionReport,
        elapsed: f32,
        vk: &SP1VerifyingKey,
    ) -> eyre::Result<()> {
        // Submit proof to the API
        let json = &serde_json::json!({
            "proof": STANDARD.encode(proof_bytes),
            "block_number": block_number,
            "proving_cycles": execution_report.total_instruction_count(),
            "proving_time": (elapsed * 1000.0) as u64,
            "verifier_id": vk.bytes32(),
            "cluster_id": self.cluster_id,
        });

        // Save the proof data to a file
        let proof_file_path = "latest_proof.json";
        std::fs::write(proof_file_path, serde_json::to_string_pretty(json)?)?;
        println!("Saved proof data to {}", proof_file_path);

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{}/proofs/proved", self.endpoint))
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", self.api_token))
            .json(json)
            .send()
            .await?;

        println!("Proved submission status: {}", response.status());
        if !response.status().is_success() {
            println!("Error response: {}", response.text().await?);
        }

        Ok(())
    }
}

impl ExecutionHooks for EthProofsClient {
    async fn on_execution_start(&self, block_number: u64) -> eyre::Result<()> {
        self.queued(block_number).await?;

        Ok(())
    }

    async fn on_proving_start(&self, block_number: u64) -> eyre::Result<()> {
        self.proving(block_number).await?;

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
        .await?;

        Ok(())
    }
}

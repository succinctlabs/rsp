//! A typed client for the ethproofs HTTP API (the API-key-authenticated surface).
//!
//! Distinct from the proving service's submission client: admin/CLI operations need
//! request/response semantics with error propagation, whereas the hot path wants background,
//! best-effort POSTs. Only the API-key-accessible endpoints are modelled here; the VK-upload
//! endpoint uses website (Supabase) session auth and is intentionally not covered — see the
//! `gen-vk` command in the `ethproofs-cli` binary.

use eyre::{eyre, Context};
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// A cluster as returned by `GET /clusters`.
#[derive(Debug, Clone, Deserialize)]
pub struct Cluster {
    /// The cluster's index — what the API calls `id` in `/clusters/{id}` paths.
    pub id: u64,
    /// Human-readable display name.
    pub name: String,
    /// Free-form hardware description, if set.
    #[serde(default)]
    pub hardware_description: Option<String>,
    /// The cluster's versions. The numeric ids are what the (session-authenticated) VK-upload
    /// endpoint keys on.
    #[serde(default)]
    pub versions: Vec<ClusterVersion>,
}

/// A single version of a [`Cluster`].
#[derive(Debug, Clone, Deserialize)]
pub struct ClusterVersion {
    /// The version's numeric id.
    pub id: u64,
}

/// Body for `POST /clusters`.
#[derive(Debug, Clone, Serialize)]
pub struct CreateCluster {
    /// Human-readable name (≤50 chars).
    pub name: String,
    /// zkVM version id (see <https://ethproofs.org/docs/zkvms>).
    pub zkvm_version_id: u64,
    /// Number of GPUs; `>1` marks the cluster multi-GPU. Defaults to 1 server-side when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_gpus: Option<u64>,
    /// Free-form hardware description (≤200 chars).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hardware_description: Option<String>,
    /// `"cloud-hosted"` or `"on-prem"`. Defaults to `"cloud-hosted"` server-side when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_type: Option<String>,
}

/// Body for `PATCH /clusters/{id}`. Every field is optional; only the set ones are sent, and the
/// server requires at least one. Changing `zkvm_version_id` or `vk_path` creates a new cluster
/// version.
#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateCluster {
    /// New display name (≤50 chars).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// New GPU count. The server rejects changes that would flip the single/multi-GPU class.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_gpus: Option<u64>,
    /// New hardware description (≤200 chars).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hardware_description: Option<String>,
    /// Whether the cluster is active and can receive proofs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_active: Option<bool>,
    /// New zkVM version id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zkvm_version_id: Option<u64>,
    /// A Supabase storage path to a previously-uploaded verification key. This is only a path;
    /// the VK bytes themselves are set through the website's VK-upload flow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vk_path: Option<String>,
}

/// A typed, response-returning client for the ethproofs API.
#[derive(Debug, Clone)]
pub struct EthproofsApi {
    base_url: String,
    api_token: String,
    client: ClientWithMiddleware,
}

impl EthproofsApi {
    /// Create a client for `base_url` (the endpoint including `/api/v0`), authenticating with
    /// `api_token`. Transient failures are retried with exponential backoff.
    pub fn new(base_url: impl Into<String>, api_token: impl Into<String>) -> Self {
        let retry_policy = ExponentialBackoff::builder().build_with_max_retries(3);
        let client = ClientBuilder::new(reqwest::Client::new())
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build();

        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_token: api_token.into(),
            client,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{path}", self.base_url)
    }

    /// List the clusters owned by the authenticated team.
    pub async fn list_clusters(&self) -> eyre::Result<Vec<Cluster>> {
        let resp = self
            .client
            .get(self.url("clusters"))
            .header("Authorization", format!("Bearer {}", self.api_token))
            .send()
            .await?;

        read_json(resp).await
    }

    /// Create a cluster, returning its new id (index).
    pub async fn create_cluster(&self, body: &CreateCluster) -> eyre::Result<u64> {
        let resp = self
            .client
            .post(self.url("clusters"))
            .header("Authorization", format!("Bearer {}", self.api_token))
            .json(body)
            .send()
            .await?;

        let created: CreatedCluster = read_json(resp).await?;
        Ok(created.id)
    }

    /// Update a cluster's metadata and/or version.
    pub async fn patch_cluster(&self, id: u64, body: &UpdateCluster) -> eyre::Result<()> {
        let resp = self
            .client
            .patch(self.url(&format!("clusters/{id}")))
            .header("Authorization", format!("Bearer {}", self.api_token))
            .json(body)
            .send()
            .await?;

        // The endpoint returns `{ "success": true }`; we only need it to have been a 2xx.
        let _: serde_json::Value = read_json(resp).await?;
        Ok(())
    }
}

/// The response shape of `POST /clusters`.
#[derive(Deserialize)]
struct CreatedCluster {
    id: u64,
}

/// Read a JSON body, turning a non-2xx status into an error that carries the response body (which
/// the API uses to explain validation failures) rather than a bare status code.
async fn read_json<T: DeserializeOwned>(resp: reqwest::Response) -> eyre::Result<T> {
    let status = resp.status();
    let body = resp.text().await?;

    if !status.is_success() {
        return Err(eyre!("ethproofs API returned {status}: {body}"));
    }

    serde_json::from_str(&body)
        .wrap_err_with(|| format!("failed to parse ethproofs response body: {body}"))
}

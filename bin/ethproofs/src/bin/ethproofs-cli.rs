//! `ethproofs-cli` — administrative and utility commands for the ethproofs API.
//!
//! Separate from the long-running `ethproofs` proving service: this drives the request/response
//! parts of the API (clusters) and generates the program's verification-key file for upload.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use ethproofs::api::{CreateCluster, EthproofsApi, UpdateCluster};
use eyre::eyre;
use sp1_sdk::{include_elf, HashableKey, Prover, ProverClient, ProvingKey};

#[derive(Parser)]
#[command(name = "ethproofs-cli", about = "Utilities for the ethproofs API", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Manage clusters.
    #[command(subcommand)]
    Cluster(ClusterCommand),
}

#[derive(Subcommand)]
enum ClusterCommand {
    /// List the clusters owned by the authenticated team.
    List(ApiArgs),
    /// Create a new cluster.
    Create(CreateArgs),
    /// Update a cluster's metadata or version.
    Patch(PatchArgs),
    /// Generate the verification-key file for the `rsp-client` program, ready to upload as a
    /// cluster's VK via the ethproofs website.
    GenVk(GenVkArgs),
    /// Write the raw `rsp-client` guest ELF to a file.
    GenElf(GenElfArgs),
}

/// Endpoint + token shared by the API-backed commands.
#[derive(Args)]
struct ApiArgs {
    /// ETH proofs API endpoint (base URL, including `/api/v0`).
    #[clap(long, env = "ETH_PROOFS_ENDPOINT")]
    endpoint: String,
    /// ETH proofs API token.
    #[clap(long, env = "ETH_PROOFS_API_TOKEN")]
    api_token: String,
}

impl ApiArgs {
    fn client(&self) -> EthproofsApi {
        EthproofsApi::new(self.endpoint.clone(), self.api_token.clone())
    }
}

/// The cluster deployment type accepted by the API.
#[derive(Clone, Copy, ValueEnum)]
enum DeploymentType {
    CloudHosted,
    OnPrem,
}

impl DeploymentType {
    fn as_api(self) -> String {
        match self {
            DeploymentType::CloudHosted => "cloud-hosted",
            DeploymentType::OnPrem => "on-prem",
        }
        .to_string()
    }
}

#[derive(Args)]
struct CreateArgs {
    #[clap(flatten)]
    api: ApiArgs,
    /// Human-readable cluster name (≤50 chars).
    #[clap(long)]
    name: String,
    /// zkVM version id (see https://ethproofs.org/docs/zkvms).
    #[clap(long)]
    zkvm_version_id: u64,
    /// Number of GPUs (>1 marks the cluster multi-GPU).
    #[clap(long)]
    num_gpus: Option<u64>,
    /// Free-form hardware description (≤200 chars).
    #[clap(long)]
    hardware_description: Option<String>,
    /// Deployment type.
    #[clap(long, value_enum)]
    deployment_type: Option<DeploymentType>,
}

#[derive(Args)]
struct PatchArgs {
    #[clap(flatten)]
    api: ApiArgs,
    /// The cluster id (index) to update.
    #[clap(long)]
    id: u64,
    /// New display name (≤50 chars).
    #[clap(long)]
    name: Option<String>,
    /// New GPU count.
    #[clap(long)]
    num_gpus: Option<u64>,
    /// New hardware description (≤200 chars).
    #[clap(long)]
    hardware_description: Option<String>,
    /// Mark the cluster active/inactive.
    #[clap(long)]
    is_active: Option<bool>,
    /// New zkVM version id.
    #[clap(long)]
    zkvm_version_id: Option<u64>,
    /// A Supabase storage path to a previously-uploaded VK. Most users should instead upload the
    /// key produced by `gen-vk` through the website; see that command's output.
    #[clap(long)]
    vk_path: Option<String>,
}

#[derive(Args)]
struct GenVkArgs {
    /// Where to write the 32-byte verification-key file.
    #[clap(long, default_value = "vk.bin")]
    output: PathBuf,
}

#[derive(Args)]
struct GenElfArgs {
    /// Where to write the guest ELF.
    #[clap(long, default_value = "rsp-client.elf")]
    output: PathBuf,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    dotenv::dotenv().ok();
    let cli = Cli::parse();

    match cli.command {
        Command::Cluster(cmd) => match cmd {
            ClusterCommand::List(api) => list_clusters(&api.client()).await?,
            ClusterCommand::Create(args) => create_cluster(args).await?,
            ClusterCommand::Patch(args) => patch_cluster(args).await?,
            ClusterCommand::GenVk(args) => gen_vk(args).await?,
            ClusterCommand::GenElf(args) => gen_elf(args)?,
        },
    }

    Ok(())
}

fn gen_elf(args: GenElfArgs) -> eyre::Result<()> {
    // `include_elf!` embeds the compiled guest program; no prover is needed to just dump it.
    let elf = include_elf!("rsp-client").to_vec();
    std::fs::write(&args.output, &elf)?;
    println!("Wrote {} bytes to {}", elf.len(), args.output.display());
    Ok(())
}

async fn list_clusters(api: &EthproofsApi) -> eyre::Result<()> {
    let clusters = api.list_clusters().await?;

    if clusters.is_empty() {
        println!("No clusters found.");
        return Ok(());
    }

    for c in &clusters {
        let version_ids =
            c.versions.iter().map(|v| v.id.to_string()).collect::<Vec<_>>().join(", ");
        println!(
            "id={}  name={}  hardware={}  version_ids=[{}]",
            c.id,
            c.name,
            c.hardware_description.as_deref().unwrap_or("-"),
            version_ids,
        );
    }

    Ok(())
}

async fn create_cluster(args: CreateArgs) -> eyre::Result<()> {
    let CreateArgs { api, name, zkvm_version_id, num_gpus, hardware_description, deployment_type } =
        args;

    let body = CreateCluster {
        name,
        zkvm_version_id,
        num_gpus,
        hardware_description,
        deployment_type: deployment_type.map(DeploymentType::as_api),
    };

    let id = api.client().create_cluster(&body).await?;
    println!("Created cluster id={id}");
    Ok(())
}

async fn patch_cluster(args: PatchArgs) -> eyre::Result<()> {
    let PatchArgs {
        api,
        id,
        name,
        num_gpus,
        hardware_description,
        is_active,
        zkvm_version_id,
        vk_path,
    } = args;

    let body =
        UpdateCluster { name, num_gpus, hardware_description, is_active, zkvm_version_id, vk_path };

    api.client().patch_cluster(id, &body).await?;
    println!("Updated cluster id={id}");
    Ok(())
}

async fn gen_vk(args: GenVkArgs) -> eyre::Result<()> {
    // The light prover only executes/verifies (it cannot prove), so it initializes fast and needs
    // no GPU. `setup` is deterministic key generation independent of the proving backend, so the
    // vk it returns is byte-identical to the one the CUDA prover uses for submitted proofs.
    // `include_elf!` embeds the compiled `rsp-client` guest program (the same ELF the proving
    // service proves), built by this package's build script.
    let prover = ProverClient::builder().light().build().await;
    let pk = prover.setup(include_elf!("rsp-client")).await.map_err(|err| eyre!("{err}"))?;
    let vk = pk.verifying_key();

    // The ethproofs sp1-hypercube verifier consumes `bincode(vk.hash_koalabear())` (a 32-byte
    // vkey hash) as its vk_bytes; that is exactly the file to upload as the cluster VK.
    let vk_bytes = bincode::serialize(&vk.hash_koalabear())?;
    std::fs::write(&args.output, &vk_bytes)?;

    println!("Wrote {} bytes to {}", vk_bytes.len(), args.output.display());
    println!("vkey bytes32: {}", vk.bytes32());
    println!(
        "Upload this file as the cluster's verification key via the ethproofs website — the \
         VK-upload endpoint is not accessible with an API key."
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// clap's own consistency checks (conflicting flags, bad defaults, etc.).
    #[test]
    fn cli_is_well_formed() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("ethproofs-cli").chain(args.iter().copied()))
    }

    #[test]
    fn gen_vk_defaults_output_and_needs_no_credentials() {
        let cli = parse(&["cluster", "gen-vk"]).unwrap();
        let Command::Cluster(ClusterCommand::GenVk(args)) = cli.command else {
            panic!("expected gen-vk");
        };
        assert_eq!(args.output, PathBuf::from("vk.bin"));
    }

    #[test]
    fn gen_elf_defaults_output_and_needs_no_credentials() {
        let cli = parse(&["cluster", "gen-elf"]).unwrap();
        let Command::Cluster(ClusterCommand::GenElf(args)) = cli.command else {
            panic!("expected gen-elf");
        };
        assert_eq!(args.output, PathBuf::from("rsp-client.elf"));
    }

    #[test]
    fn create_requires_name_and_zkvm_version() {
        // Missing both name and zkvm-version-id.
        assert!(parse(&["cluster", "create", "--endpoint", "e", "--api-token", "t"]).is_err());

        let cli = parse(&[
            "cluster",
            "create",
            "--endpoint",
            "e",
            "--api-token",
            "t",
            "--name",
            "ZKnight-01",
            "--zkvm-version-id",
            "1",
            "--num-gpus",
            "8",
            "--deployment-type",
            "on-prem",
        ])
        .unwrap();

        let Command::Cluster(ClusterCommand::Create(args)) = cli.command else {
            panic!("expected create");
        };
        assert_eq!(args.name, "ZKnight-01");
        assert_eq!(args.zkvm_version_id, 1);
        assert_eq!(args.num_gpus, Some(8));
        assert!(matches!(args.deployment_type, Some(DeploymentType::OnPrem)));
    }

    #[test]
    fn patch_takes_only_the_flags_provided() {
        let cli = parse(&[
            "cluster",
            "patch",
            "--endpoint",
            "e",
            "--api-token",
            "t",
            "--id",
            "3",
            "--is-active",
            "false",
        ])
        .unwrap();

        let Command::Cluster(ClusterCommand::Patch(args)) = cli.command else {
            panic!("expected patch");
        };
        assert_eq!(args.id, 3);
        assert_eq!(args.is_active, Some(false));
        assert_eq!(args.name, None);
        assert_eq!(args.vk_path, None);
    }

    /// Credentials fall back to env vars, so they can be omitted on the command line.
    #[test]
    fn list_reads_credentials_from_env() {
        std::env::set_var("ETH_PROOFS_ENDPOINT", "https://ethproofs.org/api/v0");
        std::env::set_var("ETH_PROOFS_API_TOKEN", "secret");

        let cli = parse(&["cluster", "list"]).unwrap();
        let Command::Cluster(ClusterCommand::List(api)) = cli.command else {
            panic!("expected list");
        };
        assert_eq!(api.endpoint, "https://ethproofs.org/api/v0");
        assert_eq!(api.api_token, "secret");

        std::env::remove_var("ETH_PROOFS_ENDPOINT");
        std::env::remove_var("ETH_PROOFS_API_TOKEN");
    }
}

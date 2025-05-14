use clap::Parser;
use url::Url;

/// The arguments for the cli.
#[derive(Debug, Clone, Parser)]
pub struct Args {
    /// The HTTP rpc url used to fetch data about the block.
    #[arg(long, env)]
    pub http_rpc_url: Url,

    /// The WS rpc url used to fetch data about the block.
    #[arg(long, env)]
    pub ws_rpc_url: Url,

    /// The database connection string.
    #[arg(long, env)]
    pub database_url: String,

    /// The maximum number of concurrent executions.
    #[arg(long, env)]
    pub max_concurrent_executions: usize,

    /// Retry count on failed execution.
    #[arg(long, env, default_value_t = 3)]
    pub execution_retries: usize,

    /// PagerDuty integration key.
    #[arg(long, env)]
    pub pager_duty_integration_key: Option<String>,
}

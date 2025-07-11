use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Initializes the logger with common configuration for RSP applications.
/// 
/// Sets up tracing with appropriate filters for SP1 components.
pub fn init_logger() {
    // Set default log level if not specified
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }

    // Initialize the logger with common filters
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(
            EnvFilter::from_default_env()
                .add_directive("sp1_core_machine=warn".parse().unwrap())
                .add_directive("sp1_core_executor=warn".parse().unwrap())
                .add_directive("sp1_prover=warn".parse().unwrap()),
        )
        .init();
} 
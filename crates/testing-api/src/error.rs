use std::fmt;

/// Errors that can occur when using the testing API.
#[derive(Debug)]
pub enum TestingApiError {
    /// Error communicating with RPC provider.
    RpcError(String),
    /// Error during block execution.
    ExecutionError(String),
    /// File system error.
    FileSystemError(std::io::Error),
    /// Configuration error.
    ConfigError(String),
    /// Feature not enabled error.
    FeatureNotEnabled(&'static str),
}

impl fmt::Display for TestingApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TestingApiError::RpcError(msg) => write!(f, "RPC error: {}", msg),
            TestingApiError::ExecutionError(msg) => write!(f, "Execution error: {}", msg),
            TestingApiError::FileSystemError(err) => write!(f, "File system error: {}", err),
            TestingApiError::ConfigError(msg) => write!(f, "Configuration error: {}", msg),
            TestingApiError::FeatureNotEnabled(feature) => {
                write!(f, "Feature not enabled: {}. Enable the feature flag to use this functionality.", feature)
            }
        }
    }
}

impl std::error::Error for TestingApiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TestingApiError::FileSystemError(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for TestingApiError {
    fn from(err: std::io::Error) -> Self {
        TestingApiError::FileSystemError(err)
    }
}

impl From<eyre::Report> for TestingApiError {
    fn from(err: eyre::Report) -> Self {
        TestingApiError::ExecutionError(err.to_string())
    }
}

impl From<url::ParseError> for TestingApiError {
    fn from(err: url::ParseError) -> Self {
        TestingApiError::ConfigError(format!("Invalid URL: {}", err))
    }
}

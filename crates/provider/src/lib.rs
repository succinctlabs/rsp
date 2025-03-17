use alloy_json_rpc::RpcError;
use alloy_provider::{Network, RootProvider};
use alloy_rpc_client::RpcClient;
use alloy_transport::{
    layers::{RateLimitRetryPolicy, RetryBackoffLayer, RetryPolicy},
    TransportError, TransportErrorKind,
};
use url::Url;

pub fn create_provider<N: Network>(rpc_url: Url) -> RootProvider<N> {
    let retry_layer =
        RetryBackoffLayer::new_with_policy(3, 1000, 100, ServerErrorRetryPolicy::default());
    let client = RpcClient::builder().layer(retry_layer).http(rpc_url);

    RootProvider::new(client)
}

#[derive(Debug, Copy, Clone, Default)]
struct ServerErrorRetryPolicy(RateLimitRetryPolicy);

impl RetryPolicy for ServerErrorRetryPolicy {
    fn should_retry(&self, error: &TransportError) -> bool {
        if self.0.should_retry(error) {
            return true;
        }

        if let RpcError::Transport(TransportErrorKind::HttpError(http_error)) = error {
            if http_error.status >= 500 && http_error.status < 600 {
                return true;
            }
        }

        false
    }

    fn backoff_hint(&self, error: &TransportError) -> Option<std::time::Duration> {
        self.0.backoff_hint(error)
    }
}

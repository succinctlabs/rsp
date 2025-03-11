//! Can't use `pagerduty-rs` because the library is unmaintained.

use serde::Serialize;
use tracing::error;

const PAGER_DUTY_ENDPOINT: &str = "https://events.pagerduty.com/v2";

#[derive(Debug)]
pub struct AlertingClient {
    client: reqwest::Client,
    routing_key: String,
}
impl AlertingClient {
    pub fn new(routing_key: String) -> Self {
        Self { client: reqwest::Client::new(), routing_key }
    }

    /// Send an alert to the PageDuty endpoint.
    pub async fn send_alert(&self, summary: String) {
        let alert = PagerDutyAlert {
            payload: PagerDutyAlertPayload {
                summary,
                severity: "error".to_string(),
                source: "RSP".to_string(),
            },
            routing_key: self.routing_key.clone(),
            event_action: "trigger".to_string(),
        };

        match self.client.post(format!("{}/enqueue", PAGER_DUTY_ENDPOINT)).json(&alert).send().await
        {
            Ok(response) => {
                if let Err(err) = response.error_for_status() {
                    error!("PG returned an error: {err}");
                }
            }
            Err(err) => error!("Error sending alert: {err}"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct PagerDutyAlert {
    payload: PagerDutyAlertPayload,
    routing_key: String,
    event_action: String,
}

#[derive(Debug, Clone, Serialize)]
struct PagerDutyAlertPayload {
    summary: String,
    severity: String,
    source: String,
}

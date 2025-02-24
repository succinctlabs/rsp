use serde::Serialize;
use tracing::error;

const PAGER_DUTY_ENDPOINT: &str = "https://events.pagerduty.com/v2";

/// Send an alert to the PageDuty endpoint.
pub async fn send_alert(client: &reqwest::Client, summary: String, routing_key: String) {
    let alert = PagerDutyAlert {
        payload: PagerDutyAlertPayload {
            summary,
            severity: "error".to_string(),
            source: "RSP".to_string(),
        },
        routing_key,
        event_action: "trigger".to_string(),
    };

    match client.post(format!("{}/enqueue", PAGER_DUTY_ENDPOINT)).json(&alert).send().await {
        Ok(response) => {
            if let Err(err) = response.error_for_status() {
                error!("PG returned an error: {err}");
            }
        }
        Err(err) => error!("Error sending alert: {err}"),
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

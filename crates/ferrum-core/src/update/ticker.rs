use super::Updater;
use std::time::Duration;
use tokio::task::JoinHandle;

const FIRST_CHECK: Duration = Duration::from_secs(60);
const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Checks a minute after start and daily after that; a failed check is a log line, not a stop.
pub fn spawn(updater: Updater) -> JoinHandle<()> {
    tokio::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            if let Err(e) = updater.tick().await {
                tracing::warn!(error = ?e, "update check failed");
            }
            tokio::time::sleep(INTERVAL).await;
        }
    })
}

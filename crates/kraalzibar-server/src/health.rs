use tonic_health::pb::health_server::HealthServer;
use tonic_health::server::HealthReporter;

pub fn create_health_service() -> (
    HealthReporter,
    HealthServer<impl tonic_health::pb::health_server::Health>,
) {
    tonic_health::server::health_reporter()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_reporter_can_set_serving() {
        let (mut reporter, _service) = create_health_service();
        reporter
            .set_serving::<tonic_health::pb::health_server::HealthServer<tonic_health::server::HealthService>>()
            .await;
    }
}

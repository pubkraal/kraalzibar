use kraalzibar_core::tuple::{SnapshotToken, TenantId};

pub fn audit_schema_write(
    tenant_id: &TenantId,
    force: bool,
    breaking_changes_overridden: bool,
    token: Option<&str>,
) {
    tracing::info!(
        target: "audit",
        event = "schema_write",
        tenant_id = %tenant_id,
        force = force,
        breaking_changes_overridden = breaking_changes_overridden,
        snapshot_token = token.unwrap_or(""),
        "schema written"
    );
}

pub fn audit_relationship_write(
    tenant_id: &TenantId,
    write_count: usize,
    delete_count: usize,
    token: &SnapshotToken,
) {
    tracing::info!(
        target: "audit",
        event = "relationship_write",
        tenant_id = %tenant_id,
        write_count = write_count,
        delete_count = delete_count,
        snapshot_token = token.value(),
        "relationships written"
    );
}

pub fn audit_auth_success(tenant_id: &TenantId, key_id: &str) {
    tracing::info!(
        target: "audit",
        event = "auth_success",
        tenant_id = %tenant_id,
        key_id = key_id,
        "authentication succeeded"
    );
}

pub fn audit_auth_failure(reason: &str, key_id: Option<&str>) {
    tracing::warn!(
        target: "audit",
        event = "auth_failure",
        reason = reason,
        key_id = key_id.unwrap_or("unknown"),
        "authentication failed"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::layer::SubscriberExt;

    #[derive(Debug)]
    struct CapturedEvent {
        target: String,
        _message: String,
        fields: Vec<(String, String)>,
    }

    struct TestLayer {
        events: Arc<Mutex<Vec<CapturedEvent>>>,
    }

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for TestLayer {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut fields = Vec::new();
            let mut visitor = FieldVisitor(&mut fields);
            event.record(&mut visitor);

            let message = fields
                .iter()
                .find(|(k, _)| k == "message")
                .map(|(_, v)| v.clone())
                .unwrap_or_default();

            self.events.lock().unwrap().push(CapturedEvent {
                target: event.metadata().target().to_string(),
                _message: message,
                fields,
            });
        }
    }

    struct FieldVisitor<'a>(&'a mut Vec<(String, String)>);

    impl tracing::field::Visit for FieldVisitor<'_> {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0
                .push((field.name().to_string(), format!("{value:?}")));
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.push((field.name().to_string(), value.to_string()));
        }

        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            self.0.push((field.name().to_string(), value.to_string()));
        }

        fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
            self.0.push((field.name().to_string(), value.to_string()));
        }
    }

    fn with_test_subscriber<F: FnOnce()>(f: F) -> Vec<CapturedEvent> {
        let events = Arc::new(Mutex::new(Vec::new()));
        let layer = TestLayer {
            events: Arc::clone(&events),
        };
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, f);
        Arc::try_unwrap(events).unwrap().into_inner().unwrap()
    }

    fn has_field(event: &CapturedEvent, key: &str, value: &str) -> bool {
        event.fields.iter().any(|(k, v)| k == key && v == value)
    }

    #[test]
    fn audit_schema_write_emits_event_with_correct_fields() {
        let tenant_id = TenantId::new(uuid::Uuid::nil());
        let events = with_test_subscriber(|| {
            audit_schema_write(&tenant_id, true, true, Some("tok_123"));
        });

        assert_eq!(events.len(), 1);
        assert!(has_field(&events[0], "event", "schema_write"));
        assert!(has_field(&events[0], "force", "true"));
        assert!(has_field(&events[0], "breaking_changes_overridden", "true"));
        assert!(has_field(&events[0], "snapshot_token", "tok_123"));
    }

    #[test]
    fn audit_relationship_write_emits_event_with_count_and_token() {
        let tenant_id = TenantId::new(uuid::Uuid::nil());
        let token = SnapshotToken::new(42);
        let events = with_test_subscriber(|| {
            audit_relationship_write(&tenant_id, 3, 1, &token);
        });

        assert_eq!(events.len(), 1);
        assert!(has_field(&events[0], "event", "relationship_write"));
        assert!(has_field(&events[0], "write_count", "3"));
        assert!(has_field(&events[0], "delete_count", "1"));
        assert!(has_field(&events[0], "snapshot_token", "42"));
    }

    #[test]
    fn audit_auth_success_includes_tenant_and_key_id() {
        let tenant_id = TenantId::new(uuid::Uuid::nil());
        let events = with_test_subscriber(|| {
            audit_auth_success(&tenant_id, "key_abc");
        });

        assert_eq!(events.len(), 1);
        assert!(has_field(&events[0], "event", "auth_success"));
        assert!(has_field(&events[0], "key_id", "key_abc"));
    }

    #[test]
    fn audit_auth_failure_does_not_include_secret() {
        let events = with_test_subscriber(|| {
            audit_auth_failure("invalid key format", None);
        });

        assert_eq!(events.len(), 1);
        let all_values: Vec<&str> = events[0].fields.iter().map(|(_, v)| v.as_str()).collect();
        for val in &all_values {
            assert!(
                !val.contains("secret"),
                "audit event should not contain secret: {val}"
            );
        }
    }

    #[test]
    fn audit_auth_failure_includes_reason() {
        let events = with_test_subscriber(|| {
            audit_auth_failure("unknown api key", Some("key_xyz"));
        });

        assert_eq!(events.len(), 1);
        assert!(has_field(&events[0], "reason", "unknown api key"));
        assert!(has_field(&events[0], "key_id", "key_xyz"));
    }

    #[test]
    fn audit_events_use_target_audit() {
        let tenant_id = TenantId::new(uuid::Uuid::nil());
        let token = SnapshotToken::new(1);
        let events = with_test_subscriber(|| {
            audit_schema_write(&tenant_id, false, false, None);
            audit_relationship_write(&tenant_id, 1, 0, &token);
            audit_auth_success(&tenant_id, "k");
            audit_auth_failure("bad", None);
        });

        assert_eq!(events.len(), 4);
        for event in &events {
            assert_eq!(
                event.target, "audit",
                "event target should be 'audit', got '{}'",
                event.target
            );
        }
    }
}

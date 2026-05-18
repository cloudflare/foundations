use std::sync::{Arc, Mutex};

use foundations::telemetry::TestTelemetryContext;
use foundations::telemetry::log::error;
use foundations::telemetry::settings::LoggingSettings;
use slog::{Drain, Never, OwnedKVList, Record};

struct CapturingDrain {
    messages: Arc<Mutex<Vec<String>>>,
}

impl Drain for CapturingDrain {
    type Ok = ();
    type Err = Never;

    fn log(&self, record: &Record, _: &OwnedKVList) -> Result<(), Never> {
        self.messages
            .lock()
            .unwrap()
            .push(format!("{}", record.msg()));
        Ok(())
    }
}

#[foundations::telemetry::with_test_telemetry(test)]
fn custom_drain_receives_log_records(mut ctx: TestTelemetryContext) {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let drain = CapturingDrain {
        messages: messages.clone(),
    };

    ctx.set_custom_log_drain(LoggingSettings::default(), Arc::new(drain));

    error!("hello from custom drain");

    let msgs = messages.lock().unwrap();
    assert!(
        msgs.iter().any(|m| m.contains("hello from custom drain")),
        "custom drain did not receive the expected log record; got: {msgs:?}"
    );
}

use std::{io::Write, sync::Mutex};

use serde_json::{Map, Value};
use tracing::{
    Level,
    field::{Field, Visit},
};
use tracing_subscriber::layer::{Context, Layer};

fn level_to_pino(level: &Level) -> u8 {
    match *level {
        Level::ERROR => 50,
        Level::WARN => 40,
        Level::INFO => 30,
        Level::DEBUG => 20,
        Level::TRACE => 10,
    }
}

struct PinoVisitor {
    fields: Map<String, Value>,
    msg: String,
}

impl Visit for PinoVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.msg = value.to_string();
        } else {
            self.fields
                .insert(field.name().to_string(), Value::String(value.to_string()));
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.msg = format!("{value:?}");
        } else {
            self.fields.insert(
                field.name().to_string(),
                Value::String(format!("{value:?}")),
            );
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields
            .insert(field.name().to_string(), Value::from(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .insert(field.name().to_string(), Value::from(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.fields
            .insert(field.name().to_string(), Value::from(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields
            .insert(field.name().to_string(), Value::from(value));
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.fields
            .insert(field.name().to_string(), Value::String(value.to_string()));
    }
}

pub struct PinoLayer<W: Write + Send + Sync + 'static> {
    writer: Mutex<W>,
}

impl<W: Write + Send + Sync + 'static> PinoLayer<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer: Mutex::new(writer),
        }
    }
}

impl<S, W> Layer<S> for PinoLayer<W>
where
    S: tracing::Subscriber,
    W: Write + Send + Sync + 'static,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = PinoVisitor {
            fields: Map::new(),
            msg: String::new(),
        };
        event.record(&mut visitor);

        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let mut record = serde_json::json!({
        "level": level_to_pino(event.metadata().level()),
            "time": time,
            "msg": visitor.msg,
            "target": event.metadata().target(),
        });

        if let Value::Object(obj) = &mut record {
            for (k, v) in visitor.fields {
                obj.insert(k, v);
            }
        }

        let line = serde_json::to_string(&record).unwrap_or_default();
        if let Ok(mut guard) = self.writer.lock() {
            writeln!(guard, "{line}").ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_to_pino_maps_correctly() {
        assert_eq!(level_to_pino(&Level::ERROR), 50);
        assert_eq!(level_to_pino(&Level::WARN), 40);
        assert_eq!(level_to_pino(&Level::INFO), 30);
        assert_eq!(level_to_pino(&Level::DEBUG), 20);
        assert_eq!(level_to_pino(&Level::TRACE), 10);
    }
}

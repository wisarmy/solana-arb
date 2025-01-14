use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub fn init(enable_console: bool) {
    let file_appender = RollingFileAppender::new(Rotation::DAILY, "logs", "app.log");
    let file_layer = tracing_subscriber::fmt::layer().with_writer(file_appender);

    let registry = tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with(file_layer);

    if enable_console {
        registry.with(tracing_subscriber::fmt::layer()).init();
    } else {
        registry.init();
    }
}

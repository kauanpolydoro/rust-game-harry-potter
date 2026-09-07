use tracing::Subscriber;
use tracing_subscriber::{EnvFilter, filter::filter_fn, fmt::MakeWriter, prelude::*};

/// Builds the production JSON subscriber with a writer supplied by the host.
/// Dependency targets stay disabled even when the requested verbosity is trace.
pub fn tracing_subscriber<W>(writer: W, filter: EnvFilter) -> impl Subscriber + Send + Sync
where
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    tracing_subscriber::registry().with(filter).with(
        tracing_subscriber::fmt::layer()
            .json()
            .with_writer(writer)
            .with_filter(filter_fn(|metadata| {
                metadata.target() == "harry_potter_server"
                    || metadata.target().starts_with("harry_potter_server::")
            })),
    )
}

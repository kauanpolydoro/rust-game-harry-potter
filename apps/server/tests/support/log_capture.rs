use std::{
    io::Write,
    sync::{Arc, Mutex, OnceLock},
};

#[derive(Clone, Default)]
pub(crate) struct LogCapture {
    bytes: Arc<Mutex<Vec<u8>>>,
    start: usize,
}

impl LogCapture {
    pub(crate) fn start() -> Self {
        static CAPTURE: OnceLock<LogCapture> = OnceLock::new();
        let capture = CAPTURE
            .get_or_init(|| {
                let capture = Self::default();
                let writer = capture.clone();
                tracing::subscriber::set_global_default(harry_potter_server::tracing_subscriber(
                    move || writer.clone(),
                    tracing_subscriber::EnvFilter::new("trace"),
                ))
                .unwrap();
                capture
            })
            .clone();
        let start = capture.bytes.lock().unwrap().len();
        Self {
            bytes: capture.bytes,
            start,
        }
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8(self.bytes.lock().unwrap()[self.start..].to_vec()).unwrap()
    }
}

impl Write for LogCapture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.bytes.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

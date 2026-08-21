use ayni_core::CancellationToken;
use signal_hook::SigId;
use signal_hook::consts::SIGINT;
use signal_hook::low_level::unregister;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const SIGNAL_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Bridges the process SIGINT flag into Ayni's cooperative cancellation token.
///
/// `signal-hook` keeps the signal handler async-signal-safe by writing only an
/// atomic flag. A short-lived bridge thread performs the ordinary Rust method
/// call outside the signal handler and is joined when the operation ends.
pub(crate) struct SignalCancellation {
    token: CancellationToken,
    interrupted: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    signal_id: Option<SigId>,
    bridge: Option<JoinHandle<()>>,
}

impl SignalCancellation {
    pub(crate) fn install() -> Result<Self, String> {
        let token = CancellationToken::default();
        let interrupted = Arc::new(AtomicBool::new(false));
        let signal_id = signal_hook::flag::register(SIGINT, Arc::clone(&interrupted))
            .map_err(|error| format!("failed to install Ctrl-C handler: {error}"))?;
        let stop = Arc::new(AtomicBool::new(false));
        let bridge_token = token.clone();
        let bridge_interrupted = Arc::clone(&interrupted);
        let bridge_stop = Arc::clone(&stop);
        let bridge = thread::Builder::new()
            .name(String::from("ayni-sigint"))
            .spawn(move || {
                while !bridge_stop.load(Ordering::Acquire) {
                    if bridge_interrupted.load(Ordering::Acquire) {
                        bridge_token.cancel();
                        return;
                    }
                    thread::park_timeout(SIGNAL_POLL_INTERVAL);
                }
            })
            .map_err(|error| {
                unregister(signal_id);
                format!("failed to start Ctrl-C bridge: {error}")
            })?;
        Ok(Self {
            token,
            interrupted,
            stop,
            signal_id: Some(signal_id),
            bridge: Some(bridge),
        })
    }

    #[must_use]
    pub(crate) fn token(&self) -> CancellationToken {
        self.token.clone()
    }

    #[must_use]
    pub(crate) fn interrupted(&self) -> bool {
        self.interrupted.load(Ordering::Acquire)
    }
}

impl Drop for SignalCancellation {
    fn drop(&mut self) {
        if let Some(signal_id) = self.signal_id.take() {
            unregister(signal_id);
        }
        self.stop.store(true, Ordering::Release);
        if let Some(bridge) = self.bridge.take() {
            bridge.thread().unpark();
            let _ = bridge.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SignalCancellation;
    use signal_hook::consts::SIGINT;
    use std::process::Command;
    use std::time::{Duration, Instant};

    #[test]
    fn sigint_cancels_the_operation_token() {
        let executable = std::env::current_exe().expect("test executable");
        let output = Command::new(executable)
            .args([
                "--ignored",
                "--exact",
                "ui::cancellation::tests::fixture_raises_sigint",
                "--nocapture",
            ])
            .output()
            .expect("signal fixture starts");
        assert!(
            output.status.success(),
            "signal fixture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    #[ignore]
    fn fixture_raises_sigint() {
        let cancellation = SignalCancellation::install().expect("signal handler");
        signal_hook::low_level::raise(SIGINT).expect("raise SIGINT");
        let started = Instant::now();
        while !cancellation.token().is_cancelled() {
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "SIGINT was not bridged to cancellation"
            );
            std::thread::yield_now();
        }
        assert!(cancellation.interrupted());
    }
}

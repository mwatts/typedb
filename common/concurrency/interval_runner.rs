/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use std::{
    sync::mpsc::{sync_channel, RecvTimeoutError, SyncSender},
    thread,
    time::Duration,
};

#[derive(Debug)]
pub struct IntervalRunner {
    /// `None` when the runner is disabled (no background thread was spawned).
    shutdown_sink: Option<SyncSender<SyncSender<()>>>,
}

impl IntervalRunner {
    const ZERO_DURATION: Duration = Duration::from_secs(0);

    pub fn new(action: impl FnMut() + Send + 'static, interval: Duration) -> Self {
        Self::new_with_initial_delay(action, interval, Self::ZERO_DURATION)
    }

    pub fn new_with_initial_delay(
        mut action: impl FnMut() + Send + 'static,
        interval: Duration,
        initial_delay: Duration,
    ) -> Self {
        let (shutdown_sender, shutdown_receiver) = sync_channel::<SyncSender<()>>(1);
        thread::spawn(move || {
            match shutdown_receiver.recv_timeout(initial_delay) {
                Ok(done_sender) => {
                    drop(action);
                    done_sender.send(()).unwrap();
                    return;
                }
                Err(RecvTimeoutError::Timeout) => (),
                Err(RecvTimeoutError::Disconnected) => return, // TODO log?
            }

            loop {
                action();
                match shutdown_receiver.recv_timeout(interval) {
                    Ok(done_sender) => {
                        drop(action);
                        done_sender.send(()).unwrap();
                        break;
                    }
                    Err(RecvTimeoutError::Timeout) => (),
                    Err(RecvTimeoutError::Disconnected) => break, // TODO log?
                }
            }
        });
        Self { shutdown_sink: Some(shutdown_sender) }
    }

    /// Creates a disabled runner (no background thread). Useful when periodic
    /// tasks should be skipped, e.g. in testing scenarios.
    pub fn disabled() -> Self {
        Self { shutdown_sink: None }
    }

    /// Creates a runner if `interval` is `Some`, otherwise returns a disabled runner.
    pub fn maybe_new(interval: Option<Duration>, action: impl FnMut() + Send + 'static) -> Self {
        match interval {
            Some(interval) => Self::new(action, interval),
            None => Self::disabled(),
        }
    }

    /// Creates a runner with initial delay equal to the interval if `interval` is `Some`,
    /// otherwise returns a disabled runner.
    pub fn maybe_new_with_initial_delay(
        interval: Option<Duration>,
        action: impl FnMut() + Send + 'static,
    ) -> Self {
        match interval {
            Some(interval) => Self::new_with_initial_delay(action, interval, interval),
            None => Self::disabled(),
        }
    }
}

impl Drop for IntervalRunner {
    fn drop(&mut self) {
        let Some(ref shutdown_sink) = self.shutdown_sink else {
            return;
        };
        let (done_sender, done_receiver) = sync_channel(1);
        // The background thread may have panicked, dropping its receiver.
        // In that case, send() fails — move on instead of panicking.
        if shutdown_sink.send(done_sender).is_err() {
            return;
        }
        // Give the thread a bounded amount of time to finish. If it's stuck or
        // panicked after receiving the signal, we must not hang the caller.
        let _ = done_receiver.recv_timeout(Duration::from_secs(5));
    }
}

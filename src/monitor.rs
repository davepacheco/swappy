//! Monitor subsystem
//!
//! Functions that expect to take a while and cause interesting effects on the
//! system can enable the monitor to print stats once / second and disable the
//! monitor to stop printing stats.

use crate::bytesize_display::ByteSizeDisplayGiB;
use crate::kstat::kstat_read_physmem;
use crate::swap::AnonInfo;
use anyhow::Context;
use std::sync::mpsc::RecvTimeoutError;

/// Handle for the monitor
// This is essentially a client that sends messages over a channel to the
// monitor thread and in some cases receives responses back.
pub struct Monitor {
    #[allow(dead_code)]
    monitor_thread: std::thread::JoinHandle<Result<(), anyhow::Error>>,
    monitor_tx: std::sync::mpsc::SyncSender<MonitorMessage>,
}

impl Monitor {
    /// Starts a background thread for monitoring and returns a [`Monitor`]
    /// handle that can be used to turn monitoring on or off
    pub fn new() -> Monitor {
        let (monitor_tx, monitor_rx) = std::sync::mpsc::sync_channel(4);
        Monitor {
            monitor_thread: std::thread::spawn(move || {
                monitor_thread(monitor_rx)
            }),
            monitor_tx,
        }
    }

    /// Enable monitoring
    ///
    /// This causes the background thread to start collecting and printing stats
    /// once per second.
    pub fn enable(&self) {
        if let Err(error) = self.monitor_tx.send(MonitorMessage::StartStats) {
            // This is likely that the other thread panicked.
            eprintln!("warning: failed to enable monitor: {:#}", error);
        }
    }

    /// Disable monitoring
    ///
    /// This causes the background thread to stop collecting and printing stats.
    /// When this function returns, no more stats will be printed.
    pub fn disable(&self) {
        // Create a channel (functioning as a oneshot) for the monitor thread to
        // let us know when it's done.  We'll wait for the response.  If we
        // didn't do this, then it's possible that one last stat line would be
        // printed after we return.  For the user, this would be an annoying
        // virtual artifact where they got a prompt, then got a bunch of extra
        // output.
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        if let Err(error) = self.monitor_tx.send(MonitorMessage::StopStats(tx))
        {
            // This is likely that the other thread panicked.
            eprintln!("warning: failed to disable monitor: {:#}", error);
        }
        if let Err(error) = rx.recv() {
            // This is likely that the other thread panicked.
            eprintln!("warning: failed to wait for monitor: {:#}", error);
        }
    }
}

/// Messages sent to start/stop the monitor
enum MonitorMessage {
    /// Start collecting and printing stats
    StartStats,

    /// Stop collecting and printing stats and send an ACK message when done
    StopStats(std::sync::mpsc::SyncSender<()>),
}

/// Background thread that implements the monitor
fn monitor_thread(
    rx: std::sync::mpsc::Receiver<MonitorMessage>,
) -> Result<(), anyhow::Error> {
    loop {
        // Wait indefinitely to be told to start monitoring.
        match rx.recv().context("waiting for StartStats")? {
            MonitorMessage::StopStats(_) => panic!("stats already stopped"),
            MonitorMessage::StartStats => (),
        }

        // Now we're in monitor mode.  Print a header row.  Then we'll wait
        // again on the channel until we're told to stop.  The only difference
        // is that we wait with a timeout.  If we hit the timeout, we fetch and
        // print stats and then try again.

        println!(
            "{:5} {:10} {:9} {:10}",
            "FREE", "SWAP_ALLOC", "SWAP_RESV", "SWAP_TOTAL"
        );

        loop {
            match rx.recv_timeout(std::time::Duration::from_secs(1)) {
                Err(RecvTimeoutError::Timeout) => monitor_print(),
                Err(error) => {
                    return Err(error).context("waiting for StopStats")
                }
                Ok(MonitorMessage::StartStats) => {
                    panic!("stats already started")
                }
                Ok(MonitorMessage::StopStats(tx)) => {
                    tx.send(()).context("confirming StopStats")?;
                    break;
                }
            }
        }
    }
}

/// Invoked once / second while the monitor is enabled
fn monitor_print() {
    if let Err(error) = monitor_print_stats().context("monitor_print()") {
        eprintln!("warning: {:#}", error);
    }
}

/// The meat of `monitor_print()`, which is separated for easier error handling
fn monitor_print_stats() -> Result<(), anyhow::Error> {
    let kstat = kstat_rs::Ctl::new().context("initializing kstat")?;
    let physmem = kstat_read_physmem(&kstat).context("kstat_read_physmem")?;
    let swapinfo = AnonInfo::fetch()?;

    // TODO add kmem reap, arc reap, pageout activity

    println!(
        "{:>5} {:>10} {:>9} {:>10}",
        ByteSizeDisplayGiB(physmem.freemem).to_string(),
        ByteSizeDisplayGiB(swapinfo.allocated()).to_string(),
        ByteSizeDisplayGiB(swapinfo.reserved()).to_string(),
        ByteSizeDisplayGiB(swapinfo.total()).to_string(),
    );

    Ok(())
}

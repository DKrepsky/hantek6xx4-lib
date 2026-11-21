//! Notifications when a scope is plugged in or removed.
//!
//! A [`DeviceMonitor`] watches the USB bus on a background thread and tells the application about every scope that
//! appears or disappears. It uses libusb hotplug support where the platform has it and falls back to polling the
//! device list where it does not, so the application sees the same events either way.
//!
//! # A scope appears twice when it is plugged in
//!
//! The scope re-enumerates itself about 1.2 s after it is powered: it shows up on the bus, disappears and shows up
//! again with a new address. Opening it during that window fails or can leave it unresponsive until it is replugged. The
//! monitor therefore holds back a new connection until the scope has **settled**, that is until it has stayed
//! connected for [`MonitorOptions::settle`] (2 s by default). A scope that disappears before that is never reported.
//! Scopes that are already connected when the monitor starts, and removals, are reported immediately.
//!
//! ```no_run
//! use hantek6xx4_lib::{DeviceEvent, DeviceMonitor, Scope};
//!
//! # fn main() -> hantek6xx4_lib::Result<()> {
//! // observer: the closure runs on the monitor thread for every event
//! let monitor = DeviceMonitor::start(|event| match event {
//!     DeviceEvent::Connected(info) => println!("connected: {info}"),
//!     DeviceEvent::Disconnected(info) => println!("disconnected: {info}"),
//! })?;
//!
//! // or take the events from a channel and handle them on your own thread
//! let (_monitor, events) = DeviceMonitor::channel()?;
//! for event in events.iter().take(1) {
//!     if let DeviceEvent::Connected(info) = event {
//!         let mut scope = Scope::open_device(info)?;
//!         println!("{:?}", scope.info());
//!     }
//! }
//! drop(monitor); // stops watching
//! # Ok(()) }
//! ```

use crate::error::{Error, Result};
use crate::transport::{DeviceInfo, UsbTransport, PID, VID};
use rusb::{Context, Device, Hotplug, HotplugBuilder, UsbContext};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// A scope appeared on or disappeared from the USB bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceEvent {
    /// A scope is connected and ready to be opened. Reported for the scopes already plugged in when the monitor starts,
    /// and then for every scope that is plugged in and stays connected for the settle time.
    Connected(DeviceInfo),
    /// A scope that was reported as connected was removed. The [`DeviceInfo`] is the one the matching `Connected`
    /// event carried.
    Disconnected(DeviceInfo),
}

impl DeviceEvent {
    /// The device the event is about.
    pub fn device(&self) -> DeviceInfo {
        match *self {
            DeviceEvent::Connected(d) | DeviceEvent::Disconnected(d) => d,
        }
    }
}

/// How the monitor learns about changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorMode {
    /// libusb reports changes as they happen.
    Hotplug,
    /// The device list is read at a fixed interval and compared with the previous one.
    Polling,
}

/// Default interval of the polling fallback.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Default settle time: longer than the 1.2 s the scope takes to re-enumerate after power-up.
pub const DEFAULT_SETTLE: Duration = Duration::from_secs(2);

/// How often the background thread checks whether it has been asked to stop and whether a connection has settled.
const TICK: Duration = Duration::from_millis(50);

/// Settings of a [`DeviceMonitor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorOptions {
    /// A newly plugged-in scope is reported only after it has stayed connected this long; one that disappears sooner is
    /// never reported. Defaults to [`DEFAULT_SETTLE`]. Zero reports every appearance at once.
    pub settle: Duration,
    /// Poll the device list at this interval instead of using libusb hotplug notifications. `None` uses hotplug where
    /// the platform supports it and polls every [`DEFAULT_POLL_INTERVAL`] where it does not.
    pub polling: Option<Duration>,
}

impl Default for MonitorOptions {
    fn default() -> Self {
        Self {
            settle: DEFAULT_SETTLE,
            polling: None,
        }
    }
}

/// Watches for scopes being plugged in and removed. Dropping the monitor stops the background thread.
///
/// The handler is called on the monitor's own thread, one event at a time, in the order the events happened. Keep it
/// short, or hand the event over to another thread (see [`DeviceMonitor::channel`]); while it runs, no further events
/// are delivered. Do not drop the monitor from inside the handler.
pub struct DeviceMonitor {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    mode: MonitorMode,
}

impl DeviceMonitor {
    /// Start watching with the default [`MonitorOptions`] and call `handler` for every event. Scopes that are already
    /// connected are reported first.
    pub fn start<F>(handler: F) -> Result<Self>
    where
        F: FnMut(DeviceEvent) + Send + 'static,
    {
        Self::start_with(MonitorOptions::default(), handler)
    }

    /// Start watching with the given options.
    pub fn start_with<F>(options: MonitorOptions, handler: F) -> Result<Self>
    where
        F: FnMut(DeviceEvent) + Send + 'static,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel::<Result<MonitorMode>>();
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("hantek6xx4-monitor".into())
            .spawn(move || match options.polling {
                Some(interval) => run_polling(
                    UsbTransport::devices,
                    interval,
                    options.settle,
                    &thread_stop,
                    handler,
                    ready_tx,
                ),
                None if rusb::has_hotplug() => run_hotplug(options.settle, &thread_stop, handler, ready_tx),
                None => run_polling(
                    UsbTransport::devices,
                    DEFAULT_POLL_INTERVAL,
                    options.settle,
                    &thread_stop,
                    handler,
                    ready_tx,
                ),
            })
            .map_err(|e| Error::Protocol(format!("could not start the monitor thread: {e}")))?;
        match ready_rx.recv() {
            Ok(Ok(mode)) => Ok(Self {
                stop,
                thread: Some(thread),
                mode,
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err(Error::Protocol("the monitor thread ended unexpectedly".into())),
        }
    }

    /// Like [`DeviceMonitor::start`] but always polls the device list every `interval` instead of using libusb hotplug.
    pub fn start_polling<F>(handler: F, interval: Duration) -> Result<Self>
    where
        F: FnMut(DeviceEvent) + Send + 'static,
    {
        Self::start_with(
            MonitorOptions {
                polling: Some(interval),
                ..MonitorOptions::default()
            },
            handler,
        )
    }

    /// Start watching with the default options and receive the events from a channel instead of through a callback.
    ///
    /// The channel closes when the monitor is dropped.
    pub fn channel() -> Result<(Self, Receiver<DeviceEvent>)> {
        Self::channel_with(MonitorOptions::default())
    }

    /// Like [`DeviceMonitor::channel`] with the given options.
    pub fn channel_with(options: MonitorOptions) -> Result<(Self, Receiver<DeviceEvent>)> {
        let (tx, rx) = mpsc::channel();
        // a send only fails once the receiver is gone, in which case nobody is listening any more
        let monitor = Self::start_with(options, move |event| {
            let _ = tx.send(event);
        })?;
        Ok((monitor, rx))
    }

    /// How this monitor learns about changes.
    pub fn mode(&self) -> MonitorMode {
        self.mode
    }

    #[cfg(test)]
    fn spawn_with_source<E, F>(enumerate: E, interval: Duration, settle: Duration, handler: F) -> Result<Self>
    where
        E: FnMut() -> Result<Vec<DeviceInfo>> + Send + 'static,
        F: FnMut(DeviceEvent) + Send + 'static,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel::<Result<MonitorMode>>();
        let thread_stop = Arc::clone(&stop);
        let thread =
            std::thread::spawn(move || run_polling(enumerate, interval, settle, &thread_stop, handler, ready_tx));
        let mode = ready_rx.recv().expect("monitor thread").expect("monitor started");
        Ok(Self {
            stop,
            thread: Some(thread),
            mode,
        })
    }
}

impl Drop for DeviceMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

// ---- settling --------------------------------------------------------------------------------

/// Holds back new connections until they have settled and drops the ones that vanish first.
///
/// Time is passed in by the caller so that the logic can be tested without sleeping.
struct Settler<F> {
    handler: F,
    settle: Duration,
    /// Devices seen but not reported yet, with the time they appeared.
    pending: Vec<(DeviceInfo, Instant)>,
    /// Devices reported as connected and not yet reported as removed.
    announced: Vec<DeviceInfo>,
}

impl<F: FnMut(DeviceEvent)> Settler<F> {
    fn new(handler: F, settle: Duration) -> Self {
        Self {
            handler,
            settle,
            pending: Vec::new(),
            announced: Vec::new(),
        }
    }

    /// A device that has been there since before the monitor started: report it at once.
    fn present(&mut self, device: DeviceInfo) {
        self.announce(device);
    }

    /// A raw bus event at time `now`.
    fn event(&mut self, event: DeviceEvent, now: Instant) {
        match event {
            DeviceEvent::Connected(d) if self.settle.is_zero() => self.announce(d),
            DeviceEvent::Connected(d) => {
                if !self.pending.iter().any(|(p, _)| *p == d) && !self.announced.contains(&d) {
                    self.pending.push((d, now));
                }
            }
            DeviceEvent::Disconnected(d) => {
                if let Some(i) = self.pending.iter().position(|(p, _)| *p == d) {
                    // it vanished before it settled: the application never heard of it
                    self.pending.remove(i);
                } else if let Some(i) = self.announced.iter().position(|a| *a == d) {
                    self.announced.remove(i);
                    (self.handler)(DeviceEvent::Disconnected(d));
                }
            }
        }
    }

    /// Report the pending devices that have been connected for the settle time.
    fn tick(&mut self, now: Instant) {
        let (ready, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|(_, since)| now.duration_since(*since) >= self.settle);
        self.pending = waiting;
        for (device, _) in ready {
            self.announce(device);
        }
    }

    fn announce(&mut self, device: DeviceInfo) {
        if !self.announced.contains(&device) {
            self.announced.push(device);
            (self.handler)(DeviceEvent::Connected(device));
        }
    }
}

// ---- libusb hotplug --------------------------------------------------------------------------

/// Forwards libusb hotplug callbacks to the settler.
struct Forward<F> {
    settler: Arc<Mutex<Settler<F>>>,
    /// True while libusb replays the devices that were connected before the registration.
    existing: Arc<AtomicBool>,
}

impl<F: FnMut(DeviceEvent) + Send> Hotplug<Context> for Forward<F> {
    fn device_arrived(&mut self, device: Device<Context>) {
        if let Some(info) = DeviceInfo::of(&device) {
            let mut settler = self.settler.lock().unwrap();
            if self.existing.load(Ordering::SeqCst) {
                settler.present(info);
            } else {
                settler.event(DeviceEvent::Connected(info), Instant::now());
            }
        }
    }

    fn device_left(&mut self, device: Device<Context>) {
        if let Some(info) = DeviceInfo::of(&device) {
            self.settler
                .lock()
                .unwrap()
                .event(DeviceEvent::Disconnected(info), Instant::now());
        }
    }
}

fn run_hotplug<F>(settle: Duration, stop: &AtomicBool, handler: F, ready: mpsc::Sender<Result<MonitorMode>>)
where
    F: FnMut(DeviceEvent) + Send + 'static,
{
    let context = match Context::new() {
        Ok(c) => c,
        Err(e) => {
            let _ = ready.send(Err(e.into()));
            return;
        }
    };
    let settler = Arc::new(Mutex::new(Settler::new(handler, settle)));
    let existing = Arc::new(AtomicBool::new(true));
    // `enumerate(true)` replays the devices that are already connected before `register` returns
    let registration = HotplugBuilder::new()
        .vendor_id(VID)
        .product_id(PID)
        .enumerate(true)
        .register(
            &context,
            Box::new(Forward {
                settler: Arc::clone(&settler),
                existing: Arc::clone(&existing),
            }),
        );
    existing.store(false, Ordering::SeqCst);
    let registration = match registration {
        Ok(r) => r,
        Err(e) => {
            let _ = ready.send(Err(e.into()));
            return;
        }
    };
    let _ = ready.send(Ok(MonitorMode::Hotplug));
    while !stop.load(Ordering::SeqCst) {
        if let Err(e) = context.handle_events(Some(TICK)) {
            log::warn!("libusb event handling failed: {e}");
            std::thread::sleep(TICK);
        }
        settler.lock().unwrap().tick(Instant::now());
    }
    drop(registration);
}

// ---- polling fallback ------------------------------------------------------------------------

/// The events that turn the device list `before` into `after`: removals first, then arrivals.
fn diff(before: &[DeviceInfo], after: &[DeviceInfo]) -> Vec<DeviceEvent> {
    let gone = before
        .iter()
        .filter(|d| !after.contains(d))
        .map(|&d| DeviceEvent::Disconnected(d));
    let new = after
        .iter()
        .filter(|d| !before.contains(d))
        .map(|&d| DeviceEvent::Connected(d));
    gone.chain(new).collect()
}

fn run_polling<E, F>(
    mut enumerate: E,
    interval: Duration,
    settle: Duration,
    stop: &AtomicBool,
    handler: F,
    ready: mpsc::Sender<Result<MonitorMode>>,
) where
    E: FnMut() -> Result<Vec<DeviceInfo>>,
    F: FnMut(DeviceEvent),
{
    let mut known = match enumerate() {
        Ok(devices) => devices,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let _ = ready.send(Ok(MonitorMode::Polling));
    let mut settler = Settler::new(handler, settle);
    for &device in &known {
        settler.present(device);
    }
    let mut next_poll = Instant::now() + interval;
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(TICK.min(interval));
        let now = Instant::now();
        settler.tick(now);
        if now < next_poll {
            continue;
        }
        next_poll = now + interval;
        match enumerate() {
            Ok(current) => {
                for event in diff(&known, &current) {
                    settler.event(event, now);
                }
                known = current;
            }
            // a failed read says nothing about the devices; keep the previous list and try again
            Err(e) => log::warn!("could not list USB devices: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(bus: u8, address: u8) -> DeviceInfo {
        DeviceInfo {
            bus,
            address,
            vendor_id: VID,
            product_id: PID,
        }
    }

    #[test]
    fn diff_reports_removals_before_arrivals() {
        let a = dev(1, 10);
        let b = dev(1, 11);
        let c = dev(2, 4);
        assert!(diff(&[a], &[a]).is_empty());
        assert_eq!(diff(&[], &[a]), vec![DeviceEvent::Connected(a)]);
        assert_eq!(diff(&[a], &[]), vec![DeviceEvent::Disconnected(a)]);
        // the same scope plugged in again gets a new address: one removal, one arrival, in that order
        assert_eq!(
            diff(&[a, c], &[b, c]),
            vec![DeviceEvent::Disconnected(a), DeviceEvent::Connected(b)]
        );
    }

    #[test]
    fn display_and_accessor() {
        let e = DeviceEvent::Connected(dev(1, 33));
        assert_eq!(e.device().to_string(), "bus 1 address 33 (04b5:6cde)");
    }

    #[test]
    fn no_device_maps_to_disconnected() {
        let e: Error = rusb::Error::NoDevice.into();
        assert!(e.is_disconnected());
        let e: Error = rusb::Error::Pipe.into();
        assert!(!e.is_disconnected());
    }

    // ---- the settler, driven with explicit times ----

    type Recorded = Arc<Mutex<Vec<DeviceEvent>>>;

    fn settler(settle: Duration) -> (Settler<impl FnMut(DeviceEvent)>, Recorded) {
        let out = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&out);
        (Settler::new(move |e| sink.lock().unwrap().push(e), settle), out)
    }

    #[test]
    fn settler_reports_existing_devices_at_once() {
        let (mut s, out) = settler(Duration::from_secs(2));
        s.present(dev(1, 10));
        assert_eq!(*out.lock().unwrap(), vec![DeviceEvent::Connected(dev(1, 10))]);
    }

    #[test]
    fn settler_holds_back_a_new_connection_until_it_has_settled() {
        let (mut s, out) = settler(Duration::from_secs(2));
        let t0 = Instant::now();
        s.event(DeviceEvent::Connected(dev(1, 34)), t0);
        s.tick(t0 + Duration::from_millis(1900));
        assert!(out.lock().unwrap().is_empty());
        s.tick(t0 + Duration::from_secs(2));
        assert_eq!(*out.lock().unwrap(), vec![DeviceEvent::Connected(dev(1, 34))]);
        // reported once only
        s.tick(t0 + Duration::from_secs(5));
        assert_eq!(out.lock().unwrap().len(), 1);
    }

    #[test]
    fn settler_never_reports_a_scope_that_re_enumerates_during_power_up() {
        // the real sequence: appears, disappears 1.157 s later, appears again with a new address
        let (mut s, out) = settler(Duration::from_secs(2));
        let t0 = Instant::now();
        s.event(DeviceEvent::Connected(dev(1, 34)), t0);
        s.event(DeviceEvent::Disconnected(dev(1, 34)), t0 + Duration::from_millis(1157));
        s.event(DeviceEvent::Connected(dev(1, 35)), t0 + Duration::from_millis(1200));
        s.tick(t0 + Duration::from_millis(2500)); // the first address would have been due, the second is not
        assert!(out.lock().unwrap().is_empty(), "the first address must not be reported");
        s.tick(t0 + Duration::from_millis(3200));
        assert_eq!(*out.lock().unwrap(), vec![DeviceEvent::Connected(dev(1, 35))]);
    }

    #[test]
    fn settler_reports_removal_of_announced_devices_only() {
        let (mut s, out) = settler(Duration::from_secs(1));
        let t0 = Instant::now();
        s.present(dev(1, 10));
        s.event(DeviceEvent::Disconnected(dev(1, 10)), t0);
        s.event(DeviceEvent::Disconnected(dev(9, 9)), t0); // never seen: ignored
        assert_eq!(
            *out.lock().unwrap(),
            vec![
                DeviceEvent::Connected(dev(1, 10)),
                DeviceEvent::Disconnected(dev(1, 10))
            ]
        );
    }

    #[test]
    fn settler_with_zero_settle_reports_every_appearance() {
        let (mut s, out) = settler(Duration::ZERO);
        let t0 = Instant::now();
        s.event(DeviceEvent::Connected(dev(1, 34)), t0);
        s.event(DeviceEvent::Disconnected(dev(1, 34)), t0);
        assert_eq!(
            *out.lock().unwrap(),
            vec![
                DeviceEvent::Connected(dev(1, 34)),
                DeviceEvent::Disconnected(dev(1, 34))
            ]
        );
    }

    // ---- the polling loop, on a scripted bus ----

    /// A scripted bus: each `enumerate` call returns the next list, repeating the last one.
    fn scripted(script: Vec<Result<Vec<DeviceInfo>>>) -> impl FnMut() -> Result<Vec<DeviceInfo>> + Send + 'static {
        let mut script = script.into_iter().collect::<std::collections::VecDeque<_>>();
        let mut last: Vec<DeviceInfo> = Vec::new();
        move || match script.pop_front() {
            Some(Ok(list)) => {
                last = list.clone();
                Ok(list)
            }
            Some(Err(e)) => Err(e),
            None => Ok(last.clone()),
        }
    }

    fn wait_for(events: &Mutex<Vec<DeviceEvent>>, count: usize) {
        for _ in 0..300 {
            if events.lock().unwrap().len() >= count {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("expected {count} events, got {:?}", events.lock().unwrap());
    }

    #[test]
    fn polling_reports_existing_then_changes_and_survives_a_failed_read() {
        let (a, b) = (dev(1, 10), dev(1, 11));
        let source = scripted(vec![
            Ok(vec![a]),                      // at start: a is already connected
            Ok(vec![a]),                      // nothing changed
            Err(Error::Usb(rusb::Error::Io)), // a failed read must not look like an unplug
            Ok(vec![a]),                      // still connected
            Ok(vec![b]),                      // a removed and plugged in again as b
            Ok(vec![]),                       // b removed
        ]);
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        // zero settle: every change is reported as it is seen
        let monitor = DeviceMonitor::spawn_with_source(source, Duration::from_millis(5), Duration::ZERO, move |e| {
            sink.lock().unwrap().push(e)
        })
        .unwrap();
        assert_eq!(monitor.mode(), MonitorMode::Polling);
        wait_for(&events, 4);
        drop(monitor);
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                DeviceEvent::Connected(a),
                DeviceEvent::Disconnected(a),
                DeviceEvent::Connected(b),
                DeviceEvent::Disconnected(b),
            ]
        );
    }

    #[test]
    fn polling_with_settle_skips_the_power_up_glitch() {
        let (a, b) = (dev(1, 34), dev(1, 35));
        // nothing connected; then a appears, vanishes, and b appears and stays
        let source = scripted(vec![Ok(vec![]), Ok(vec![]), Ok(vec![a]), Ok(vec![]), Ok(vec![b])]);
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let monitor = DeviceMonitor::spawn_with_source(
            source,
            Duration::from_millis(40),
            Duration::from_millis(300),
            move |e| sink.lock().unwrap().push(e),
        )
        .unwrap();
        wait_for(&events, 1);
        drop(monitor);
        assert_eq!(*events.lock().unwrap(), vec![DeviceEvent::Connected(b)]);
    }

    #[test]
    fn dropping_the_monitor_stops_the_thread_promptly() {
        let source = scripted(vec![Ok(vec![])]);
        let monitor =
            DeviceMonitor::spawn_with_source(source, Duration::from_secs(60), Duration::from_secs(2), |_| {}).unwrap();
        let started = Instant::now();
        drop(monitor);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "stop must not wait for the poll interval"
        );
    }

    #[test]
    fn a_failing_first_read_is_reported_by_start() {
        let (tx, rx) = mpsc::channel();
        let stop = AtomicBool::new(false);
        run_polling(
            || Err(Error::Usb(rusb::Error::Access)),
            Duration::from_millis(1),
            Duration::ZERO,
            &stop,
            |_| {},
            tx,
        );
        assert!(matches!(rx.recv().unwrap(), Err(Error::Usb(rusb::Error::Access))));
    }
}

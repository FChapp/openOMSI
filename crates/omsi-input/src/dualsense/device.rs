//! The thread that keeps every DualSense open over HID: it finds them (and finds them again
//! after they were switched off), reads their reports as they come (some 250 a second over
//! USB, up to 1000 over Bluetooth) and sends them what the game wants of them. The game's
//! frame only takes the last state and the buttons that moved, and never waits for a pad.

use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use hidapi::{HidApi, HidDevice};

use super::report::{self, Buttons, Input, Output, Transport};

/// How often the pads are looked for while none is open, and while some are.
const SCAN: Duration = Duration::from_secs(2);
/// A pad that sent nothing for this long was switched off or went out of range (Windows
/// keeps a Bluetooth pad's handle open a while after).
const SILENT: Duration = Duration::from_secs(2);
/// How long the thread sleeps between two rounds of reading.
const TICK: Duration = Duration::from_millis(2);
/// The least time between two output reports to a pad: the game's frames change the rumble
/// up to some hundred times a second, and a write over Bluetooth can take milliseconds,
/// which the pad's reports (read on the same thread) would wait for.
const OUTPUT_EVERY: Duration = Duration::from_millis(12);

/// A pad as the game sees it: its name, ids, how it is connected and its last full report.
#[derive(Clone, Debug)]
pub struct Pad {
    pub name: String,
    pub id: (u16, u16),
    pub transport: Transport,
    pub input: Input,
    /// The motion sensors' calibration (feature report 0x05), when the pad gave it.
    pub calibration: Option<[u8; report::FEATURE_CALIBRATION_LEN]>,
}

#[derive(Default)]
struct Shared {
    pads: Vec<Pad>,
    /// (pad's name, button number, pressed) since the game last took them.
    events: Vec<(String, usize, bool)>,
    /// What the game wants every pad to be; None: it never asked, the pads are left as
    /// they are (their own blue light, no rumble).
    output: Option<Output>,
    quit: bool,
}

/// The DualSenses, read and driven by their thread; dropping it gives the pads back.
pub struct DualSenses {
    shared: Arc<Mutex<Shared>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl DualSenses {
    /// `write`: the pads may be sent rumble, trigger effects and lights (the game; the
    /// launcher only reads them).
    pub fn start(write: bool) -> Option<DualSenses> {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let worker = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("dualsense".into())
            .spawn(move || run(&worker, write))
            .map_err(|e| log::warn!("DualSense: no thread for it: {e}"))
            .ok()?;
        Some(DualSenses {
            shared,
            thread: Some(thread),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Shared> {
        // (a panic in the thread leaves its last state: still worth reading)
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The pads open now, with their last report.
    pub fn pads(&self) -> Vec<Pad> {
        self.lock().pads.clone()
    }

    /// The buttons pressed and let go since the last call, as DirectInput hands them on.
    pub fn take_events(&self) -> Vec<(String, usize, bool)> {
        std::mem::take(&mut self.lock().events)
    }

    /// Whether the device with these ids is a pad read here (and so not DirectInput's).
    pub fn owns(&self, id: Option<(u16, u16)>) -> bool {
        id.is_some_and(|id| self.lock().pads.iter().any(|p| p.id == id))
    }

    /// What every pad is to be from now on (sent as soon as it differs from what it is).
    pub fn set_output(&self, output: Output) {
        self.lock().output = Some(output);
    }
}

impl Drop for DualSenses {
    fn drop(&mut self) {
        // (waited for: a game that quits with the triggers stiff leaves them so until the pad
        // is switched off)
        self.lock().quit = true;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// One pad open.
struct Open {
    dev: HidDevice,
    path: std::ffi::CString,
    pad: Pad,
    buttons: Buttons,
    heard: Instant,
    /// The Bluetooth output report's sequence number.
    seq: u8,
    /// The rumble of firmware 2.24 and later (see `Output::vibration2`).
    vibration2: bool,
    /// What it was last sent, as the game asked it (None: nothing yet), and when.
    sent: Option<Output>,
    sent_at: Instant,
}

impl Open {
    fn new(api: &HidApi, info: &hidapi::DeviceInfo) -> Option<Open> {
        let dev = info
            .open_device(api)
            .map_err(|e| log::warn!("DualSense: could not open it: {e}"))
            .ok()?;
        // What it sends before anything is asked of it says how it is connected: 64 bytes
        // over USB, the short or the full Bluetooth report otherwise
        let mut buf = [0u8; 128];
        let t = Instant::now();
        let mut first = 0;
        while first == 0 && t.elapsed() < Duration::from_millis(500) {
            first = dev.read_timeout(&mut buf, 50).unwrap_or(0);
        }
        let transport = match (buf[0], first) {
            (report::INPUT_USB_ID, report::INPUT_USB_LEN) => Transport::Usb,
            (_, 0) if info.interface_number() >= 0 => Transport::Usb,
            _ => Transport::Bluetooth,
        };
        // Reading a feature report is also what makes a pad over Bluetooth send its full
        // report; the firmware's version says which rumble it plays
        let mut fw = [0u8; report::FEATURE_FIRMWARE_LEN];
        fw[0] = report::FEATURE_FIRMWARE;
        let firmware = match dev.get_feature_report(&mut fw) {
            Ok(n) if n >= 46 => u16::from_le_bytes([fw[44], fw[45]]),
            _ => 0,
        };
        let mut cal = [0u8; report::FEATURE_CALIBRATION_LEN];
        cal[0] = report::FEATURE_CALIBRATION;
        let calibration = dev
            .get_feature_report(&mut cal)
            .is_ok_and(|n| n == cal.len())
            .then_some(cal);
        let name = info
            .product_string()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(super::NAME)
            .to_string();
        log::info!("DualSense: {name} over {transport:?}, firmware {firmware:#06x}, read over HID");
        let input = idle_input();
        Some(Open {
            dev,
            path: info.path().to_owned(),
            pad: Pad {
                name,
                id: (info.vendor_id(), info.product_id()),
                transport,
                input,
                calibration,
            },
            buttons: Buttons::default(),
            heard: Instant::now(),
            seq: 0,
            vibration2: firmware == 0 || firmware >= report::FIRMWARE_VIBRATION2,
            sent: None,
            sent_at: Instant::now(),
        })
    }

    /// Every report waiting; the buttons that moved go to `events`. False: the pad is gone.
    fn read(&mut self, events: &mut Vec<(String, usize, bool)>) -> bool {
        let mut buf = [0u8; 128];
        loop {
            match self.dev.read_timeout(&mut buf, 0) {
                Ok(0) => return self.heard.elapsed() < SILENT,
                Ok(n) => {
                    self.heard = Instant::now();
                    let Some(input) = report::parse_input(&buf[..n]) else {
                        continue;
                    };
                    self.moved(input.buttons, events);
                    self.pad.input = input;
                }
                Err(e) => {
                    log::info!("DualSense: {} gone ({e})", self.pad.name);
                    return false;
                }
            }
        }
    }

    fn moved(&mut self, now: Buttons, events: &mut Vec<(String, usize, bool)>) {
        let changes = super::button_changes(&self.buttons, &now, crate::controllers::HAT_BUTTONS);
        events.extend(
            changes
                .into_iter()
                .map(|(n, down)| (self.pad.name.clone(), n, down)),
        );
        self.buttons = now;
    }

    fn send(&mut self, out: &Output) {
        // (the first report also fades out the blue light of the Bluetooth connection, which
        // would stay over the colour given)
        let full = Output {
            light_out: self.sent.is_none(),
            vibration2: self.vibration2,
            ..*out
        };
        let r = full.report(self.pad.transport, self.seq);
        self.seq = (self.seq + 1) & 0x0f;
        if let Err(e) = self.dev.write(&r) {
            log::warn!("DualSense: {} took no output report: {e}", self.pad.name);
        }
        self.sent = Some(*out);
        self.sent_at = Instant::now();
    }
}

/// A pad at rest, until its first full report.
fn idle_input() -> Input {
    Input {
        left_stick: [128; 2],
        right_stick: [128; 2],
        l2: 0,
        r2: 0,
        buttons: Buttons::default(),
        gyro: [0; 3],
        accel: [0; 3],
        sensor_time: 0,
        touches: [None, None],
        battery: report::Battery::Full,
    }
}

fn is_dualsense(info: &hidapi::DeviceInfo) -> bool {
    info.vendor_id() == report::VENDOR_SONY
        && matches!(info.product_id(), report::PRODUCT_DUALSENSE | report::PRODUCT_DUALSENSE_EDGE)
        // (the gamepad's collection, when the system lists the pad's others as well)
        && (info.usage_page(), info.usage()) == (1, 5)
}

fn run(shared: &Mutex<Shared>, write: bool) {
    let mut api = match HidApi::new() {
        Ok(api) => api,
        Err(e) => return log::warn!("DualSense: HID is not available: {e}"),
    };
    let lock = || shared.lock().unwrap_or_else(|e| e.into_inner());
    let mut open: Vec<Open> = Vec::new();
    let mut scan = Instant::now();
    loop {
        if scan <= Instant::now() {
            scan = Instant::now() + SCAN;
            if let Err(e) = api.refresh_devices() {
                log::warn!("DualSense: the HID devices could not be listed: {e}");
            }
            let found: Vec<_> = api
                .device_list()
                .filter(|d| is_dualsense(d) && !open.iter().any(|o| o.path.as_c_str() == d.path()))
                .cloned()
                .collect();
            open.extend(found.iter().filter_map(|d| Open::new(&api, d)));
        }
        let mut events = Vec::new();
        open.retain_mut(|o| {
            let alive = o.read(&mut events);
            if !alive {
                o.moved(Buttons::default(), &mut events);
            }
            alive
        });
        let (output, quit) = {
            let mut s = lock();
            s.pads = open.iter().map(|o| o.pad.clone()).collect();
            s.events.append(&mut events);
            (s.output, s.quit)
        };
        if quit {
            // (the pads' own lights again, and nothing pushing back)
            for o in open.iter_mut().filter(|o| o.sent.is_some()) {
                let release = Output {
                    release_leds: true,
                    ..Output::default()
                };
                let r = release.report(o.pad.transport, o.seq);
                let _ = o.dev.write(&r);
            }
            return;
        }
        if let Some(out) = output.filter(|_| write) {
            for o in open
                .iter_mut()
                .filter(|o| o.sent != Some(out) && o.sent_at.elapsed() >= OUTPUT_EVERY)
            {
                o.send(&out);
            }
        }
        thread::sleep(TICK);
    }
}

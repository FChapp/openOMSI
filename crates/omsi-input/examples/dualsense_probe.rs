//! Tries the DualSense over raw HID on a real pad, step by step, and writes what happened to
//! `dualsense_probe.log` next to where it is run. A tool for developing
//! `omsi_input::dualsense`, not part of the game.
//!
//! What it finds out:
//! - whether the game still reads the pad the way it does today (DirectInput on Windows,
//!   through `Devices`, as the launcher reads it) after each thing raw HID does to it: only
//!   reading its reports, reading a feature report (which switches a pad over Bluetooth to
//!   its full report), sending an output report;
//! - the buttons the full report adds (PS, touchpad click, microphone), the touchpad's
//!   fingers and the motion sensors.
//!
//! `cargo run -p omsi-input --example dualsense_probe`

use std::fs::File;
use std::io::{BufRead, Write};
use std::thread::sleep;
use std::time::{Duration, Instant};

use hidapi::{HidApi, HidDevice};
use omsi_input::controllers::Devices;
use omsi_input::dualsense::report::*;

struct Log(Option<File>);

impl Log {
    fn line(&mut self, s: &str) {
        println!("{s}");
        if let Some(f) = &mut self.0 {
            let _ = writeln!(f, "{s}");
        }
    }
}

struct Pad {
    dev: HidDevice,
    transport: Transport,
    seq: u8,
    out: Output,
}

impl Pad {
    fn send(&mut self, log: &mut Log) {
        let r = self.out.report(self.transport, self.seq);
        self.seq = self.seq.wrapping_add(1) & 0x0f;
        if let Err(e) = self.dev.write(&r) {
            log.line(&format!("  !! envoi refusé : {e}"));
        }
    }

    /// The reports waiting: the last full one parsed, and how many were short (Bluetooth's
    /// simple report) or full.
    fn drain(&self) -> (Option<Input>, usize, usize) {
        let mut buf = [0u8; 128];
        let (mut last, mut short, mut full) = (None, 0, 0);
        while let Ok(n) = self.dev.read_timeout(&mut buf, 0) {
            if n == 0 {
                break;
            }
            match parse_input(&buf[..n]) {
                Some(i) => {
                    last = Some(i);
                    full += 1;
                }
                None => short += 1,
            }
        }
        (last, short, full)
    }
}

/// The game's own reading of the pad (DirectInput on Windows, gilrs elsewhere).
struct Game {
    devices: Devices,
    /// The last axes seen, to count changes.
    last: Vec<(usize, f32)>,
}

impl Game {
    fn new(log: &mut Log) -> Game {
        let devices = Devices::new(omsi_input::background::window(), false);
        let mut g = Game {
            devices,
            last: Vec::new(),
        };
        g.tick();
        for c in g.devices.connected() {
            log.line(&format!(
                "le jeu voit : {:?} ids {:x?} manette {} axes {} boutons {}",
                c.name,
                c.hardware_id,
                c.gamepad,
                c.axes.len(),
                c.buttons
            ));
        }
        g
    }

    /// Reads the devices once; the DualSense's axes moved and the buttons it pressed.
    fn tick(&mut self) -> (bool, Vec<usize>) {
        omsi_input::background::pump();
        let buttons = self.devices.poll();
        let Some(pad) = self
            .devices
            .connected()
            .into_iter()
            .find(|c| is_dualsense(&c.name, c.hardware_id))
        else {
            return (false, Vec::new());
        };
        let moved = !self.last.is_empty()
            && pad
                .axes
                .iter()
                .zip(&self.last)
                .any(|(a, b)| (a.1 - b.1).abs() > 0.02);
        self.last = pad.axes;
        let pressed = buttons
            .into_iter()
            .filter(|(n, _, down)| *down && is_dualsense(n, None))
            .map(|(_, b, _)| b);
        (moved, pressed.collect())
    }

    /// `secs` of moving the sticks: whether the game and raw HID both saw it.
    fn check(&mut self, pad: &Pad, log: &mut Log, label: &str, secs: u64) {
        log.line(&format!(
            "\n>>> {label}\n    Bouge les DEUX sticks sans arrêt pendant {secs} s..."
        ));
        pad.drain();
        let t = Instant::now();
        let (mut game_moves, mut game_buttons, mut short, mut full) = (0, 0, 0, 0);
        while t.elapsed() < Duration::from_secs(secs) {
            sleep(Duration::from_millis(20));
            let (moved, pressed) = self.tick();
            game_moves += usize::from(moved);
            game_buttons += pressed.len();
            let (_, s, f) = pad.drain();
            short += s;
            full += f;
        }
        log.line(&format!(
            "    jeu : {game_moves} mouvements, {game_buttons} boutons | HID : {short} rapports simples, {full} complets -> {}",
            if game_moves > 5 { "LE JEU LIT LA MANETTE" } else { "LE JEU NE VOIT RIEN" }
        ));
    }
}

fn is_dualsense(name: &str, id: Option<(u16, u16)>) -> bool {
    if let Some((v, p)) = id {
        return v == VENDOR_SONY && matches!(p, PRODUCT_DUALSENSE | PRODUCT_DUALSENSE_EDGE);
    }
    let n = name.to_ascii_lowercase();
    n.contains("dualsense") || n.contains("wireless controller") || n.contains("ps5")
}

fn wait_enter(log: &mut Log, what: &str) {
    log.line(&format!("\n>>> {what}\n    (Entrée pour continuer)"));
    let mut s = String::new();
    let _ = std::io::stdin().lock().read_line(&mut s);
    if !s.trim().is_empty() {
        log.line(&format!("    réponse : {}", s.trim()));
    }
}

/// Waits up to `secs` for the full report to show `seen`, telling what it saw.
fn expect(pad: &Pad, log: &mut Log, what: &str, secs: u64, seen: impl Fn(&Input) -> bool) {
    log.line(&format!("- {what}"));
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(secs) {
        sleep(Duration::from_millis(10));
        if let (Some(i), _, _) = pad.drain() {
            if seen(&i) {
                return log.line("    OK");
            }
        }
    }
    log.line("    pas vu");
}

fn main() {
    let mut log = Log(File::create("dualsense_probe.log").ok());
    log.line("openOMSI - essai DualSense v2");
    wait_enter(
        &mut log,
        "Ferme Steam, DS4Windows/DSX et tout jeu ouvert.\n    \
         Éteins la manette (maintiens PS 10 s), rallume-la (PS), attends qu'elle soit connectée.",
    );

    let mut game = Game::new(&mut log);
    let api = match HidApi::new() {
        Ok(a) => a,
        Err(e) => return log.line(&format!("hidapi ne démarre pas : {e}")),
    };
    let pads: Vec<_> = api
        .device_list()
        .filter(|d| {
            d.vendor_id() == VENDOR_SONY
                && matches!(d.product_id(), PRODUCT_DUALSENSE | PRODUCT_DUALSENSE_EDGE)
        })
        .collect();
    for d in &pads {
        log.line(&format!(
            "HID : {:04x}:{:04x} {:?} interface {} usage {:x}/{:x} bus {:?}",
            d.vendor_id(),
            d.product_id(),
            d.product_string().unwrap_or("?"),
            d.interface_number(),
            d.usage_page(),
            d.usage(),
            d.bus_type(),
        ));
    }
    let Some(info) = pads
        .iter()
        .find(|d| (d.usage_page(), d.usage()) == (1, 5))
        .or(pads.first())
    else {
        log.line("Aucune DualSense trouvée. Est-elle connectée (et Steam fermé) ?");
        return wait_enter(&mut log, "Rien à faire");
    };
    let dev = match info.open_device(&api) {
        Ok(d) => d,
        Err(e) => return log.line(&format!("impossible d'ouvrir la manette : {e}")),
    };

    // What the pad sends before anything is asked of it says how it is connected
    let mut buf = [0u8; 128];
    let mut first = Vec::new();
    let t = Instant::now();
    while first.is_empty() && t.elapsed() < Duration::from_secs(2) {
        if let Ok(n) = dev.read_timeout(&mut buf, 100) {
            first = buf[..n].to_vec();
        }
    }
    let transport = match (first.first(), first.len()) {
        (Some(&INPUT_USB_ID), INPUT_USB_LEN) => Transport::Usb,
        (Some(_), _) => Transport::Bluetooth,
        _ if info.interface_number() == -1 => Transport::Bluetooth,
        _ => Transport::Usb,
    };
    log.line(&format!(
        "premier rapport : id {:#04x}, {} octets -> {:?}{}",
        first.first().copied().unwrap_or(0),
        first.len(),
        transport,
        if transport == Transport::Bluetooth && first.len() < 20 {
            " (mode simple)"
        } else {
            ""
        }
    ));
    let mut pad = Pad {
        dev,
        transport,
        seq: 0,
        out: Output::default(),
    };

    log.line("\nÉTAPE 1 : le jeu lit-il la manette ?");
    game.check(
        &pad,
        &mut log,
        "A. rien demandé à la manette, seulement lu",
        6,
    );

    let mut fw = [0u8; FEATURE_FIRMWARE_LEN];
    fw[0] = FEATURE_FIRMWARE;
    let firmware = match pad.dev.get_feature_report(&mut fw) {
        Ok(n) if n >= 46 => u16::from_le_bytes([fw[44], fw[45]]),
        _ => 0,
    };
    let mut cal = [0u8; FEATURE_CALIBRATION_LEN];
    cal[0] = FEATURE_CALIBRATION;
    let cal_read = pad.dev.get_feature_report(&mut cal);
    log.line(&format!(
        "firmware {firmware:#06x}, calibration {cal_read:?} : {:02x?}",
        &cal[..]
    ));
    pad.out.vibration2 = firmware == 0 || firmware >= FIRMWARE_VIBRATION2;
    game.check(
        &pad,
        &mut log,
        "B. après lecture du firmware et de la calibration",
        6,
    );

    pad.out.light_out = true;
    pad.out.lightbar = [0, 255, 0];
    pad.send(&mut log);
    pad.out.light_out = false;
    game.check(
        &pad,
        &mut log,
        "C. après un envoi (la barre passe au vert)",
        6,
    );

    pad.out.trigger_right = Trigger::resistance(1, 5);
    pad.out.rumble_left = 60;
    for _ in 0..5 {
        pad.send(&mut log);
        sleep(Duration::from_millis(100));
    }
    pad.out.rumble_left = 0;
    pad.send(&mut log);
    game.check(
        &pad,
        &mut log,
        "D. après vibration + R2 durcie (R2 doit rester dure)",
        6,
    );

    log.line("\nÉTAPE 2 : boutons et trackpad (10 s max chacun)");
    expect(&pad, &mut log, "appuie sur PS", 10, |i| i.buttons.ps);
    expect(&pad, &mut log, "appuie sur le bouton micro", 10, |i| {
        i.buttons.mic
    });
    expect(&pad, &mut log, "clique le trackpad", 10, |i| {
        i.buttons.touchpad
    });
    expect(&pad, &mut log, "pose UN doigt sur le trackpad", 10, |i| {
        i.touches[0].is_some()
    });
    expect(
        &pad,
        &mut log,
        "pose DEUX doigts sur le trackpad",
        10,
        |i| i.touches.iter().all(Option::is_some),
    );
    log.line("- glisse un doigt d'un coin à l'autre pendant 5 s");
    let (mut min, mut max) = ([u16::MAX; 2], [0u16; 2]);
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(5) {
        sleep(Duration::from_millis(10));
        if let (
            Some(Input {
                touches: [Some(f), _],
                ..
            }),
            _,
            _,
        ) = pad.drain()
        {
            min = [min[0].min(f.x), min[1].min(f.y)];
            max = [max[0].max(f.x), max[1].max(f.y)];
        }
    }
    log.line(&format!(
        "    x {}..{}, y {}..{} (sur {TOUCHPAD_WIDTH} x {TOUCHPAD_HEIGHT})",
        min[0], max[0], min[1], max[1]
    ));

    log.line("\nÉTAPE 3 : gyroscope");
    log.line("- pose la manette à plat, sans la toucher, 3 s");
    sleep(Duration::from_millis(500));
    pad.drain();
    let (mut sum, mut n) = ([0i64; 3], 0i64);
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(3) {
        sleep(Duration::from_millis(5));
        if let (Some(i), _, _) = pad.drain() {
            for k in 0..3 {
                sum[k] += i64::from(i.gyro[k]);
            }
            n += 1;
        }
    }
    let rest = sum.map(|s| s / n.max(1));
    log.line(&format!("    au repos : {rest:?}"));
    log.line("- prends-la et tourne-la vers la gauche puis la droite (comme un volant), 5 s");
    let mut peak = [0i32; 3];
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(5) {
        sleep(Duration::from_millis(5));
        if let (Some(i), _, _) = pad.drain() {
            for k in 0..3 {
                peak[k] = peak[k].max((i32::from(i.gyro[k]) - rest[k] as i32).abs());
            }
        }
    }
    // 1024 per degree a second before the calibration
    log.line(&format!(
        "    pic (°/s, tangage, lacet, roulis) : {:?}",
        peak.map(|p| p / 1024)
    ));
    if let (Some(i), _, _) = pad.drain() {
        log.line(&format!("batterie : {:?}", i.battery));
    }

    pad.out = Output {
        release_leds: true,
        ..Output::default()
    };
    pad.send(&mut log);
    game.check(&pad, &mut log, "E. manette rendue (fin)", 5);
    log.line("Envoie-moi le fichier dualsense_probe.log, merci !");
    wait_enter(&mut log, "Fini");
}

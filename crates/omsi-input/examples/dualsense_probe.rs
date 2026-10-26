//! Tries the DualSense's rumble, adaptive triggers, lights and full input report on a real
//! pad, step by step, and writes what happened to `dualsense_probe.log` next to where it is
//! run. A tool for developing `omsi_input::dualsense`, not part of the game.
//!
//! `cargo run -p omsi-input --example dualsense_probe`

use std::fs::File;
use std::io::{BufRead, Write};
use std::thread::sleep;
use std::time::{Duration, Instant};

use hidapi::{HidApi, HidDevice};
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

    /// The reports waiting, the last full one parsed; and how many were short (Bluetooth's
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

fn wait_enter(log: &mut Log, what: &str) {
    log.line(&format!("\n>>> {what}\n    (Entrée pour continuer)"));
    let mut s = String::new();
    let _ = std::io::stdin().lock().read_line(&mut s);
    if !s.trim().is_empty() {
        log.line(&format!("    réponse : {}", s.trim()));
    }
}

fn step(pad: &mut Pad, log: &mut Log, what: &str, secs: f32, set: impl FnOnce(&mut Output)) {
    log.line(&format!("- {what}"));
    set(&mut pad.out);
    pad.send(log);
    sleep(Duration::from_secs_f32(secs));
}

fn main() {
    let mut log = Log(File::create("dualsense_probe.log").ok());
    log.line("openOMSI - essai DualSense");

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
            "trouvée : {:04x}:{:04x} {:?} interface {} usage {:x}/{:x} bus {:?}",
            d.vendor_id(),
            d.product_id(),
            d.product_string().unwrap_or("?"),
            d.interface_number(),
            d.usage_page(),
            d.usage(),
            d.bus_type(),
        ));
    }
    // the gamepad collection if the pad shows several
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

    wait_enter(
        &mut log,
        "ÉTAPE 1 : Win+R, tape joy.cpl, choisis la manette, Propriétés.\n    \
         Les sticks et boutons bougent-ils ? Laisse la fenêtre ouverte et réponds (oui/non).",
    );

    // Reading a feature report switches a pad over Bluetooth to its full report
    let mut fw = [0u8; FEATURE_FIRMWARE_LEN];
    fw[0] = FEATURE_FIRMWARE;
    let firmware = match dev.get_feature_report(&mut fw) {
        Ok(n) if n >= 46 => u16::from_le_bytes([fw[44], fw[45]]),
        Ok(n) => {
            log.line(&format!("rapport firmware court ({n} octets)"));
            0
        }
        Err(e) => {
            log.line(&format!("rapport firmware refusé : {e}"));
            0
        }
    };
    let mut cal = [0u8; FEATURE_CALIBRATION_LEN];
    cal[0] = FEATURE_CALIBRATION;
    log.line(&format!(
        "firmware {:#06x}, calibration : {:?}",
        firmware,
        dev.get_feature_report(&mut cal)
    ));
    let mut pad = Pad {
        dev,
        transport,
        seq: 0,
        out: Output {
            vibration2: firmware == 0 || firmware >= FIRMWARE_VIBRATION2,
            ..Output::default()
        },
    };
    sleep(Duration::from_millis(300));
    let (_, short, full) = pad.drain();
    log.line(&format!(
        "après lecture : {short} rapports simples, {full} complets"
    ));

    log.line("\nÉTAPE 2 : lumières");
    step(
        &mut pad,
        &mut log,
        "on éteint le bleu de connexion",
        1.5,
        |o| o.light_out = true,
    );
    pad.out.light_out = false;
    for (name, rgb) in [
        ("rouge", [255, 0, 0]),
        ("vert", [0, 255, 0]),
        ("bleu", [0, 0, 255]),
        ("orange (clignotant)", [255, 100, 0]),
    ] {
        step(
            &mut pad,
            &mut log,
            &format!("barre lumineuse {name}"),
            1.2,
            |o| o.lightbar = rgb,
        );
    }
    for (i, leds) in [0b00100, 0b01010, 0b10101, 0b11011, 0b11111]
        .into_iter()
        .enumerate()
    {
        step(
            &mut pad,
            &mut log,
            &format!("LED joueur {}", i + 1),
            0.6,
            |o| o.player_leds = leds,
        );
    }
    step(&mut pad, &mut log, "LED micro allumée", 1.5, |o| {
        o.mic_led = MicLed::On
    });
    step(&mut pad, &mut log, "LED micro qui pulse", 2.5, |o| {
        o.mic_led = MicLed::Pulse
    });
    step(&mut pad, &mut log, "LED micro éteinte", 0.2, |o| {
        o.mic_led = MicLed::Off
    });
    wait_enter(
        &mut log,
        "As-tu vu rouge, vert, bleu, orange, les LED joueur et la LED micro ?",
    );

    log.line("\nÉTAPE 3 : vibrations");
    step(&mut pad, &mut log, "gauche (grave) fort", 1.5, |o| {
        o.rumble_left = 255
    });
    step(&mut pad, &mut log, "droite (aiguë) fort", 1.5, |o| {
        o.rumble_left = 0;
        o.rumble_right = 255;
    });
    log.line("- montée progressive des deux");
    for v in (0..=255u32).step_by(15) {
        pad.out.rumble_left = v as u8;
        pad.out.rumble_right = (v / 2) as u8;
        pad.send(&mut log);
        sleep(Duration::from_millis(120));
    }
    log.line("- petits coups (comme des pavés)");
    for _ in 0..12 {
        pad.out.rumble_left = 180;
        pad.send(&mut log);
        sleep(Duration::from_millis(40));
        pad.out.rumble_left = 0;
        pad.send(&mut log);
        sleep(Duration::from_millis(110));
    }
    step(&mut pad, &mut log, "arrêt", 0.2, |o| {
        o.rumble_left = 0;
        o.rumble_right = 0;
    });
    wait_enter(
        &mut log,
        "As-tu senti gauche, droite, la montée, les petits coups ?",
    );

    log.line("\nÉTAPE 4 : gâchettes adaptatives (appuie dessus pendant chaque essai)");
    step(
        &mut pad,
        &mut log,
        "R2 : résistance uniforme dès le début",
        0.0,
        |o| o.trigger_right = Trigger::resistance(1, 5),
    );
    wait_enter(&mut log, "R2 est-elle plus dure ?");
    step(
        &mut pad,
        &mut log,
        "L2 : frein progressif (de plus en plus dur)",
        0.0,
        |o| o.trigger_left = Trigger::Feedback([0, 1, 2, 3, 4, 5, 6, 7, 8, 8]),
    );
    wait_enter(&mut log, "L2 durcit-elle au fil de la course ?");
    step(
        &mut pad,
        &mut log,
        "L2 : point dur à mi-course (passage ralentisseur -> frein)",
        0.0,
        |o| {
            o.trigger_left = Trigger::Weapon {
                start: 4,
                end: 6,
                strength: 8,
            }
        },
    );
    wait_enter(&mut log, "Sens-tu un cran sur L2 ?");
    step(&mut pad, &mut log, "L2 : vibration (ABS)", 0.0, |o| {
        o.trigger_left = Trigger::vibration(2, 8, 25)
    });
    wait_enter(&mut log, "L2 vibre-t-elle quand tu appuies ?");
    step(
        &mut pad,
        &mut log,
        "R2 : bloquée (portes ouvertes)",
        0.0,
        |o| {
            o.trigger_left = Trigger::Off;
            o.trigger_right = Trigger::resistance(0, 8)
        },
    );
    wait_enter(&mut log, "R2 est-elle très dure ?");
    step(&mut pad, &mut log, "gâchettes libres", 0.2, |o| {
        o.trigger_left = Trigger::Off;
        o.trigger_right = Trigger::Off
    });

    log.line("\nÉTAPE 5 : entrées pendant 20 s (bouge tout : sticks, gâchettes, boutons,");
    log.line("          incline la manette, glisse un ou deux doigts sur le trackpad)");
    pad.drain();
    let t = Instant::now();
    let (mut seen, mut short_total) = (0usize, 0usize);
    let mut last = None;
    let mut touched = false;
    let mut moved = 0i32;
    while t.elapsed() < Duration::from_secs(20) {
        sleep(Duration::from_millis(100));
        let (i, short, full) = pad.drain();
        seen += full;
        short_total += short;
        let Some(i) = i else { continue };
        touched |= i.touches.iter().any(Option::is_some);
        moved = moved.max(
            i.gyro
                .iter()
                .map(|g| i32::from(*g).abs())
                .max()
                .unwrap_or(0),
        );
        print!(
            "\r G{:3},{:3} D{:3},{:3} L2 {:3} R2 {:3} gyro {:6},{:6},{:6} doigts {:?} {:?}   ",
            i.left_stick[0],
            i.left_stick[1],
            i.right_stick[0],
            i.right_stick[1],
            i.l2,
            i.r2,
            i.gyro[0],
            i.gyro[1],
            i.gyro[2],
            i.touches[0].map(|t| (t.x, t.y)),
            i.touches[1].map(|t| (t.x, t.y)),
        );
        let _ = std::io::stdout().flush();
        last = Some(i);
    }
    println!();
    log.line(&format!(
        "{seen} rapports complets ({:.0}/s), {short_total} simples, trackpad touché : {touched}, \
         gyro max {moved}",
        seen as f32 / 20.0
    ));
    if let Some(i) = last {
        log.line(&format!("dernier état : {:?}", i.buttons));
        log.line(&format!("batterie : {:?}", i.battery));
    }

    wait_enter(
        &mut log,
        "ÉTAPE 6 : retourne dans joy.cpl. Les sticks et boutons bougent-ils ENCORE ? (oui/non)",
    );

    pad.out = Output {
        release_leds: true,
        ..Output::default()
    };
    pad.send(&mut log);
    log.line("Manette rendue. Envoie-moi le fichier dualsense_probe.log, merci !");
    wait_enter(&mut log, "Fini");
}

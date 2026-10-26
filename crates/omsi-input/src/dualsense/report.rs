//! The DualSense's reports, as bytes: the output report that drives its motors, triggers and
//! lights, and the input report it sends back (sticks, buttons, motion, touchpad, battery).
//! Nothing here opens a device, so all of it is tested on any machine.
//!
//! Layouts as the Linux driver (`drivers/hid/hid-playstation.c`) and SDL
//! (`SDL_hidapi_ps5.c`) read and write them:
//!
//! - over USB the output report is 0x02 and 63 bytes, the 47 common bytes right after the id;
//! - over Bluetooth it is 0x31 and 78 bytes: a sequence number (high nibble, one more with
//!   every report), the tag 0x10, the common bytes, and a CRC-32 of the seed byte 0xA2 and
//!   everything before the CRC, little endian, in the last four bytes. A report with a wrong
//!   CRC is dropped by the pad without a word.
//! - the input report is 0x01 over USB (64 bytes, data from byte 1) and 0x31 over Bluetooth
//!   (78 bytes, data from byte 2). A pad over Bluetooth starts with a short 0x01 report of
//!   sticks and buttons only and sends 0x31 once a feature report has been read from it.

/// How the pad is connected; the reports are framed differently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Usb,
    Bluetooth,
}

pub const VENDOR_SONY: u16 = 0x054c;
pub const PRODUCT_DUALSENSE: u16 = 0x0ce6;
pub const PRODUCT_DUALSENSE_EDGE: u16 = 0x0df2;

pub const OUTPUT_USB_ID: u8 = 0x02;
pub const OUTPUT_USB_LEN: usize = 63;
pub const OUTPUT_BT_ID: u8 = 0x31;
pub const OUTPUT_BT_LEN: usize = 78;
pub const INPUT_USB_ID: u8 = 0x01;
pub const INPUT_USB_LEN: usize = 64;
pub const INPUT_BT_ID: u8 = 0x31;
pub const INPUT_BT_LEN: usize = 78;
/// The feature report with the motion sensors' calibration; reading it is also what switches
/// a pad over Bluetooth to its full input report.
pub const FEATURE_CALIBRATION: u8 = 0x05;
pub const FEATURE_CALIBRATION_LEN: usize = 41;
/// The feature report with the firmware's version (bytes 44-45).
pub const FEATURE_FIRMWARE: u8 = 0x20;
pub const FEATURE_FIRMWARE_LEN: usize = 64;

const BT_TAG: u8 = 0x10;
const CRC_SEED_OUTPUT: u8 = 0xa2;
const CRC_SEED_INPUT: u8 = 0xa1;

// The common block's bytes and flags
const FLAG0: usize = 0;
const FLAG1: usize = 1;
const MOTOR_RIGHT: usize = 2;
const MOTOR_LEFT: usize = 3;
const MIC_LED: usize = 8;
const TRIGGER_RIGHT: usize = 10;
const TRIGGER_LEFT: usize = 21;
const MOTOR_POWER: usize = 36;
const FLAG2: usize = 38;
const LIGHTBAR_SETUP: usize = 41;
const LED_BRIGHTNESS: usize = 42;
const PLAYER_LEDS: usize = 43;
const LIGHTBAR: usize = 44;
const COMMON_LEN: usize = 47;

const F0_COMPATIBLE_VIBRATION: u8 = 1 << 0;
/// The motors play the rumble given here instead of the audio haptics.
const F0_HAPTICS_SELECT: u8 = 1 << 1;
const F0_RIGHT_TRIGGER: u8 = 1 << 2;
const F0_LEFT_TRIGGER: u8 = 1 << 3;
const F1_MIC_LED: u8 = 1 << 0;
const F1_LIGHTBAR: u8 = 1 << 2;
const F1_RELEASE_LEDS: u8 = 1 << 3;
const F1_PLAYER_LEDS: u8 = 1 << 4;
const F1_MOTOR_POWER: u8 = 1 << 6;
const F2_LED_BRIGHTNESS: u8 = 1 << 0;
const F2_LIGHTBAR_SETUP: u8 = 1 << 1;
/// The rumble of firmware 2.24 and later, which feels like a DualShock 4's; older firmware
/// takes `F0_COMPATIBLE_VIBRATION`.
const F2_COMPATIBLE_VIBRATION2: u8 = 1 << 2;
/// `LIGHTBAR_SETUP`: fade the blue light of the Bluetooth connection out, so the colour given
/// shows (it would stay over it otherwise).
const LIGHTBAR_LIGHT_OUT: u8 = 1 << 1;

/// The firmware from which `F2_COMPATIBLE_VIBRATION2` rumbles.
pub const FIRMWARE_VIBRATION2: u16 = 0x0224;

/// An adaptive trigger's effect. The trigger's travel is ten zones (0 at rest, 9 pressed
/// fully); strengths go from 1 (light) to 8 (as hard as it goes), 0 being none in that zone.
/// The byte layouts are the ones `dualsensectl` and Sony's own games use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Trigger {
    /// No resistance.
    #[default]
    Off,
    /// A resistance per zone: a spring that gets stiffer when the strengths rise.
    Feedback([u8; 10]),
    /// Free until `start`, then a wall that gives way at `end` with a click (a gun's trigger,
    /// a pedal's detent). `start` 2..=7, `end` start+1..=8.
    Weapon { start: u8, end: u8, strength: u8 },
    /// The trigger shakes from each zone with its amplitude, at `frequency` Hz.
    Vibration { amplitudes: [u8; 10], frequency: u8 },
}

const TRIGGER_OFF: u8 = 0x05;
const TRIGGER_FEEDBACK: u8 = 0x21;
const TRIGGER_WEAPON: u8 = 0x25;
const TRIGGER_VIBRATION: u8 = 0x26;

impl Trigger {
    /// The same strength from `start` to the end of the travel.
    pub fn resistance(start: u8, strength: u8) -> Trigger {
        Trigger::Feedback(from_zone(start, strength))
    }

    /// The same shaking from `start` to the end of the travel.
    pub fn vibration(start: u8, amplitude: u8, frequency: u8) -> Trigger {
        Trigger::Vibration {
            amplitudes: from_zone(start, amplitude),
            frequency,
        }
    }

    /// The mode byte and its ten parameters, as the report takes them.
    pub fn bytes(&self) -> [u8; 11] {
        let mut b = [0u8; 11];
        match *self {
            Trigger::Off => b[0] = TRIGGER_OFF,
            Trigger::Feedback(strengths) => {
                b[0] = TRIGGER_FEEDBACK;
                pack_zones(&strengths, &mut b[1..7]);
            }
            Trigger::Weapon {
                start,
                end,
                strength,
            } => {
                let start = start.clamp(2, 7);
                let end = end.clamp(start + 1, 8);
                let zones = (1u16 << start) | (1u16 << end);
                b[0] = TRIGGER_WEAPON;
                b[1..3].copy_from_slice(&zones.to_le_bytes());
                b[3] = strength.clamp(1, 8) - 1;
            }
            Trigger::Vibration {
                amplitudes,
                frequency,
            } => {
                b[0] = TRIGGER_VIBRATION;
                pack_zones(&amplitudes, &mut b[1..7]);
                b[9] = frequency;
            }
        }
        b
    }
}

fn from_zone(start: u8, value: u8) -> [u8; 10] {
    let mut zones = [0u8; 10];
    for z in zones.iter_mut().skip(start.min(9) as usize) {
        *z = value.min(8);
    }
    zones
}

/// Which zones are active (one bit each, two bytes) and their values less one (three bits
/// each, four bytes), both little endian.
fn pack_zones(values: &[u8; 10], out: &mut [u8]) {
    let mut active = 0u16;
    let mut packed = 0u32;
    for (i, &v) in values.iter().enumerate() {
        if v > 0 {
            active |= 1 << i;
            packed |= u32::from(v.min(8) - 1) << (3 * i);
        }
    }
    out[0..2].copy_from_slice(&active.to_le_bytes());
    out[2..6].copy_from_slice(&packed.to_le_bytes());
}

/// The microphone button's light.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MicLed {
    #[default]
    Off,
    On,
    Pulse,
}

/// Everything the game sets on the pad. Built whole every time: the report carries every
/// field with a flag saying it counts, and this sets all the flags, so the pad is always
/// exactly as described here.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Output {
    /// The low (left, heavy) and high (right, light) rumble, 0..=255.
    pub rumble_left: u8,
    pub rumble_right: u8,
    /// The rumble as firmware 2.24 and later plays it (see `FIRMWARE_VIBRATION2`).
    pub vibration2: bool,
    /// L2 and R2.
    pub trigger_left: Trigger,
    pub trigger_right: Trigger,
    /// The light bar's colour.
    pub lightbar: [u8; 3],
    /// The five player lights under the touchpad, one bit each from the left.
    pub player_leds: u8,
    /// 0 brightest .. 2 dimmest, for the player lights.
    pub led_brightness: u8,
    pub mic_led: MicLed,
    /// 0 full strength .. 7 weakest, for the rumble and for the triggers' vibration.
    pub rumble_attenuation: u8,
    pub trigger_attenuation: u8,
    /// Fade the Bluetooth connection's blue light out (once, before the first colour).
    pub light_out: bool,
    /// Give the lights back to the pad (when the game lets go of it).
    pub release_leds: bool,
}

impl Output {
    /// The 47 common bytes.
    fn common(&self) -> [u8; COMMON_LEN] {
        let mut c = [0u8; COMMON_LEN];
        c[FLAG0] = F0_HAPTICS_SELECT | F0_RIGHT_TRIGGER | F0_LEFT_TRIGGER;
        if self.vibration2 {
            c[FLAG2] |= F2_COMPATIBLE_VIBRATION2;
        } else {
            c[FLAG0] |= F0_COMPATIBLE_VIBRATION;
        }
        c[MOTOR_RIGHT] = self.rumble_right;
        c[MOTOR_LEFT] = self.rumble_left;
        c[TRIGGER_RIGHT..TRIGGER_RIGHT + 11].copy_from_slice(&self.trigger_right.bytes());
        c[TRIGGER_LEFT..TRIGGER_LEFT + 11].copy_from_slice(&self.trigger_left.bytes());

        c[FLAG1] = F1_MIC_LED | F1_LIGHTBAR | F1_PLAYER_LEDS | F1_MOTOR_POWER;
        c[MIC_LED] = match self.mic_led {
            MicLed::Off => 0,
            MicLed::On => 1,
            MicLed::Pulse => 2,
        };
        c[MOTOR_POWER] = (self.rumble_attenuation & 7) | ((self.trigger_attenuation & 7) << 4);
        c[FLAG2] |= F2_LED_BRIGHTNESS;
        c[LED_BRIGHTNESS] = self.led_brightness.min(2);
        // 0x20: the player lights change at once instead of fading in
        c[PLAYER_LEDS] = (self.player_leds & 0x1f) | 0x20;
        c[LIGHTBAR..LIGHTBAR + 3].copy_from_slice(&self.lightbar);
        if self.light_out {
            c[FLAG2] |= F2_LIGHTBAR_SETUP;
            c[LIGHTBAR_SETUP] = LIGHTBAR_LIGHT_OUT;
        }
        if self.release_leds {
            c[FLAG1] = (c[FLAG1] & !(F1_LIGHTBAR | F1_PLAYER_LEDS)) | F1_RELEASE_LEDS;
        }
        c
    }

    /// The whole output report for `transport`; `seq` is counted by the caller, one more with
    /// every report sent (only Bluetooth uses it).
    pub fn report(&self, transport: Transport, seq: u8) -> Vec<u8> {
        let common = self.common();
        match transport {
            Transport::Usb => {
                let mut r = vec![0u8; OUTPUT_USB_LEN];
                r[0] = OUTPUT_USB_ID;
                r[1..1 + COMMON_LEN].copy_from_slice(&common);
                r
            }
            Transport::Bluetooth => {
                let mut r = vec![0u8; OUTPUT_BT_LEN];
                r[0] = OUTPUT_BT_ID;
                r[1] = (seq & 0x0f) << 4;
                r[2] = BT_TAG;
                r[3..3 + COMMON_LEN].copy_from_slice(&common);
                let crc = crc32(CRC_SEED_OUTPUT, &r[..OUTPUT_BT_LEN - 4]);
                r[OUTPUT_BT_LEN - 4..].copy_from_slice(&crc.to_le_bytes());
                r
            }
        }
    }
}

/// CRC-32 (IEEE, as zlib's) of the seed byte followed by `data`.
pub fn crc32(seed: u8, data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in std::iter::once(&seed).chain(data) {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// One finger on the touchpad, 0..1920 across and 0..1080 down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Touch {
    /// Counts up with every new touch, so a finger lifted and put down again is told apart.
    pub id: u8,
    pub x: u16,
    pub y: u16,
}

pub const TOUCHPAD_WIDTH: u16 = 1920;
pub const TOUCHPAD_HEIGHT: u16 = 1080;

/// The battery as the pad tells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Battery {
    /// Running on the battery, or charging, with roughly this many percent left.
    Discharging(u8),
    Charging(u8),
    Full,
    /// Too hot, too cold or a fault: not charging.
    Error,
}

/// The buttons, by name; a hat direction counts as its two buttons on a diagonal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Buttons {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub square: bool,
    pub cross: bool,
    pub circle: bool,
    pub triangle: bool,
    pub l1: bool,
    pub r1: bool,
    pub l2: bool,
    pub r2: bool,
    pub create: bool,
    pub options: bool,
    pub l3: bool,
    pub r3: bool,
    pub ps: bool,
    pub touchpad: bool,
    pub mic: bool,
}

/// One full input report.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Input {
    /// Left stick, right stick, each 0..=255 with 128 in the middle (y grows downwards).
    pub left_stick: [u8; 2],
    pub right_stick: [u8; 2],
    /// L2 and R2, 0 released .. 255 pressed fully.
    pub l2: u8,
    pub r2: u8,
    pub buttons: Buttons,
    /// The gyroscope (pitch, yaw, roll) and accelerometer (x, y, z), raw: 1024 per degree a
    /// second and 8192 per g before the calibration (`FEATURE_CALIBRATION`).
    pub gyro: [i16; 3],
    pub accel: [i16; 3],
    /// The sensors' clock, in units of 0.33 µs; it starts when the pad connects.
    pub sensor_time: u32,
    pub touches: [Option<Touch>; 2],
    pub battery: Battery,
}

/// The full input report in `data` (as read, with its id), or None for any other report: the
/// short Bluetooth one, a report of another length, or a Bluetooth report whose CRC is wrong.
pub fn parse_input(data: &[u8]) -> Option<Input> {
    let d = match (data.first()?, data.len()) {
        (&INPUT_USB_ID, INPUT_USB_LEN) => &data[1..],
        (&INPUT_BT_ID, INPUT_BT_LEN) => {
            let crc = u32::from_le_bytes(data[INPUT_BT_LEN - 4..].try_into().ok()?);
            if crc32(CRC_SEED_INPUT, &data[..INPUT_BT_LEN - 4]) != crc {
                return None;
            }
            &data[2..]
        }
        _ => return None,
    };
    let le16 = |i: usize| i16::from_le_bytes([d[i], d[i + 1]]);
    let hat = d[7] & 0x0f;
    let buttons = Buttons {
        up: matches!(hat, 7 | 0 | 1),
        right: matches!(hat, 1..=3),
        down: matches!(hat, 3..=5),
        left: matches!(hat, 5..=7),
        square: d[7] & 0x10 != 0,
        cross: d[7] & 0x20 != 0,
        circle: d[7] & 0x40 != 0,
        triangle: d[7] & 0x80 != 0,
        l1: d[8] & 0x01 != 0,
        r1: d[8] & 0x02 != 0,
        l2: d[8] & 0x04 != 0,
        r2: d[8] & 0x08 != 0,
        create: d[8] & 0x10 != 0,
        options: d[8] & 0x20 != 0,
        l3: d[8] & 0x40 != 0,
        r3: d[8] & 0x80 != 0,
        ps: d[9] & 0x01 != 0,
        touchpad: d[9] & 0x02 != 0,
        mic: d[9] & 0x04 != 0,
    };
    let touch = |i: usize| {
        let p = &d[i..i + 4];
        // bit 7 of the first byte set: no finger
        (p[0] & 0x80 == 0).then(|| Touch {
            id: p[0] & 0x7f,
            x: u16::from(p[1]) | (u16::from(p[2] & 0x0f) << 8),
            y: u16::from(p[2] >> 4) | (u16::from(p[3]) << 4),
        })
    };
    let level = d[52] & 0x0f;
    let percent = (level * 10 + 5).min(100);
    let battery = match d[52] >> 4 {
        0 => Battery::Discharging(percent),
        1 => Battery::Charging(percent),
        2 => Battery::Full,
        _ => Battery::Error,
    };
    Some(Input {
        left_stick: [d[0], d[1]],
        right_stick: [d[2], d[3]],
        l2: d[4],
        r2: d[5],
        buttons,
        gyro: [le16(15), le16(17), le16(19)],
        accel: [le16(21), le16(23), le16(25)],
        sensor_time: u32::from_le_bytes([d[27], d[28], d[29], d[30]]),
        touches: [touch(32), touch(36)],
        battery,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_is_zlibs() {
        // the seed byte counts as the first byte of the data
        assert_eq!(crc32(0xa2, &[]), crc32_ref(&[0xa2]));
        let mut s = vec![0xa2];
        s.extend_from_slice(b"123456789");
        assert_eq!(crc32(0xa2, b"123456789"), crc32_ref(&s));
        // the check value of CRC-32/ISO-HDLC
        assert_eq!(crc32_ref(b"123456789"), 0xcbf4_3926);
    }

    /// A plain CRC-32 without the seed byte, to check the seeded one against.
    fn crc32_ref(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &b in data {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    #[test]
    fn usb_report_puts_the_common_bytes_after_the_id() {
        let out = Output {
            rumble_left: 200,
            rumble_right: 50,
            lightbar: [1, 2, 3],
            ..Output::default()
        };
        let r = out.report(Transport::Usb, 0);
        assert_eq!(r.len(), OUTPUT_USB_LEN);
        assert_eq!(r[0], OUTPUT_USB_ID);
        assert_eq!((r[1 + MOTOR_RIGHT], r[1 + MOTOR_LEFT]), (50, 200));
        assert_eq!(&r[1 + LIGHTBAR..1 + LIGHTBAR + 3], &[1, 2, 3]);
        assert_eq!(
            r[1 + FLAG0] & F0_COMPATIBLE_VIBRATION,
            F0_COMPATIBLE_VIBRATION
        );
        assert_eq!(r[1 + FLAG1] & F1_LIGHTBAR, F1_LIGHTBAR);
    }

    #[test]
    fn bluetooth_report_is_framed_and_signed() {
        let out = Output {
            lightbar: [255, 0, 0],
            vibration2: true,
            ..Output::default()
        };
        let r = out.report(Transport::Bluetooth, 0x13);
        assert_eq!(r.len(), OUTPUT_BT_LEN);
        assert_eq!((r[0], r[1], r[2]), (OUTPUT_BT_ID, 0x30, BT_TAG));
        assert_eq!(&r[3 + LIGHTBAR..3 + LIGHTBAR + 3], &[255, 0, 0]);
        assert_eq!(
            r[3 + FLAG2] & F2_COMPATIBLE_VIBRATION2,
            F2_COMPATIBLE_VIBRATION2
        );
        assert_eq!(r[3 + FLAG0] & F0_COMPATIBLE_VIBRATION, 0);
        let crc = u32::from_le_bytes(r[74..78].try_into().unwrap());
        assert_eq!(crc, crc32(0xa2, &r[..74]));
    }

    #[test]
    fn released_leds_are_not_set_as_well() {
        let c = Output {
            release_leds: true,
            ..Output::default()
        }
        .common();
        assert_eq!(
            c[FLAG1] & (F1_LIGHTBAR | F1_PLAYER_LEDS | F1_RELEASE_LEDS),
            F1_RELEASE_LEDS
        );
    }

    #[test]
    fn triggers_pack_their_zones() {
        assert_eq!(Trigger::Off.bytes(), [0x05, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        // dualsensectl's `trigger right feedback 3 6`: zones 3-9 at 6 (5 in three bits each)
        let b = Trigger::resistance(3, 6).bytes();
        assert_eq!(b[0], 0x21);
        assert_eq!(u16::from_le_bytes([b[1], b[2]]), 0b11_1111_1000);
        let packed = u32::from_le_bytes([b[3], b[4], b[5], b[6]]);
        for zone in 0..10 {
            let want = if zone >= 3 { 5 } else { 0 };
            assert_eq!((packed >> (3 * zone)) & 7, want, "zone {zone}");
        }
        // `trigger right weapon 4 7 8`
        assert_eq!(
            Trigger::Weapon {
                start: 4,
                end: 7,
                strength: 8
            }
            .bytes(),
            [0x25, 0x90, 0x00, 7, 0, 0, 0, 0, 0, 0, 0]
        );
        let v = Trigger::vibration(0, 8, 40).bytes();
        assert_eq!((v[0], v[1], v[2], v[9]), (0x26, 0xff, 0x03, 40));
    }

    fn usb_input() -> Vec<u8> {
        let mut r = vec![0u8; INPUT_USB_LEN];
        r[0] = INPUT_USB_ID;
        let d = &mut r[1..];
        d[0..6].copy_from_slice(&[10, 20, 128, 255, 0, 200]);
        d[7] = 0x08 | 0x20; // hat released, cross
        d[8] = 0x02 | 0x20; // R1, Options
        d[9] = 0x02; // touchpad click
        d[15..17].copy_from_slice(&(-1024i16).to_le_bytes());
        d[25..27].copy_from_slice(&8192i16.to_le_bytes());
        // a finger (id 3) at 1000, 500; the second slot empty
        d[32] = 3;
        d[33] = (1000 & 0xff) as u8;
        d[34] = ((1000 >> 8) as u8) | (((500 & 0x0f) as u8) << 4);
        d[35] = (500 >> 4) as u8;
        d[36] = 0x80;
        d[52] = 0x16; // charging, 60%
        r
    }

    #[test]
    fn usb_input_is_read() {
        let i = parse_input(&usb_input()).unwrap();
        assert_eq!(
            (i.left_stick, i.right_stick, i.l2, i.r2),
            ([10, 20], [128, 255], 0, 200)
        );
        let b = i.buttons;
        assert!(b.cross && b.r1 && b.options && b.touchpad);
        assert!(!(b.up || b.down || b.left || b.right || b.square || b.l1 || b.ps));
        assert_eq!(i.gyro[0], -1024);
        assert_eq!(i.accel[2], 8192);
        assert_eq!(
            i.touches,
            [
                Some(Touch {
                    id: 3,
                    x: 1000,
                    y: 500
                }),
                None
            ]
        );
        assert_eq!(i.battery, Battery::Charging(65));
    }

    #[test]
    fn bluetooth_input_needs_its_crc() {
        let usb = usb_input();
        let mut bt = vec![0u8; INPUT_BT_LEN];
        bt[0] = INPUT_BT_ID;
        bt[2..2 + 63].copy_from_slice(&usb[1..]);
        let crc = crc32(0xa1, &bt[..74]);
        bt[74..].copy_from_slice(&crc.to_le_bytes());
        assert_eq!(parse_input(&bt), parse_input(&usb));
        bt[10] ^= 1;
        assert_eq!(parse_input(&bt), None);
        // the short report a pad over Bluetooth starts with
        assert_eq!(
            parse_input(&[0x01, 128, 128, 128, 128, 8, 0, 0, 0, 0]),
            None
        );
    }

    #[test]
    fn hat_diagonals_press_two_buttons() {
        let mut r = usb_input();
        r[8] = 0x01; // up-right
        let b = parse_input(&r).unwrap().buttons;
        assert!(b.up && b.right && !b.down && !b.left);
        r[8] = 0x06; // left
        let b = parse_input(&r).unwrap().buttons;
        assert!(b.left && !b.up && !b.down && !b.right);
    }
}

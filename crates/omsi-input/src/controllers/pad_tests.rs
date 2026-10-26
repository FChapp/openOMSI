//! The tests of what kind a device is and of a gamepad's sticks.

use super::stick_deadzone_with;

#[test]
fn a_mapped_constant_force_wheel_is_not_a_gamepad() {
    assert!(super::mapped_device_is_gamepad(true, false));
    assert!(!super::mapped_device_is_gamepad(true, true));
    assert!(!super::mapped_device_is_gamepad(false, true));
}

#[test]
fn the_dead_zone_is_the_settings_and_the_rest_of_the_stick_still_reaches_one() {
    assert_eq!(stick_deadzone_with(0.14, 0.15), 0.0);
    assert!(stick_deadzone_with(0.3, 0.15) > 0.0);
    assert!((stick_deadzone_with(-1.0, 0.15) + 1.0).abs() < 1e-6);
    assert_eq!(stick_deadzone_with(0.05, 0.0), 0.05);
}

#[test]
fn an_idle_device_nobody_set_up_does_not_hold_the_sticks_steering() {
    assert!(super::stick_steers(None, false, 0.0));
    assert!(super::stick_steers(Some(0.0), false, -0.6));
    assert!(super::stick_steers(Some(0.0), false, 0.0));
    assert!(super::stick_steers(Some(0.02), false, 0.0));
    assert!(!super::stick_steers(Some(0.8), false, 0.3));
    assert!(!super::stick_steers(Some(0.0), true, 1.0));
}

#[test]
fn disabling_automatic_look_keeps_driving_controls_and_assigned_look() {
    for assigned in [[0.0, 0.0], [0.5, -0.25]] {
        let mut analog = super::Analog {
            steering: Some(0.3),
            stick: true,
            throttle: Some(0.8),
            brake: Some(0.2),
            clutch: Some(0.4),
            look: assigned,
            gyro: [0.0; 2],
        };
        analog.apply_default_gamepad_look(false, 1.0, -1.0);
        assert_eq!(analog.look, assigned);
        assert_eq!(analog.steering, Some(0.3));
        assert!(analog.stick);
        assert_eq!(analog.throttle, Some(0.8));
        assert_eq!(analog.brake, Some(0.2));
        assert_eq!(analog.clutch, Some(0.4));
    }
}

#[test]
fn enabled_automatic_look_keeps_dead_zone_direction_and_assigned_axes() {
    let mut analog = super::Analog::default();
    analog.apply_default_gamepad_look(true, 0.1, -0.1);
    assert_eq!(analog.look, [0.0, 0.0]);
    analog.apply_default_gamepad_look(true, 1.0, -1.0);
    assert_eq!(analog.look, [1.0, 1.0]);
    analog.look = [0.25, -0.5];
    analog.apply_default_gamepad_look(true, -1.0, 1.0);
    assert_eq!(analog.look, [0.25, -0.5]);
}

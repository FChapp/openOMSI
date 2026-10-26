//! Windows: the touchpad as the mouse - the cursor moved and the left button pressed as the
//! system's own mouse would, so the cockpit's switches and the menus take it as they take a
//! mouse.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_MOUSE, MOUSE_EVENT_FLAGS, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MOVE, MOUSEINPUT, SendInput,
};

fn send(flags: MOUSE_EVENT_FLAGS, dx: i32, dy: i32) {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    // (refused only when another program holds the input, as a UAC prompt does: nothing to do)
    unsafe {
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

/// The cursor moved by this many pixels.
pub fn move_by(dx: i32, dy: i32) {
    if dx != 0 || dy != 0 {
        send(MOUSEEVENTF_MOVE, dx, dy);
    }
}

/// The left button pressed or let go.
pub fn left_button(down: bool) {
    send(
        if down {
            MOUSEEVENTF_LEFTDOWN
        } else {
            MOUSEEVENTF_LEFTUP
        },
        0,
        0,
    );
}

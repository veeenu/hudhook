// Here are the default values
pub static mut SHOW_UI: bool = true; // Show the UI by default when injected

// Here are defined the default keycode (keycode as u16, None = disabled function)
pub static mut SHOW_CURSOR_KEY: Option<u16> = None; // Define the keycode for hide / show the cursor
pub static mut SHOW_UI_KEY: Option<u16> = None; // Define the keycode for hide / show the ui

// Don't touch this. This is used to store the global io keypress for toggle configs
pub static mut KEY_PRESS: [bool; 652] = [false; 652];

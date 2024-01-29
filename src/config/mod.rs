// Here are the default values
pub static mut SHOW_UI: bool = true; // Show the UI by default when injected
pub static mut FORCE_SHOW_CURSOR_ON_UI_SHOW: bool = false; // Force cursor to show when ui not hidden

// Here are defined the default keycode (keycode as u16, None = disabled function)
pub static mut SHOW_UI_KEY: Option<u16> = None; // Define the keycode for hide / show the ui
pub static mut SHOW_CURSOR_KEY: Option<u16> = None; // Define the keycode for hide / show the cursor

// Don't touch this. This is used to store the global io keypress for toggle configs
pub static mut KEY_PRESS: [bool; 652] = [false; 652];

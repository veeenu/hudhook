pub mod dx11;
pub mod dx12;

#[inline]
pub fn loword(l: u32) -> u16 {
    (l & 0xffff) as u16
}
#[inline]
pub fn hiword(l: u32) -> u16 {
    ((l >> 16) & 0xffff) as u16
}

#[inline]
pub fn get_wheel_delta_wparam(wparam: u32) -> u16 {
    hiword(wparam) as u16
}

#[inline]
pub fn get_xbutton_wparam(wparam: u32) -> u16 {
    hiword(wparam)
}


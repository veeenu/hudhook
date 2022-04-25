use winapi::shared::winerror::*;

pub fn check_hresult(h: HRESULT) {
    let h = windows::HRESULT(h as _);
    if h.is_err() {
        panic!("{}", h.message());
    }
}

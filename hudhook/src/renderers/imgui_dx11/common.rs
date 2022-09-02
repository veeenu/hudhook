use windows::core::HRESULT;

pub fn check_hresult(h: HRESULT) {
    if h.is_err() {
        panic!("{}", h.message());
    }
}

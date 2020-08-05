use std::ffi::CString;
#[derive(Debug)]
pub struct Error(pub String);

impl From<String> for Error {
  fn from(s: String) -> Error {
    Error(s)
  }
}

pub type Result<T> = std::result::Result<T, Error>;

#[repr(C)]
pub struct VERTEX_CONSTANT_BUFFER(pub [[f32; 4]; 4]);

//
// A reckless implementation of a conversion from
// a string to raw C char data. Pls only use with
// static const strings.
//

pub unsafe fn reckless_string(s: &str) -> CString {
  CString::new(s).unwrap()
}

//
// Convert pointer to ref, emit error if null
//

pub fn ptr_as_ref<'a, T>(ptr: *const T) -> Result<&'a T> {
  match unsafe { ptr.as_ref() } {
    Some(t) => Ok(t),
    None => Err(format!("Null pointer").into()),
  }
}


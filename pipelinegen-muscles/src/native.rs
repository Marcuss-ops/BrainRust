#![allow(unsafe_code)]

use std::ffi::{CStr, CString};

unsafe extern "C" {
    fn pg_native_render_clip(
        input: *const std::os::raw::c_char,
        output: *const std::os::raw::c_char,
        width: u32,
        height: u32,
        error: *mut std::os::raw::c_char,
        capacity: usize,
    ) -> i32;
}

pub(crate) fn render_source_only(input: &str, output: &str, width: u32, height: u32) -> Result<(), String> {
    let input = CString::new(input).map_err(|_| "native input path contains NUL".to_string())?;
    let output = CString::new(output).map_err(|_| "native output path contains NUL".to_string())?;
    let mut error = vec![0i8; 1024];
    let code = unsafe {
        pg_native_render_clip(input.as_ptr(), output.as_ptr(), width, height, error.as_mut_ptr(), error.len())
    };
    if code == 0 {
        return Ok(());
    }
    let message = unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy().into_owned();
    Err(if message.is_empty() { format!("native renderer failed ({code})") } else { message })
}

#[cfg(target_arch = "x86_64")]
#[allow(clippy::erasing_op, clippy::identity_op)]
mod x86_64;
#[cfg(target_arch = "x86_64")]
pub use x86_64::plt_callback_trampoline as trampoline;

use super::JNI;

#[repr(C)]
pub struct PltData {
    jni: *mut JNI,
    plt_callback: unsafe extern "C" fn(jni: *mut JNI, reloc_index: usize) -> usize,
}

impl PltData {
    pub fn new(jni: *mut JNI) -> Self {
        Self { jni, plt_callback }
    }
}

extern "C" fn plt_callback(jni_addr: *mut JNI, reloc_index: usize) -> usize {
    let jni_ptr = unsafe { &mut *jni_addr };
    jni_ptr.plt_callback(reloc_index).unwrap_or(0xBADBABE)
}

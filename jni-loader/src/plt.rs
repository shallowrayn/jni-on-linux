#[allow(clippy::erasing_op, clippy::identity_op)]
#[cfg_attr(target_arch = "x86_64", path = "plt/x86_64.rs")]
#[cfg_attr(target_arch = "aarch64", path = "plt/aarch64.rs")]
mod asm;
pub use asm::plt_callback_trampoline as trampoline;

use super::JNI;

#[repr(C)]
pub struct PltData {
    pub jni: *mut JNI,
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

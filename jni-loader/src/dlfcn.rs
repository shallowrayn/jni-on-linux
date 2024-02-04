#[cfg_attr(target_arch = "x86_64", path = "dlfcn/x86_64.rs")]
#[cfg_attr(target_arch = "aarch64", path = "dlfcn/aarch64.rs")]
mod asm;
use std::{
    ffi::{c_char, c_int, c_void, CStr},
    fs::File,
    num::NonZeroUsize,
};

use nix::{
    libc::memcpy,
    sys::mman::{mmap, mprotect, munmap, MapFlags, ProtFlags},
    unistd::{sysconf, SysconfVar},
};

use super::{Error, JNI, UNDEFINED_SYMBOL_VALUE};

pub struct DlopenSymbols {
    mapping_base: usize,
    mapping_size: usize,
    pub dlopen: *const (),
    pub dlsym: *const (),
    pub dlclose: *const (),
}

impl DlopenSymbols {
    pub fn new(jni: *const JNI) -> Result<Self, Error> {
        let page_size = sysconf(SysconfVar::PAGE_SIZE).map_err(|e| Error::MemoryMapFailed(e.to_string()))?;
        let Some(page_size) = page_size else {
            return Err(Error::MemoryMapFailed("Failed to get page size".to_string()));
        };
        let page_size = page_size as usize;

        let space_needed = 3 * asm::TRAMPOLINE_SIZE;
        let mapping_size = (space_needed + page_size - 1) & !(page_size - 1);
        let mapping_base = match unsafe {
            mmap::<File>(
                None,
                NonZeroUsize::new_unchecked(mapping_size),
                ProtFlags::PROT_READ | ProtFlags::PROT_WRITE | ProtFlags::PROT_EXEC,
                MapFlags::MAP_PRIVATE | MapFlags::MAP_ANONYMOUS,
                None,
                0,
            )
        } {
            Ok(base) => base as usize,
            Err(errno) => return Err(Error::MemoryMapFailed(errno.to_string())),
        };
        let trampoline_ptr = asm::jni_dlfcn_trampoline as *const c_void;

        let dlopen_addr = mapping_base;
        unsafe { memcpy(dlopen_addr as *mut c_void, trampoline_ptr, asm::TRAMPOLINE_SIZE) };
        let dlopen_jni_addr = dlopen_addr + asm::JNI_OFFSET;
        unsafe { *(dlopen_jni_addr as *mut _) = jni };
        let dlopen_fn_addr = dlopen_addr + asm::FN_OFFSET;
        unsafe { *(dlopen_fn_addr as *mut _) = jni_dlopen_callback as usize };
        let dlopen_code_addr = dlopen_addr + asm::CODE_OFFSET;

        let dlsym_addr = mapping_base + asm::TRAMPOLINE_SIZE;
        unsafe { memcpy(dlsym_addr as *mut c_void, trampoline_ptr, asm::TRAMPOLINE_SIZE) };
        let dlsym_jni_addr = dlsym_addr + asm::JNI_OFFSET;
        unsafe { *(dlsym_jni_addr as *mut _) = jni };
        let dlsym_fn_addr = dlsym_addr + asm::FN_OFFSET;
        unsafe { *(dlsym_fn_addr as *mut _) = jni_dlsym_callback as usize };
        let dlsym_code_addr = dlsym_addr + asm::CODE_OFFSET;

        let dlclose_addr = mapping_base + 2 * asm::TRAMPOLINE_SIZE;
        unsafe { memcpy(dlclose_addr as *mut c_void, trampoline_ptr, asm::TRAMPOLINE_SIZE) };
        let dlclose_jni_addr = dlclose_addr + asm::JNI_OFFSET;
        unsafe { *(dlclose_jni_addr as *mut _) = jni };
        let dlclose_fn_addr = dlclose_addr + asm::FN_OFFSET;
        unsafe { *(dlclose_fn_addr as *mut _) = jni_dlclose_callback as usize };
        let dlclose_code_addr = dlclose_addr + asm::CODE_OFFSET;

        if let Err(errno) =
            unsafe { mprotect(mapping_base as *mut c_void, mapping_size, ProtFlags::PROT_READ | ProtFlags::PROT_EXEC) }
        {
            let _ = unsafe { munmap(mapping_base as *mut c_void, mapping_size) };
            return Err(Error::MemoryMapFailed(errno.to_string()));
        }

        Ok(Self {
            mapping_base,
            mapping_size,
            dlopen: dlopen_code_addr as *const (),
            dlsym: dlsym_code_addr as *const (),
            dlclose: dlclose_code_addr as *const (),
        })
    }
}

impl Drop for DlopenSymbols {
    fn drop(&mut self) {
        let _ = unsafe { munmap(self.mapping_base as *mut c_void, self.mapping_size) };
    }
}

unsafe extern "C" fn jni_dlopen_callback(jni_ptr: *mut JNI, filename: *const c_char, flags: c_int) -> *const c_void {
    let ptr = (*jni_ptr).dlopen(CStr::from_ptr(filename).to_str().unwrap(), flags);
    ptr.map(|ptr| ptr as *const c_void).unwrap_or(std::ptr::null())
}

unsafe extern "C" fn jni_dlsym_callback(jni_ptr: *mut JNI, handle: *mut JNI, symbol: *const c_char) -> *const c_void {
    let symbol_addr = (*jni_ptr).dlsym(&mut *handle, CStr::from_ptr(symbol).to_str().unwrap());
    symbol_addr.unwrap_or(UNDEFINED_SYMBOL_VALUE) as *const c_void
}

unsafe extern "C" fn jni_dlclose_callback(jni_ptr: *mut JNI, handle: *mut JNI) -> c_int {
    (*jni_ptr).dlclose(&mut *handle)
}

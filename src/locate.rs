use std::{env, fs, path::PathBuf};
#[cfg(target_os = "linux")]
use std::{ffi::CStr, os::raw::c_char};

#[cfg(target_os = "linux")]
use auxv::getauxval::Getauxval;
use log::trace;

/// Search for a library using ld.so's search order. Custom search paths can
/// be specified and will take priority
pub fn locate_library(name: &str, extra_paths: Option<Vec<PathBuf>>) -> Option<PathBuf> {
    locate_library_internal(name, extra_paths, None, None)
}

pub(crate) fn locate_library_internal(
    name: &str, extra_paths: Option<Vec<PathBuf>>, parent_path: Option<PathBuf>, dt_runpath: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(extra_paths) = extra_paths {
        for path in extra_paths {
            if let Some(lib_path) = check_directory(name, path) {
                return Some(lib_path);
            }
        }
    }

    if let Some(lib_path) = parent_path.clone().and_then(|p| check_directory(name, p)) {
        return Some(lib_path);
    }

    if let Ok(ld_library_path) = env::var("LD_LIBRARY_PATH") {
        let ld_library_path = replace_tokens(ld_library_path, parent_path);
        trace!("Checking LD_LIBRARY_PATH {ld_library_path}");
        for path in split_paths(&ld_library_path) {
            if let Some(lib_path) = check_directory(name, path) {
                return Some(lib_path);
            }
        }
    }

    if let Some(dt_runpath) = dt_runpath {
        trace!("Checking DT_RUNPATH {dt_runpath:?}");
        if let Some(lib_path) = check_directory(name, dt_runpath) {
            return Some(lib_path);
        }
    }

    #[cfg(target_pointer_width = "64")]
    {
        if let Some(lib_path) = check_directory(name, PathBuf::from("/lib64/")) {
            return Some(lib_path);
        }
        if let Some(lib_path) = check_directory(name, PathBuf::from("/usr/lib64/")) {
            return Some(lib_path);
        }
    }
    #[cfg(not(target_pointer_width = "64"))]
    {
        if let Some(lib_path) = check_directory(name, PathBuf::from("/lib32/")) {
            return Some(lib_path);
        }
        if let Some(lib_path) = check_directory(name, PathBuf::from("/usr/lib32/")) {
            return Some(lib_path);
        }
    }

    if let Some(lib_path) = check_directory(name, PathBuf::from("/lib/")) {
        return Some(lib_path);
    }
    if let Some(lib_path) = check_directory(name, PathBuf::from("/usr/lib/")) {
        return Some(lib_path);
    }

    None
}

fn check_directory(name: &str, directory: PathBuf) -> Option<PathBuf> {
    if !directory.exists() {
        return None;
    }
    trace!("Looking for {name} in {directory:?}");
    let mut file_path = directory.clone();
    file_path.push(name);
    if file_path.exists() {
        return Some(file_path);
    }

    if !directory.is_dir() {
        return None;
    }
    for entry in fs::read_dir(directory).ok()?.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let mut file_path = path;
        file_path.push(name);
        if file_path.exists() {
            return Some(file_path);
        }
    }
    None
}

fn replace_tokens(mut str: String, parent_path: Option<PathBuf>) -> String {
    if let Some(parent_path) = parent_path {
        if let Some(parent_path_str) = parent_path.to_str() {
            str = str.replace("$ORIGIN", parent_path_str);
            str = str.replace("${ORIGIN}", parent_path_str);
        }
    }

    #[cfg(target_pointer_width = "64")]
    let lib_replacement = "lib64";
    #[cfg(not(target_pointer_width = "64"))]
    let lib_replacement = "lib";
    str = str.replace("$LIB", lib_replacement);
    str = str.replace("${LIB}", lib_replacement);

    #[cfg(target_os = "linux")]
    {
        const AT_PLATFORM: auxv::AuxvType = 15;
        let aux = auxv::getauxval::NativeGetauxval {};
        if let Ok(platform) = aux.getauxval(AT_PLATFORM) {
            let platform = unsafe { CStr::from_ptr(platform as *const c_char) };
            let platform = String::from_utf8_lossy(platform.to_bytes()).to_string();
            str = str.replace("$PLATFORM", &platform);
            str = str.replace("${PLATFORM}", &platform);
        } else if let Ok(results) = auxv::procfs::search_procfs_auxv(&[AT_PLATFORM]) {
            let platform = *results.get(&AT_PLATFORM).unwrap();
            let platform = unsafe { CStr::from_ptr(platform as *const c_char) };
            let platform = String::from_utf8_lossy(platform.to_bytes()).to_string();
            str = str.replace("$PLATFORM", &platform);
            str = str.replace("${PLATFORM}", &platform);
        } else {
            panic!("Failed to resolve AT_PLATFORM");
        }
    }

    str
}

fn split_paths(input: &str) -> Vec<PathBuf> {
    if input.find(':').is_some() {
        input.split(':').map(PathBuf::from).collect()
    } else if input.find(';').is_some() {
        input.split(';').map(PathBuf::from).collect()
    } else {
        vec![PathBuf::from(input)]
    }
}

use std::path::PathBuf;

use thiserror::Error;

use super::locate;

pub struct JNI {
    path: PathBuf,
}

impl JNI {
    pub fn new(path: PathBuf) -> Result<Self, Error> {
        if !path.exists() {
            return Err(Error::FileNotFound);
        }

        Ok(Self { path })
    }

    pub fn new_from_name(name: &str) -> Result<Self, Error> {
        match locate::locate_library(name, None) {
            Some(lib_path) => Self::new(lib_path),
            None => Err(Error::FileNotFound),
        }
    }
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("failed to find file")]
    FileNotFound,
}

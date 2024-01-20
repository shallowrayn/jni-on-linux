use std::{fs::File, path::PathBuf};

use elf::{abi::ET_DYN, endian::AnyEndian, ElfStream};
use thiserror::Error;

use super::{locate, mmap::MemoryMapping};

pub struct JNI {
    path: PathBuf,
    elf_file: ElfStream<AnyEndian, File>,
    mapping: MemoryMapping,
}

impl JNI {
    pub fn new(path: PathBuf) -> Result<Self, Error> {
        if !path.exists() {
            return Err(Error::FileNotFound);
        }
        let Ok(file) = File::open(path.clone()) else {
            return Err(Error::FailedToOpen);
        };
        let Ok(mapping_file) = File::open(path.clone()) else {
            return Err(Error::FailedToOpen);
        };
        let Ok(elf_file) = ElfStream::open_stream(file) else {
            return Err(Error::FailedToOpen);
        };
        if elf_file.ehdr.e_type != ET_DYN {
            return Err(Error::NotDynamicObject);
        }
        let mapping = match MemoryMapping::new(mapping_file, elf_file.segments()) {
            Ok(mapping) => mapping,
            Err(error) => return Err(Error::MemoryMapFailed(error)),
        };

        Ok(Self { path, elf_file, mapping })
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
    #[error("failed to open file")]
    FailedToOpen,
    #[error("the file is not a shared object file")]
    NotDynamicObject,
    #[error("failed to map memory - {0}")]
    MemoryMapFailed(String),
}

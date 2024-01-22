use std::{collections::HashMap, fs::File, path::PathBuf, rc::Rc, sync::Mutex};

use elf::{
    abi::{DT_NEEDED, DT_RUNPATH, ET_DYN},
    endian::AnyEndian,
    ElfStream,
};
use thiserror::Error;

use super::{locate, mmap::MemoryMapping};

pub struct JNI {
    path: PathBuf,
    elf_file: ElfStream<AnyEndian, File>,
    mapping: MemoryMapping,
    dependencies: HashMap<String, Option<Rc<Mutex<JNI>>>>,
    loaded_dependencies: bool,
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

        Ok(Self { path, elf_file, mapping, dependencies: HashMap::new(), loaded_dependencies: false })
    }

    pub fn new_from_name(name: &str) -> Result<Self, Error> {
        match locate::locate_library(name, None) {
            Some(lib_path) => Self::new(lib_path),
            None => Err(Error::FileNotFound),
        }
    }

    pub fn add_dependency(&mut self, name: &str, lib: Option<JNI>) {
        self.dependencies.insert(name.to_string(), lib.map(Mutex::new).map(Rc::new));
    }

    pub fn add_shared_dependency(&mut self, name: &str, lib: Option<Rc<Mutex<JNI>>>) {
        self.dependencies.insert(name.to_string(), lib);
    }

    pub fn load_dependencies(&mut self) {
        if self.loaded_dependencies {
            return;
        }
        self.loaded_dependencies = true;

        // Dependencies are stored using DT_NEEDED keys in the .dynamic section. We also need DT_RUNPATH for locating
        let Ok(Some(dynamic_section)) = self.elf_file.dynamic() else {
            return;
        };
        let mut dependency_offsets = Vec::new();
        let mut dt_runpath_offset = None;
        for entry in dynamic_section {
            match entry.d_tag {
                DT_NEEDED => {
                    dependency_offsets.push(entry.d_val() as usize);
                },
                DT_RUNPATH => {
                    dt_runpath_offset = Some(entry.d_val() as usize);
                },
                _ => {},
            }
        }

        // Values from .dynamic are offsets into the .dynstr string table
        let Ok(Some(dynamic_string_table_header)) = self.elf_file.section_header_by_name(".dynstr") else {
            return;
        };
        let dynamic_string_table_header = *dynamic_string_table_header; // End mutable borrow of self.elf_file
        let Ok(dynamic_string_table) = self.elf_file.section_data_as_strtab(&dynamic_string_table_header) else {
            return;
        };
        let dependencies: Vec<String> = dependency_offsets
            .into_iter()
            .flat_map(|offset| dynamic_string_table.get(offset))
            .map(|s| s.to_string())
            .collect();

        // Loop through dependencies, if they haven't been overridden then try to locate and load them
        let parent_dir = self.path.parent().map(PathBuf::from);
        let dt_runpath = dt_runpath_offset.and_then(|offset| dynamic_string_table.get(offset).ok()).map(PathBuf::from);
        for lib_name in dependencies {
            if self.dependencies.contains_key(&lib_name) {
                continue;
            }
            match locate::locate_library_internal(&lib_name, None, parent_dir.clone(), dt_runpath.clone()) {
                Some(lib_path) => {
                    self.dependencies.insert(lib_name, JNI::new(lib_path).ok().map(Mutex::new).map(Rc::new));
                },
                None => {
                    self.dependencies.insert(lib_name, None);
                },
            }
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

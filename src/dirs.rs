use crate::prelude::*;
use std::env;
use std::ffi::OsStr;
use std::path::PathBuf;

lazy_static! {
    pub static ref WORK_DIR: PathBuf = {
        env::var_os("CRATER_WORK_DIR")
            .unwrap_or_else(|| OsStr::new("work").to_os_string())
            .into()
    };
    pub static ref LOCAL_DIR: PathBuf = WORK_DIR.join("local");

    pub static ref CARGO_HOME: String = LOCAL_DIR.join("cargo-home").to_string_lossy().into();
    pub static ref RUSTUP_HOME: String = LOCAL_DIR.join("rustup-home").to_string_lossy().into();

    pub static ref EXPERIMENT_DIR: PathBuf = WORK_DIR.join("ex");
    pub static ref LOG_DIR: PathBuf = WORK_DIR.join("logs");

    pub static ref LOCAL_CRATES_DIR: PathBuf = "local-crates".into();
}

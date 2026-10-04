use std::path::PathBuf;

const APP_DIR: &str = "ZServiceManager";

fn env_dir(var: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_DIR)
}

/// `%APPDATA%\ZServiceManager`
pub fn config_dir() -> PathBuf {
    env_dir("APPDATA")
}

pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

/// `%LOCALAPPDATA%\ZServiceManager`
pub fn data_dir() -> PathBuf {
    env_dir("LOCALAPPDATA")
}

pub fn logs_dir() -> PathBuf {
    data_dir().join("logs")
}

pub fn app_log_file() -> PathBuf {
    data_dir().join("zsm.log")
}

pub fn resume_file() -> PathBuf {
    data_dir().join("resume.json")
}

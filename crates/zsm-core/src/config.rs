use std::collections::{BTreeMap, HashSet};
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub start_minimized: bool,
    pub check_updates: bool,
    pub update_check_interval_hours: u64,
    pub scrollback_lines: usize,
    pub log_retention_days: u32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            start_minimized: true,
            check_updates: true,
            update_check_interval_hours: 6,
            scrollback_lines: 10_000,
            log_retention_days: 14,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Console,
    Gui,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IoMode {
    #[default]
    Pty,
    Pipe,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicy {
    Never,
    #[default]
    OnFailure,
    Always,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: Kind,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub working_dir: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub io_mode: IoMode,
    #[serde(default)]
    pub autostart: bool,
    #[serde(default)]
    pub restart: RestartPolicy,
    #[serde(default = "default_restart_max")]
    pub restart_max: u32,
    #[serde(default = "default_restart_window")]
    pub restart_window_secs: u64,
    #[serde(default)]
    pub stop_command: String,
    #[serde(default = "default_stop_timeout")]
    pub stop_timeout_secs: u64,
    #[serde(default = "default_ctrl_c_timeout")]
    pub ctrl_c_timeout_secs: u64,
}

fn default_restart_max() -> u32 {
    5
}
fn default_restart_window() -> u64 {
    600
}
fn default_stop_timeout() -> u64 {
    60
}
fn default_ctrl_c_timeout() -> u64 {
    15
}

impl ServiceConfig {
    pub fn new(id: &str, command: &str) -> Self {
        Self {
            id: id.to_owned(),
            name: String::new(),
            kind: Kind::Console,
            command: command.to_owned(),
            args: Vec::new(),
            working_dir: String::new(),
            env: BTreeMap::new(),
            io_mode: IoMode::Pty,
            autostart: false,
            restart: RestartPolicy::OnFailure,
            restart_max: default_restart_max(),
            restart_window_secs: default_restart_window(),
            stop_command: String::new(),
            stop_timeout_secs: default_stop_timeout(),
            ctrl_c_timeout_secs: default_ctrl_c_timeout(),
        }
    }

    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            &self.id
        } else {
            &self.name
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() {
            return Err("id 가 비어 있음".into());
        }
        if !self
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(format!(
                "id `{}` 에는 영문, 숫자, -, _ 만 쓸 수 있음",
                self.id
            ));
        }
        if self.command.trim().is_empty() {
            return Err("command 가 비어 있음".into());
        }
        if self.restart_max == 0 {
            return Err("restart_max 는 1 이상이어야 함".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ServiceEntry {
    pub config: ServiceConfig,
    pub error: Option<String>,
    /// Original table of an entry that failed to parse; written back untouched on save.
    raw: Option<toml::Table>,
}

impl ServiceEntry {
    pub fn valid(config: ServiceConfig) -> Self {
        Self {
            config,
            error: None,
            raw: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    pub app: AppSettings,
    pub services: Vec<ServiceEntry>,
    /// Whole-file parse error; the app keeps running with defaults.
    pub error: Option<String>,
}

pub const DEFAULT_CONFIG: &str = r#"[app]
start_minimized = true
check_updates = true
update_check_interval_hours = 6
scrollback_lines = 10000
log_retention_days = 14

# [[service]]
# id = "vintagestory"
# name = "Vintage Story Server"
# kind = "console"
# command = '%APPDATA%\VintageStory\VintageStoryServer.exe'
# args = []
# working_dir = '%APPDATA%\VintageStory'
# env = {}
# io_mode = "pty"
# autostart = true
# restart = "on-failure"
# restart_max = 5
# restart_window_secs = 600
# stop_command = "/stop"
# stop_timeout_secs = 60
# ctrl_c_timeout_secs = 15
"#;

impl Config {
    pub fn load_or_create(path: &Path) -> Config {
        match std::fs::read_to_string(path) {
            Ok(text) => Config::parse(&text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                if let Err(e) = std::fs::write(path, DEFAULT_CONFIG) {
                    log::error!("기본 설정 파일 생성 실패 {}: {e}", path.display());
                }
                Config::parse(DEFAULT_CONFIG)
            }
            Err(e) => Config {
                error: Some(format!("설정 파일을 읽지 못함: {e}")),
                ..Config::default()
            },
        }
    }

    pub fn parse(text: &str) -> Config {
        let mut table: toml::Table = match text.parse() {
            Ok(t) => t,
            Err(e) => {
                return Config {
                    error: Some(format!("설정 파일 구문 오류: {e}")),
                    ..Config::default()
                }
            }
        };
        let mut error = None;
        let app = match table.remove("app") {
            Some(v) => v.try_into().unwrap_or_else(|e| {
                error = Some(format!("[app] 오류, 기본값 사용: {e}"));
                AppSettings::default()
            }),
            None => AppSettings::default(),
        };
        let raw_services = match table.remove("service") {
            Some(toml::Value::Array(items)) => items,
            Some(_) => {
                error = Some("service 는 [[service]] 배열이어야 함".into());
                Vec::new()
            }
            None => Vec::new(),
        };
        let mut services: Vec<ServiceEntry> = raw_services
            .into_iter()
            .enumerate()
            .map(|(i, v)| parse_service(i, v))
            .collect();
        let mut seen = HashSet::new();
        for entry in &mut services {
            if !seen.insert(entry.config.id.clone()) && entry.error.is_none() {
                entry.error = Some(format!("id `{}` 가 중복됨", entry.config.id));
            }
        }
        Config {
            app,
            services,
            error,
        }
    }

    pub fn to_toml(&self) -> String {
        #[derive(Serialize)]
        #[serde(untagged)]
        enum Entry<'a> {
            Parsed(&'a ServiceConfig),
            Raw(&'a toml::Table),
        }
        #[derive(Serialize)]
        struct File<'a> {
            app: &'a AppSettings,
            #[serde(skip_serializing_if = "Vec::is_empty")]
            service: Vec<Entry<'a>>,
        }
        let file = File {
            app: &self.app,
            service: self
                .services
                .iter()
                .map(|e| match &e.raw {
                    Some(raw) => Entry::Raw(raw),
                    None => Entry::Parsed(&e.config),
                })
                .collect(),
        };
        toml::to_string(&file).expect("config serialize")
    }

    /// Writes the config, keeping the previous file as `config.toml.bak`.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if path.exists() {
            std::fs::copy(path, path.with_extension("toml.bak"))?;
        } else if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, self.to_toml())?;
        std::fs::rename(&tmp, path)
    }
}

fn parse_service(index: usize, value: toml::Value) -> ServiceEntry {
    let toml::Value::Table(raw) = value else {
        let mut config = ServiceConfig::new(&format!("service-{}", index + 1), "");
        config.name = config.id.clone();
        return ServiceEntry {
            config,
            error: Some("[[service]] 항목이 테이블이 아님".into()),
            raw: None,
        };
    };
    match toml::Value::Table(raw.clone()).try_into::<ServiceConfig>() {
        Ok(config) => {
            let error = config.validate().err();
            ServiceEntry {
                config,
                error,
                raw: None,
            }
        }
        Err(e) => {
            let text = |key: &str| raw.get(key).and_then(|v| v.as_str()).map(str::to_owned);
            let id = text("id").unwrap_or_else(|| format!("service-{}", index + 1));
            let mut config = ServiceConfig::new(&id, &text("command").unwrap_or_default());
            config.name = text("name").unwrap_or_default();
            ServiceEntry {
                config,
                error: Some(e.message().to_owned()),
                raw: Some(raw),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_parses_without_services() {
        let c = Config::parse(DEFAULT_CONFIG);
        assert_eq!(c.error, None);
        assert_eq!(c.app, AppSettings::default());
        assert!(c.services.is_empty());
    }

    #[test]
    fn bad_service_is_isolated() {
        let c = Config::parse(
            r#"
            [[service]]
            id = "ok"
            command = "a.exe"
            restart = "always"

            [[service]]
            id = "bad"
            command = "b.exe"
            kind = "tui"

            [[service]]
            id = "bad id"
            command = "c.exe"

            [[service]]
            id = "ok"
            command = "d.exe"
            "#,
        );
        assert_eq!(c.services.len(), 4);
        assert_eq!(c.services[0].error, None);
        assert_eq!(c.services[0].config.restart, RestartPolicy::Always);
        assert!(c.services[1].error.as_deref().unwrap().contains("tui"));
        assert!(c.services[2].error.is_some());
        assert!(c.services[3].error.as_deref().unwrap().contains("중복"));
    }

    #[test]
    fn round_trip_keeps_unparsable_entries() {
        let c = Config::parse(
            r#"
            [[service]]
            id = "bad"
            command = "b.exe"
            kind = "tui"
            "#,
        );
        let again = Config::parse(&c.to_toml());
        assert_eq!(again.services[0].config.id, "bad");
        assert!(again.services[0].error.is_some());
    }

    #[test]
    fn round_trip_valid_service() {
        let mut svc = ServiceConfig::new("vs", r"%APPDATA%\x.exe");
        svc.env.insert("A".into(), "1".into());
        svc.stop_command = "/stop".into();
        let c = Config {
            services: vec![ServiceEntry::valid(svc.clone())],
            ..Config::default()
        };
        assert_eq!(Config::parse(&c.to_toml()).services[0].config, svc);
    }
}

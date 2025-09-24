use std::sync::{LazyLock, RwLock, RwLockReadGuard};

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(default)]
pub struct Config {
    pub http_port: Option<u16>,
    pub comfyui_address: String,
    /// Passed as `Bearer {comfyui_authentication}` in the `Authorization` header
    /// to the ComfyUI server (i.e. <https://github.com/ai-dock/comfyui>)
    pub comfyui_authentication: Option<String>,
    /// If true, the server will accept self-signed certificates from the ComfyUI server
    pub comfyui_https_insecure: bool,
    pub workflow_settings: prismata_server_lib::comfyui::WorkflowSettings,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            http_port: None,
            comfyui_address: "http://127.0.0.1:8188".into(),
            comfyui_authentication: None,
            comfyui_https_insecure: false,
            workflow_settings: Default::default(),
        }
    }
}
static INSTANCE: LazyLock<RwLock<Config>> = LazyLock::new(|| RwLock::new(Config::load()));
impl Config {
    const FILE: &'static str = "config_server.toml";
    fn load() -> Self {
        let config = if std::fs::exists(Self::FILE).unwrap() {
            let config_file = std::fs::read_to_string(Self::FILE).unwrap();
            toml::from_str(&config_file).unwrap()
        } else {
            Config::default()
        };
        config.save();
        config
    }
    fn save(&self) {
        std::fs::write(Self::FILE, toml::to_string_pretty(self).unwrap()).unwrap();
    }
    pub fn read() -> RwLockReadGuard<'static, Config> {
        INSTANCE.read().unwrap()
    }
    pub fn _write<R>(f: impl FnOnce(&mut Config) -> R) -> R {
        let old_config = Self::read().clone();
        let mut config = old_config.clone();
        let result = f(&mut config);
        if config != old_config {
            config.save();
            *INSTANCE.write().unwrap() = config;
        }
        result
    }
}

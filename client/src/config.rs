use std::sync::{LazyLock, RwLock, RwLockReadGuard};

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(default)]
pub struct Config {
    #[cfg(feature = "native")]
    pub width: u32,
    #[cfg(feature = "native")]
    pub height: u32,

    #[cfg(feature = "native")]
    pub server: Option<Server>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            #[cfg(feature = "native")]
            width: 1280,
            #[cfg(feature = "native")]
            height: 720,

            #[cfg(feature = "native")]
            server: Some(Server::default()),
        }
    }
}
#[cfg(feature = "native")]
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(default)]
/// Listen server settings that are passed to the server builder.
pub struct Server {
    pub comfyui_address: String,
    /// Passed as `Bearer {comfyui_authentication}` in the `Authorization` header
    /// to the ComfyUI server (i.e. <https://github.com/ai-dock/comfyui>)
    pub comfyui_authentication: Option<String>,
    /// If true, the server will accept self-signed certificates from the ComfyUI server
    pub comfyui_https_insecure: bool,
    pub workflow_settings: prismata_server_lib::comfyui::WorkflowSettings,
}
#[cfg(feature = "native")]
impl Default for Server {
    fn default() -> Self {
        Self {
            comfyui_address: "http://127.0.0.1:8188".into(),
            comfyui_authentication: None,
            comfyui_https_insecure: false,
            workflow_settings: Default::default(),
        }
    }
}
static INSTANCE: LazyLock<RwLock<Config>> = LazyLock::new(|| RwLock::new(Config::load()));
impl Config {
    #[cfg(feature = "native")]
    const FILE: &'static str = "config_client.toml";
    #[cfg(feature = "native")]
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
    #[cfg(not(feature = "native"))]
    fn load() -> Self {
        // Not persistent on web for now
        Config::default()
    }
    #[cfg(feature = "native")]
    fn save(&self) {
        std::fs::write(Self::FILE, toml::to_string_pretty(self).unwrap()).unwrap();
    }
    #[cfg(not(feature = "native"))]
    fn save(&self) {
        // No-op on web for now
    }
    pub fn read() -> RwLockReadGuard<'static, Config> {
        INSTANCE.read().unwrap()
    }
    #[allow(dead_code)]
    pub fn write<R>(f: impl FnOnce(&mut Config) -> R) -> R {
        let old_config = Self::read().clone();
        let mut config = old_config.clone();
        let result = f(&mut config);
        if config != old_config {
            config.save();
            *INSTANCE.write().unwrap() = config;
        }
        result
    }
    #[cfg(feature = "native")]
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

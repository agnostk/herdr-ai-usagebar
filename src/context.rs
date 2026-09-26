//! The runtime environment herdr injects into plugin commands.

use std::path::PathBuf;

use anyhow::{Context as _, Result};

use crate::config::{self, Config};
use crate::herdr::Herdr;
use crate::session::Session;

pub struct Context {
    pub plugin_id: String,
    pub state_dir: PathBuf,
    pub config_dir: PathBuf,
    pub socket: String,
    pub herdr_bin: PathBuf,
}

impl Context {
    pub fn from_env() -> Result<Self> {
        let var = |name: &str| {
            std::env::var(name).with_context(|| {
                format!(
                    "{name} is not set; run this through herdr, e.g. \
                     `herdr plugin action invoke agnostk.ai-usagebar.refresh`"
                )
            })
        };
        Ok(Self {
            plugin_id: var("HERDR_PLUGIN_ID")?,
            state_dir: var("HERDR_PLUGIN_STATE_DIR")?.into(),
            config_dir: var("HERDR_PLUGIN_CONFIG_DIR")?.into(),
            socket: var("HERDR_SOCKET_PATH")?,
            herdr_bin: std::env::var_os("HERDR_BIN_PATH")
                .map_or_else(|| PathBuf::from("herdr"), PathBuf::from),
        })
    }

    pub fn herdr(&self) -> Herdr {
        Herdr::new(self.herdr_bin.clone(), &self.plugin_id)
    }

    pub fn session(&self) -> Session {
        Session::new(&self.state_dir, &self.socket)
    }

    pub fn config_path(&self) -> PathBuf {
        self.config_dir.join(config::FILE_NAME)
    }

    pub fn load_config(&self) -> Result<Config> {
        Config::load(&self.config_path())
    }
}

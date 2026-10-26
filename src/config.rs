use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub const DEFAULT_NETWORKS: &[&str] = &["Mugonat Systems 5.0", "Mugonat Systems 2.4"];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub target_networks: Vec<String>,
    pub auto_start: bool,
    pub revision: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            target_networks: DEFAULT_NETWORKS
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            auto_start: false,
            revision: 0,
        }
    }
}

impl Settings {
    pub fn load() -> Result<Self> {
        let path = settings_path()?;
        match fs::read_to_string(&path) {
            Ok(contents) => serde_json::from_str(&contents)
                .with_context(|| format!("Could not read settings at {}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("Could not read {}", path.display())),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = settings_path()?;
        let parent = path
            .parent()
            .context("The settings path has no parent directory")?;
        fs::create_dir_all(parent)
            .with_context(|| format!("Could not create {}", parent.display()))?;

        let json = serde_json::to_vec_pretty(self)?;
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, json)
            .with_context(|| format!("Could not write {}", temporary.display()))?;
        fs::rename(&temporary, &path)
            .with_context(|| format!("Could not replace {}", path.display()))?;
        Ok(())
    }
}

fn settings_path() -> Result<PathBuf> {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .context("Neither XDG_CONFIG_HOME nor HOME is set")?;
    Ok(config_home.join("mugonat-portal").join("settings.json"))
}

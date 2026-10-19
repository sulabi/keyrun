use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct OverlayConfig {
    pub window_background_opacity: f32,
    pub row_background_opacity: f32,
    pub row_borders: bool,
    pub show_command: bool,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            window_background_opacity: 0.0,
            row_background_opacity: 1.0,
            row_borders: true,
            show_command: true,
        }
    }
}

impl OverlayConfig {
    pub fn validate(&self) -> Result<()> {
        for (name, opacity) in [
            ("window_background_opacity", self.window_background_opacity),
            ("row_background_opacity", self.row_background_opacity),
        ] {
            anyhow::ensure!(
                (0.0..=1.0).contains(&opacity),
                "overlay.{name} must be between 0 and 1"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_and_opacity_validation() {
        let config: OverlayConfig = toml::from_str("").unwrap();
        assert_eq!(config.window_background_opacity, 0.0);
        assert_eq!(config.row_background_opacity, 1.0);
        assert!(config.row_borders);
        assert!(config.show_command);

        let hidden: OverlayConfig = toml::from_str("show_command = false").unwrap();
        assert!(!hidden.show_command);
        assert!(config.validate().is_ok());

        for source in [
            "row_background_opacity=1.1",
            "window_background_opacity=nan",
        ] {
            let config: OverlayConfig = toml::from_str(source).unwrap();
            assert!(config.validate().is_err());
        }
    }
}

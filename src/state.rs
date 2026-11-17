//! Window geometry and zoom, remembered between launches.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub position: Option<(i32, i32)>,
    pub size: (u32, u32),
    pub maximized: bool,
    pub zoom: f64,
    pub skin: bool,
}

impl Default for State {
    fn default() -> Self {
        Self { position: None, size: (0, 0), maximized: false, zoom: 1.0, skin: true }
    }
}

fn file(config: &Path) -> PathBuf {
    config.join("state.json")
}

impl State {
    pub fn load(config: &Path) -> Self {
        let mut state = Self::default();
        let Ok(text) = fs::read_to_string(file(config)) else { return state };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { return state };
        let int = |k: &str| v.get(k).and_then(Value::as_i64);
        if let (Some(x), Some(y)) = (int("x"), int("y")) {
            state.position = Some((x as i32, y as i32));
        }
        if let (Some(w), Some(h)) = (int("w"), int("h")) {
            if (400..=20_000).contains(&w) && (300..=20_000).contains(&h) {
                state.size = (w as u32, h as u32);
            }
        }
        state.maximized = v.get("maximized").and_then(Value::as_bool).unwrap_or(false);
        if let Some(z) = v.get("zoom").and_then(Value::as_f64) {
            state.zoom = z.clamp(0.5, 2.5);
        }
        state.skin = v.get("skin").and_then(Value::as_bool).unwrap_or(true);
        state
    }

    pub fn save(&self, config: &Path) {
        let v = json!({
            "x": self.position.map(|p| p.0),
            "y": self.position.map(|p| p.1),
            "w": self.size.0,
            "h": self.size.1,
            "maximized": self.maximized,
            "zoom": self.zoom,
            "skin": self.skin,
        });
        let _ = fs::create_dir_all(config);
        let tmp = config.join("state.json.tmp");
        if fs::write(&tmp, v.to_string()).is_ok() {
            let _ = fs::rename(tmp, file(config));
        }
    }
}

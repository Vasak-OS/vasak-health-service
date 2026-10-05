//! Si el registro está prendido, según `vasak.conf`.
//!
//! El interruptor lo escribe el escritorio (el tablero de tiempo de pantalla) y
//! Configuración, en la misma `vasak.conf` que el resto del sistema, con la
//! clave `screen_time.enabled`. El servicio la lee al arrancar; mientras corre,
//! el cambio le llega en el acto por `SetEnabled`.

use std::path::PathBuf;

/// La ruta de `vasak.conf`. `VASAK_CONFIG_PATH` la pisa, como en el resto del
/// sistema.
pub fn config_path() -> Option<PathBuf> {
    if let Some(set) = std::env::var_os("VASAK_CONFIG_PATH") {
        if !set.is_empty() {
            return Some(PathBuf::from(set));
        }
    }
    dirs::home_dir().map(|home| home.join(".config/vasak/vasak.conf"))
}

/// Si el contenido de `vasak.conf` deja contar: sí, salvo un
/// `screen_time.enabled: false` explícito. Un archivo ilegible o sin la clave no
/// apaga nada.
pub fn enabled_from_json(content: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .as_ref()
        .and_then(|config| config.get("screen_time"))
        .and_then(|section| section.get("enabled"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true)
}

/// Lee `vasak.conf` y dice si el registro está prendido.
pub fn read_enabled() -> bool {
    config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|content| enabled_from_json(&content))
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prendido_salvo_que_la_configuración_diga_que_no() {
        assert!(enabled_from_json("{}"));
        assert!(enabled_from_json("no es json"));
        assert!(enabled_from_json(r#"{"screen_time":{}}"#));
        assert!(enabled_from_json(r#"{"screen_time":{"enabled":"no"}}"#));
        assert!(enabled_from_json(r#"{"screen_time":{"enabled":true}}"#));
        assert!(!enabled_from_json(r#"{"screen_time":{"enabled":false}}"#));
    }
}

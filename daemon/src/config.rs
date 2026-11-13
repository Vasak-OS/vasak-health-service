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

/// Si la capa de gamificación está prendida, según `gamification.enabled` de
/// `vasak.conf`. Apagada por omisión: a diferencia del tiempo de pantalla, acá
/// la falta de la clave —o un archivo ilegible— deja la capa apagada, para no
/// despertar al servicio de gamificación por D-Bus sin que la persona la prenda.
pub fn gamification_enabled_from_json(content: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .as_ref()
        .and_then(|config| config.get("gamification"))
        .and_then(|section| section.get("enabled"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// Lee `vasak.conf` y dice si la gamificación está prendida; apagada si el
/// archivo no se puede leer.
pub fn read_gamification_enabled() -> bool {
    config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|content| gamification_enabled_from_json(&content))
        .unwrap_or(false)
}

/// Candado para las pruebas que pisan `VASAK_CONFIG_PATH`: la variable es del
/// proceso entero y las pruebas corren en paralelo, así que las que la tocan
/// (en este módulo, en `reminders` y en `gamification`) lo toman primero para
/// no leerse el archivo equivocado entre sí.
#[cfg(test)]
pub(crate) static CONFIG_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

    #[test]
    fn la_gamificación_está_apagada_salvo_que_la_configuración_la_prenda() {
        assert!(!gamification_enabled_from_json(""));
        assert!(!gamification_enabled_from_json("{}"));
        assert!(!gamification_enabled_from_json("no es json"));
        assert!(!gamification_enabled_from_json(
            r#"{"screen_time":{"enabled":true}}"#
        ));
        assert!(!gamification_enabled_from_json(r#"{"gamification":{}}"#));
        assert!(!gamification_enabled_from_json(
            r#"{"gamification":{"enabled":"yes"}}"#
        ));
        assert!(!gamification_enabled_from_json(
            r#"{"gamification":{"enabled":1}}"#
        ));
        assert!(!gamification_enabled_from_json(
            r#"{"gamification":{"enabled":false}}"#
        ));
        assert!(gamification_enabled_from_json(
            r#"{"gamification":{"enabled":true}}"#
        ));
    }

    #[test]
    fn la_gamificación_se_lee_de_la_ruta_de_vasak_conf() {
        use crate::store::tests::TempDir;
        // `VASAK_CONFIG_PATH` es del proceso: se serializa con las otras pruebas que la pisan.
        let _env = CONFIG_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let temp = TempDir::new("gamificacion");
        let file = temp.0.join("vasak.conf");
        std::fs::write(&file, r#"{"gamification":{"enabled":true}}"#)
            .expect("se escribe la configuración");

        std::env::set_var("VASAK_CONFIG_PATH", &file);
        let prendida = read_gamification_enabled();
        std::env::set_var("VASAK_CONFIG_PATH", temp.0.join("no-existe.conf"));
        let ausente = read_gamification_enabled();
        std::env::remove_var("VASAK_CONFIG_PATH");

        assert!(prendida);
        assert!(!ausente);
    }
}

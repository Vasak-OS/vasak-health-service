//! La categoría de cada aplicación, sacada de su `.desktop`.
//!
//! El informe no muestra sólo cuánto se usó cada aplicación, sino de qué tipo
//! es: redes, desarrollo, un juego. Eso ya lo declara cada aplicación en su
//! archivo `.desktop` (`Categories=`), con las categorías principales de
//! freedesktop. Acá se busca el `.desktop` que corresponde al `app-id` y se
//! elige su categoría principal.
//!
//! # Dónde se busca
//!
//! En las carpetas estándar de aplicaciones: la del usuario
//! (`$XDG_DATA_HOME/applications`) y las del sistema (`$XDG_DATA_DIRS`, que por
//! omisión son `/usr/share` y `/usr/local/share`). La del usuario primero, que
//! es la que pisa a las demás.
//!
//! # Se recuerda lo buscado
//!
//! Resolver un `app-id` abre y lee un archivo; el informe pregunta por las
//! mismas aplicaciones una y otra vez. Cada respuesta —incluida «no se
//! encontró», que si no se repetiría la búsqueda entera— queda en memoria.

use std::collections::HashMap;
use std::path::PathBuf;

use parking_lot::Mutex;

/// Las categorías principales de freedesktop, en orden de preferencia: la
/// primera que declare la aplicación es la que se muestra. Una aplicación suele
/// declarar una principal y varias secundarias (`Network;Email;` → `Network`).
const MAIN_CATEGORIES: [&str; 13] = [
    "AudioVideo",
    "Audio",
    "Video",
    "Development",
    "Education",
    "Game",
    "Graphics",
    "Network",
    "Office",
    "Science",
    "Settings",
    "System",
    "Utility",
];

pub struct Categories {
    dirs: Vec<PathBuf>,
    cache: Mutex<HashMap<String, String>>,
}

impl Categories {
    pub fn new() -> Self {
        Self {
            dirs: application_dirs(),
            cache: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    fn with_dirs(dirs: Vec<PathBuf>) -> Self {
        Self {
            dirs,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// La categoría principal de `app_id`, o vacío si no se pudo averiguar.
    pub fn of(&self, app_id: &str) -> String {
        if let Some(found) = self.cache.lock().get(app_id) {
            return found.clone();
        }
        let category = self.resolve(app_id);
        self.cache
            .lock()
            .insert(app_id.to_string(), category.clone());
        category
    }

    fn resolve(&self, app_id: &str) -> String {
        for dir in &self.dirs {
            // `app-id.desktop` tal cual, y en minúsculas por si el compositor da
            // el `app-id` con otra caja que el nombre del archivo.
            for name in [
                format!("{app_id}.desktop"),
                format!("{}.desktop", app_id.to_lowercase()),
            ] {
                let path = dir.join(&name);
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Some(category) = main_category(&content) {
                        return category;
                    }
                    // El archivo existe pero no declara una categoría principal:
                    // no tiene sentido seguir buscando otro con el mismo nombre.
                    return String::new();
                }
            }
        }
        String::new()
    }
}

/// Las carpetas de aplicaciones, de la más específica a la más general.
fn application_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(data) = dirs::data_dir() {
        dirs.push(data.join("applications"));
    }
    let system = std::env::var_os("XDG_DATA_DIRS")
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());
    for base in system.split(':').filter(|part| !part.is_empty()) {
        dirs.push(PathBuf::from(base).join("applications"));
    }
    dirs
}

/// La categoría principal declarada en un `.desktop`, si hay alguna.
///
/// Lee sólo el grupo `[Desktop Entry]`: una acción del archivo (`[Desktop
/// Action …]`) podría traer su propia línea `Categories=`, que no es la de la
/// aplicación.
fn main_category(content: &str) -> Option<String> {
    let mut in_entry = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some(value) = line.strip_prefix("Categories=") {
            return value
                .split(';')
                .map(str::trim)
                .find(|part| MAIN_CATEGORIES.contains(part))
                .map(str::to_string);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::TempDir;

    #[test]
    fn la_categoría_principal_es_la_primera_de_la_lista_que_es_principal() {
        // `Email` no es principal; `Network` sí, y viene primero.
        assert_eq!(
            main_category("[Desktop Entry]\nCategories=Network;Email;").as_deref(),
            Some("Network")
        );
        assert_eq!(
            main_category("[Desktop Entry]\nCategories=Game;").as_deref(),
            Some("Game")
        );
    }

    #[test]
    fn sin_categoría_principal_no_hay_categoría() {
        assert_eq!(
            main_category("[Desktop Entry]\nCategories=Email;Chat;"),
            None
        );
        assert_eq!(main_category("[Desktop Entry]\nName=Algo"), None);
        assert_eq!(main_category(""), None);
    }

    #[test]
    fn sólo_se_mira_el_grupo_de_la_aplicación_y_no_sus_acciones() {
        let desktop = "[Desktop Entry]\nName=Algo\n\n[Desktop Action Nueva]\nCategories=Game;";
        assert_eq!(main_category(desktop), None);
    }

    #[test]
    fn se_resuelve_desde_el_archivo_y_después_de_la_memoria() {
        let temp = TempDir::new("categorias");
        std::fs::write(
            temp.0.join("firefox.desktop"),
            "[Desktop Entry]\nCategories=Network;WebBrowser;",
        )
        .expect("se escribe");
        let categories = Categories::with_dirs(vec![temp.0.clone()]);

        assert_eq!(categories.of("firefox"), "Network");
        // Una que no existe queda como vacío, y recordada.
        assert_eq!(categories.of("no-existe"), "");
        assert!(categories.cache.lock().contains_key("no-existe"));
    }
}

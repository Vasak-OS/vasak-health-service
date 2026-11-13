//! La interfaz D-Bus: lo que el escritorio y Configuración pueden pedirle.
//!
//! Los nombres de los métodos en el bus son los mismos de Rust en PascalCase
//! (`push_sample` → `PushSample`), y están declarados una sola vez, igual, en el
//! proxy del crate `vasak-health-protocol`.

use zbus::fdo::Error as FdoError;
use zbus::interface;

use std::path::PathBuf;
use std::sync::Arc;

use vasak_health_protocol::{GamificationProgress, ScreenTimeReport};

use crate::gamification;
use crate::recorder::{parse_range, Recorder};

pub struct HealthService {
    pub recorder: Arc<Recorder>,
    /// Si la capa de gamificación está prendida. Se lee una vez al arrancar,
    /// igual que en el bucle: hoy no cambia en caliente.
    pub gamification_enabled: bool,
    /// Dónde vive el progreso de gamificación; `None` sin carpeta de datos.
    pub gamification_dir: Option<PathBuf>,
}

#[interface(name = "ar.net.vasak.os.Health")]
impl HealthService {
    // ── Ingreso: lo empuja el escritorio ──────────────────────────────────────

    /// Una muestra: la aplicación enfocada (vacío si ninguna) y el bloqueo.
    fn push_sample(&self, app_id: &str, locked: bool) {
        let app = Some(app_id).filter(|app| !app.trim().is_empty());
        self.recorder.on_sample(app, locked);
    }

    /// Empezó la inactividad hace `idle_ms` milisegundos.
    fn idle_started(&self, idle_ms: u64) {
        self.recorder.on_idle_started(idle_ms);
    }

    /// Volvió la actividad.
    fn idle_ended(&self) {
        self.recorder.on_idle_ended();
    }

    /// Se perdió la detección de inactividad.
    fn idle_monitor_lost(&self) {
        self.recorder.on_idle_monitor_lost();
    }

    /// Guarda todo lo contado: antes de cerrar la sesión o apagar.
    fn flush(&self) {
        self.recorder.save_everything();
    }

    // ── Consulta y control: el escritorio y Configuración ─────────────────────

    /// Los días de `from` a `to` (`AAAA-MM-DD`, los dos incluidos).
    fn screen_time(&self, from: &str, to: &str) -> Result<ScreenTimeReport, FdoError> {
        let (from, to) = parse_range(from, to).map_err(FdoError::InvalidArgs)?;
        Ok(self.recorder.report(from, to))
    }

    /// Borra todo el historial.
    fn clear_screen_time(&self) -> Result<(), FdoError> {
        self.recorder.clear().map_err(FdoError::Failed)
    }

    /// Prende o apaga el registro en el acto.
    fn set_enabled(&self, enabled: bool) {
        self.recorder.set_enabled(enabled);
    }

    /// Si el registro está prendido.
    fn enabled(&self) -> bool {
        self.recorder.is_enabled()
    }

    /// El progreso de gamificación: XP, racha y logros. Se lee del disco en cada
    /// consulta —el bucle lo guarda de forma atómica tras cada evento, así que
    /// está al día— y no se comparte estado con el bucle. Apagada la capa, no se
    /// lee nada y el informe viene en cero.
    fn gamification(&self) -> GamificationProgress {
        let progress = if self.gamification_enabled {
            self.gamification_dir
                .as_deref()
                .map(gamification::load)
                .unwrap_or_default()
        } else {
            gamification::Progress::default()
        };
        progress.report(self.gamification_enabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gamification::{self, Progress};
    use crate::recorder::Recorder;
    use crate::store::tests::TempDir;

    fn service(enabled: bool, dir: Option<PathBuf>) -> HealthService {
        HealthService {
            recorder: Arc::new(Recorder::new(None, false)),
            gamification_enabled: enabled,
            gamification_dir: dir,
        }
    }

    fn progreso_guardado(dir: &std::path::Path) {
        let progress = Progress {
            xp: 250,
            streak_days: 6,
            achievements: vec!["racha_3".to_string(), "racha_5".to_string()],
            ..Progress::default()
        };
        gamification::save(dir, &progress).expect("se guarda el progreso");
    }

    #[test]
    fn apagada_no_lee_el_disco_y_responde_en_cero() {
        let temp = TempDir::new("gamif-apagada");
        progreso_guardado(&temp.0);

        let informe = service(false, Some(temp.0.clone())).gamification();

        assert!(!informe.enabled);
        assert_eq!(informe.xp, 0);
        assert_eq!(informe.streak_days, 0);
        assert!(informe.achievements.is_empty());
    }

    #[test]
    fn prendida_devuelve_el_progreso_guardado() {
        let temp = TempDir::new("gamif-prendida");
        progreso_guardado(&temp.0);

        let informe = service(true, Some(temp.0.clone())).gamification();

        assert!(informe.enabled);
        assert_eq!(informe.xp, 250);
        assert_eq!(informe.streak_days, 6);
        assert_eq!(informe.achievements, vec!["racha_3", "racha_5"]);
    }

    #[test]
    fn prendida_sin_archivo_responde_en_cero() {
        let temp = TempDir::new("gamif-vacia");

        let informe = service(true, Some(temp.0.clone())).gamification();

        assert!(informe.enabled);
        assert_eq!(informe.xp, 0);
        assert!(informe.achievements.is_empty());
    }

    #[test]
    fn prendida_sin_carpeta_de_datos_responde_en_cero() {
        let informe = service(true, None).gamification();

        assert!(informe.enabled);
        assert_eq!(informe.xp, 0);
    }
}

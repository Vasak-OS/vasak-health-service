//! La interfaz D-Bus: lo que el escritorio y Configuración pueden pedirle.
//!
//! Los nombres de los métodos en el bus son los mismos de Rust en PascalCase
//! (`push_sample` → `PushSample`), y están declarados una sola vez, igual, en el
//! proxy del crate `vasak-health-protocol`.

use zbus::fdo::Error as FdoError;
use zbus::interface;

use std::sync::Arc;

use vasak_health_protocol::ScreenTimeReport;

use crate::recorder::{parse_range, Recorder};

pub struct HealthService {
    pub recorder: Arc<Recorder>,
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
}

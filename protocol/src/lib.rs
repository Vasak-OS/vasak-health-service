//! El contrato entre el servicio de salud, el escritorio y Configuración.
//!
//! Todo lo que viaja por D-Bus se define acá una sola vez, así los tres lados
//! no se separan: un campo renombrado en una punta deja de compilar en la otra
//! en lugar de volverse, en silencio, un informe que nunca cuadra.
//!
//! # Quién mide y quién pregunta
//!
//! El escritorio es el único que habla con el compositor (la ventana enfocada
//! por el socket de Wayfire) y con la inactividad (`ext-idle-notify-v1`). Ya
//! tiene esas dos conexiones para dibujar el panel, así que **las muestras las
//! empuja él** —qué aplicación está enfocada, si la pantalla está bloqueada,
//! cuándo empezó y terminó la inactividad— con los métodos de ingreso de abajo.
//! El servicio hace la contabilidad, la guarda, le pone categoría a cada
//! aplicación y la devuelve cuando el escritorio o Configuración la consultan.
//! Un segundo proceso abriendo sus propias conexiones a Wayland sería otra
//! conexión, otro permiso y más trabajo para lo mismo.

use serde::{Deserialize, Serialize};
use zbus::zvariant::Type;

// ── Direcciones del bus ───────────────────────────────────────────────────────

/// El servicio corre en el bus de **sesión**, como el usuario con la sesión
/// abierta: el tiempo de pantalla es de quien está sentado en la máquina, y
/// nada de esto sale de ahí.
pub const SERVICE_NAME: &str = "ar.net.vasak.os.Health";
pub const SERVICE_PATH: &str = "/ar/net/vasak/os/Health";
pub const SERVICE_INTERFACE: &str = "ar.net.vasak.os.Health";

/// Cuántas cubetas tiene el desglose por hora: una por cada hora del día local.
pub const HOURS_IN_DAY: usize = 24;

// ── Tiempo de pantalla ────────────────────────────────────────────────────────

/// Cuánto usó una aplicación en un día.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct AppUsage {
    /// El `app-id` de la ventana, como lo da el compositor.
    pub app_id: String,
    /// La categoría freedesktop principal (`Network`, `Development`, `Game`…),
    /// o vacío si no se pudo averiguar. Sale del `.desktop` de la aplicación.
    pub category: String,
    /// Milisegundos de uso ese día.
    pub millis: u64,
    /// Milisegundos por hora del día local: siempre [`HOURS_IN_DAY`] valores,
    /// del 0 al 23. Para datos traídos de la versión vieja —que sólo guardaba el
    /// total— pueden venir todos en cero aunque `millis` no lo esté: ahí no hay
    /// desglose por hora, sólo el total.
    pub hours: Vec<u64>,
}

/// Un día con el uso de cada aplicación.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct DayUsage {
    /// La fecha local, `AAAA-MM-DD`.
    pub date: String,
    /// Las aplicaciones con algo de uso ese día.
    pub apps: Vec<AppUsage>,
}

/// Lo que devuelve una consulta de tiempo de pantalla.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct ScreenTimeReport {
    /// Si el registro está prendido. Apagado, lo guardado se sigue pudiendo
    /// consultar, pero no se cuenta nada nuevo.
    pub enabled: bool,
    /// Hoy, en la zona horaria de la sesión: quien consulta no adivina el día.
    pub today: String,
    /// El primer día con algo guardado, o vacío si no hay historia. Antes de
    /// ése no hay nada que promediar.
    pub first_day: String,
    /// Sólo los días con algo, en orden. Los días vacíos no viajan.
    pub days: Vec<DayUsage>,
}

// ── El proxy, para quien esté en zbus 5 ───────────────────────────────────────

/// El lado cliente del servicio.
///
/// Lo usa quien esté en la misma versión de zbus que el servicio. El escritorio
/// está clavado en zbus 4 por otros motivos (el menú de la bandeja), así que
/// replica estas firmas a mano, como ya hace con `vasak-connect`; cuando suba a
/// zbus 5 podrá usar este proxy directamente.
#[zbus::proxy(
    interface = "ar.net.vasak.os.Health",
    default_service = "ar.net.vasak.os.Health",
    default_path = "/ar/net/vasak/os/Health",
    gen_blocking = false
)]
pub trait Health {
    /// Una muestra: a esta vuelta, ésta es la aplicación enfocada (vacío si
    /// ninguna) y así está el bloqueo. El escritorio la empuja cada par de
    /// segundos.
    fn push_sample(&self, app_id: &str, locked: bool) -> zbus::Result<()>;

    /// Empezó la inactividad; la persona dejó de tocar hace `idle_ms`
    /// milisegundos (el aviso del compositor llega después del umbral). Eso se
    /// descuenta.
    fn idle_started(&self, idle_ms: u64) -> zbus::Result<()>;

    /// Volvió la actividad.
    fn idle_ended(&self) -> zbus::Result<()>;

    /// Se perdió la detección de inactividad (el compositor no la ofrece, o se
    /// cayó la conexión): se deja de contar para no sumar tiempo sin nadie.
    fn idle_monitor_lost(&self) -> zbus::Result<()>;

    /// Guarda todo lo contado sin esperar la ventana de descuento: antes de
    /// cerrar la sesión o apagar.
    fn flush(&self) -> zbus::Result<()>;

    /// Los días de `from` a `to` (`AAAA-MM-DD`, los dos incluidos).
    fn screen_time(&self, from: &str, to: &str) -> zbus::Result<ScreenTimeReport>;

    /// Borra todo el historial.
    fn clear_screen_time(&self) -> zbus::Result<()>;

    /// Prende o apaga el registro en el acto. Quien lo llame también debería
    /// persistir `screen_time.enabled` en `vasak.conf`, que es de donde el
    /// servicio lo lee al arrancar.
    fn set_enabled(&self, enabled: bool) -> zbus::Result<()>;

    /// Si el registro está prendido.
    fn enabled(&self) -> zbus::Result<bool>;
}

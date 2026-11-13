//! El servicio de salud de VasakOS.
//!
//! Hoy mide una sola cosa: el tiempo de pantalla por aplicación —cuánto estuvo
//! enfocada cada una, por día y por hora, con su categoría—. Mañana puede medir
//! más cosas de bienestar; por eso es un servicio y no una parte del escritorio,
//! y por eso el nombre es «salud» y no «tiempo de pantalla».
//!
//! No dibuja nada. El escritorio y Configuración —que ya existen— son los que
//! muestran los informes; acá no hay ninguna ventana que pintar. Por eso es un
//! demonio de Rust y no una aplicación de Tauri: un runtime de WebKit residente
//! para un servicio sin ventana es el error ya documentado para otros demonios
//! del sistema.
//!
//! No mira el compositor ni la inactividad: eso lo hace el escritorio, que ya
//! tiene esas conexiones, y empuja las muestras por D-Bus. Acá queda la
//! contabilidad, el almacén y las categorías.
//!
//! Corre en el bus de sesión. Todo lo que toca —el tiempo de pantalla, su
//! archivo por día— es de quien tiene la sesión abierta.

mod category;
mod config;
mod recorder;
mod reminders;
mod service;
mod store;
mod tracker;

use std::sync::Arc;
use std::time::Duration;

use chrono::Timelike;
use tokio::signal::unix::{signal, SignalKind};
use tokio::time::interval;
use tracing::{debug, error, info, warn};
use vasak_gamification_protocol::GamificationProxy;
use zbus::connection;

use recorder::{Recorder, SAVE_EVERY};
use service::HealthService;
use store::Store;
use vasak_health_protocol::{SERVICE_NAME, SERVICE_PATH};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // El nombre del crate lo pone el compilador: el binario es
    // `vasak-health-service` pero el paquete es `vasak-health-daemon`, y escribir
    // cualquiera a mano da un filtro que no engancha ningún objetivo y un
    // servicio que no registra nada.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("{}=info", env!("CARGO_CRATE_NAME")).into()),
        )
        .init();

    let store = Store::default_dir().map(Store::new);
    match &store {
        Some(store) => import_legacy_if_empty(store),
        None => error!("sin $XDG_DATA_HOME ni carpeta personal: no se guarda nada"),
    }

    let recorder = Arc::new(Recorder::new(store, config::read_enabled()));

    // La gamificación (vasak-health-service#5) ya no vive acá: su motor es ahora
    // el servicio `vasak-gamification`. health sólo le emite eventos. El
    // interruptor de `vasak.conf` se mantiene para no despertar al servicio por
    // D-Bus cuando la persona tiene la capa apagada.
    let gamification_on = config::read_gamification_enabled();

    let connection = connection::Builder::session()?
        .name(SERVICE_NAME)?
        .serve_at(
            SERVICE_PATH,
            HealthService {
                recorder: recorder.clone(),
            },
        )?
        .build()
        .await?;
    info!("servicio en {SERVICE_NAME}");

    // El proxy al servicio de gamificación, sobre la misma conexión de sesión.
    // Sólo si la capa está prendida: apagada, no se construye y no se emite nada.
    // Construirlo no exige que el servicio esté corriendo —es perezoso—; si aun
    // así fallara, se degrada a `None` y los recordatorios siguen sin gamificar.
    let gamification = if gamification_on {
        match GamificationProxy::new(&connection).await {
            Ok(proxy) => Some(proxy),
            Err(error) => {
                debug!("no se pudo preparar la gamificación: {error}");
                None
            }
        }
    } else {
        None
    };

    // Guardar cada tanto es una red de seguridad además de lo que dispara cada
    // muestra: si el escritorio deja de empujar muestras, lo último contado se
    // guarda igual.
    let mut saver = interval(SAVE_EVERY);

    // Los recordatorios de pausa e hidratación (vasak-health-service#4): cada
    // minuto se suma al acumulado de cada tipo si la persona está activa, y al
    // llegar al intervalo configurado se avisa, siempre que la hora caiga dentro
    // de la franja horaria. La configuración se relee en cada vuelta, que es
    // barato una vez por minuto y evita un método de D-Bus nuevo sólo para
    // enterarse del cambio.
    const REMINDER_STEP: Duration = Duration::from_secs(60);
    let mut reminder_tick = interval(REMINDER_STEP);
    // La primera vuelta de un `interval` de tokio es inmediata: se descarta para
    // no sumar un minuto que no pasó.
    reminder_tick.tick().await;
    let mut breaks = reminders::BreakReminder::default();
    let mut hydration = reminders::BreakReminder::default();

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;

    loop {
        tokio::select! {
            _ = saver.tick() => recorder.save_finalized(),
            _ = reminder_tick.tick() => {
                // La hora local y la franja gobiernan los dos tipos por igual. Se
                // consulta `due()` siempre (acumula y reinicia el contador aunque
                // estemos fuera de la franja) y recién después se filtra por la
                // franja: un aviso que cae fuera de hora se descarta, no se corre.
                let hour = chrono::Local::now().hour();
                let window = reminders::read_reminder_window();
                let in_window = reminders::within_window(hour, &window);
                let active = recorder.is_active();

                let break_config = reminders::read_break_reminders();
                let break_fires = breaks.due(&break_config, active, REMINDER_STEP) && in_window;
                if break_fires {
                    reminders::notify_break(&connection).await;
                }

                let hydration_config = reminders::read_hydration_reminders();
                let hydration_fires =
                    hydration.due(&hydration_config, active, REMINDER_STEP) && in_window;
                if hydration_fires {
                    reminders::notify_hydration(&connection).await;
                }

                for kind in reminder_events(gamification_on, break_fires, hydration_fires) {
                    emit_gamification(gamification.as_ref(), kind).await;
                }
            }
            _ = sigterm.recv() => break,
            _ = sigint.recv() => break,
        }
    }

    // Al salir —systemd lo para, o se cierra la sesión— se guarda todo.
    info!("guardando antes de salir");
    recorder.save_everything();
    Ok(())
}

/// Si el almacén está vacío, trae una sola vez lo que haya en la carpeta de la
/// versión vieja (el escritorio): así no se pierde la historia al mudarse.
fn import_legacy_if_empty(store: &Store) {
    if !store.is_empty() {
        return;
    }
    let Some(legacy) = Store::legacy_dir().map(Store::new) else {
        return;
    };
    if legacy.is_empty() {
        return;
    }
    match store.import_from(&legacy) {
        Ok(()) => info!("historial de tiempo de pantalla importado desde el escritorio"),
        Err(error) => warn!("no se pudo importar el historial viejo: {error}"),
    }
}

/// Los `kind`s de gamificación que se emiten en una vuelta, en orden. Puro:
/// separa *qué* se emite (probable sin bus) de la emisión en sí ([`emit_gamification`],
/// fire-and-forget), igual que `reminders::break_notification` separa el texto
/// del envío. Apagada la capa, no se emite nada: así no se despierta al servicio
/// de gamificación por D-Bus.
///
/// Limitación honesta: el evento es «el recordatorio avisó estando la persona
/// activa», no «la persona confirmó que tomó agua / hizo la pausa» —confirmar
/// necesitaría feedback de la UI, que hoy no existe—. El servicio decide los
/// puntos de cada `kind`; el emisor sólo manda el `kind`.
fn reminder_events(enabled: bool, break_fires: bool, hydration_fires: bool) -> Vec<&'static str> {
    if !enabled {
        return Vec::new();
    }
    let mut kinds = Vec::new();
    if break_fires {
        kinds.push("break");
    }
    if hydration_fires {
        kinds.push("hydration");
    }
    kinds
}

/// Emite un evento al servicio de gamificación, fire-and-forget. Sin proxy
/// —capa apagada, o el servicio no está instalado o está arrancando— no hace
/// nada: un evento que no se pudo emitir no voltea ni frena el recordatorio.
async fn emit_gamification(proxy: Option<&GamificationProxy<'_>>, kind: &str) {
    if let Some(proxy) = proxy {
        if let Err(error) = proxy.emit(kind).await {
            debug!("no se pudo emitir el evento de gamificación «{kind}»: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_recordatorio_de_pausa_emite_break() {
        assert_eq!(reminder_events(true, true, false), ["break"]);
    }

    #[test]
    fn un_recordatorio_de_hidratación_emite_hydration() {
        assert_eq!(reminder_events(true, false, true), ["hydration"]);
    }

    #[test]
    fn si_avisan_los_dos_emite_break_antes_que_hydration() {
        assert_eq!(reminder_events(true, true, true), ["break", "hydration"]);
    }

    #[test]
    fn sin_avisos_no_emite_nada() {
        assert!(reminder_events(true, false, false).is_empty());
    }

    #[test]
    fn con_la_capa_apagada_no_emite_nada_aunque_avisen() {
        assert!(reminder_events(false, true, true).is_empty());
        assert!(reminder_events(false, true, false).is_empty());
        assert!(reminder_events(false, false, true).is_empty());
    }

    #[tokio::test]
    async fn sin_el_servicio_emitir_no_entra_en_pánico() {
        emit_gamification(None, "break").await;
        emit_gamification(None, "hydration").await;
    }
}

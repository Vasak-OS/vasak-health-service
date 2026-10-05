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
mod service;
mod store;
mod tracker;

use std::sync::Arc;

use tokio::signal::unix::{signal, SignalKind};
use tokio::time::interval;
use tracing::{error, info, warn};
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

    let _connection = connection::Builder::session()?
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

    // Guardar cada tanto es una red de seguridad además de lo que dispara cada
    // muestra: si el escritorio deja de empujar muestras, lo último contado se
    // guarda igual.
    let mut saver = interval(SAVE_EVERY);
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;

    loop {
        tokio::select! {
            _ = saver.tick() => recorder.save_finalized(),
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

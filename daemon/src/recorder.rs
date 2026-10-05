//! El registro: junta la contabilidad, el almacén y las categorías.
//!
//! Es el equivalente de lo que antes vivía en el escritorio, menos todo lo que
//! miraba el compositor y la inactividad: eso ahora llega por D-Bus, en muestras
//! que empuja el escritorio ([`Recorder::on_sample`] y las de inactividad). Acá
//! queda lo que no depende de Wayland: contar, guardar cada tanto lo que ya no
//! puede descontarse, responder las consultas y prenderse o apagarse.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration as StdDuration, Instant};

use chrono::{Duration as ChronoDuration, Local, NaiveDate, Utc};
use parking_lot::Mutex;

use vasak_health_protocol::{AppUsage, DayUsage, ScreenTimeReport};

use crate::category::Categories;
use crate::store::Store;
use crate::tracker::{merge, Tracker, Usage};

/// La ventana de descuento: tanto del final de lo contado queda pendiente hasta
/// que ya no pueda caer dentro de una inactividad. Tiene que ser por lo menos el
/// umbral de inactividad que usa el escritorio (tres minutos), o se guardaría
/// tiempo que un aviso de inactividad todavía podría descontar.
const WINDOW: ChronoDuration = ChronoDuration::minutes(3);

/// Más que esto entre dos muestras es un salto del reloj (una suspensión sin
/// bloqueo, el proceso detenido): lo del medio no cuenta.
const MAX_GAP: ChronoDuration = ChronoDuration::seconds(30);

/// Cada cuánto se guarda lo que ya no puede descontarse.
pub const SAVE_EVERY: StdDuration = StdDuration::from_secs(60);

/// El rango que se pide no puede ser más largo que esto: el tablero pide un mes
/// y una semana, y un pedido de años sería leer miles de archivos.
const MAX_RANGE_DAYS: i64 = 62;

pub struct Recorder {
    tracker: Mutex<Tracker<Local>>,
    store: Option<Store>,
    /// Lo entregado por el tracker que todavía no llegó al disco (si el disco
    /// falló, se reintenta en la próxima vuelta).
    unsaved: Mutex<Usage>,
    enabled: AtomicBool,
    /// Lo toman de punta a punta guardar y borrar: entre que el tracker entrega
    /// lo contado y eso llega al disco, un «Borrar historial» no puede meterse
    /// en el medio.
    io: Mutex<()>,
    categories: Categories,
    last_save: Mutex<Instant>,
}

impl Recorder {
    pub fn new(store: Option<Store>, enabled: bool) -> Self {
        Self {
            tracker: Mutex::new(Tracker::new(Local, WINDOW, MAX_GAP)),
            store,
            unsaved: Mutex::new(Usage::new()),
            enabled: AtomicBool::new(enabled),
            io: Mutex::new(()),
            categories: Categories::new(),
            last_save: Mutex::new(Instant::now()),
        }
    }

    /// Una muestra del escritorio: ésta es la aplicación enfocada (o `None`) y
    /// así está el bloqueo. Apagado, no cuenta nada.
    pub fn on_sample(&self, app: Option<&str>, locked: bool) {
        if self.enabled.load(Ordering::SeqCst) {
            self.tracker.lock().tick(Utc::now(), app, locked);
        }
        self.maybe_save();
    }

    /// Empezó la inactividad; la persona dejó de tocar hace `idle_ms`.
    pub fn on_idle_started(&self, idle_ms: u64) {
        let now = Utc::now();
        let since = now - ChronoDuration::milliseconds(idle_ms as i64);
        self.tracker.lock().idle_started(now, since);
    }

    /// Volvió la actividad.
    pub fn on_idle_ended(&self) {
        self.tracker.lock().idle_ended();
    }

    /// Se perdió la detección de inactividad: se cierra lo abierto y el tracker
    /// queda inactivo (hasta que el escritorio avise que volvió).
    pub fn on_idle_monitor_lost(&self) {
        tracing::warn!(
            "sin detección de inactividad: se deja de contar para no sumar tiempo sin nadie adelante"
        );
        let now = Utc::now();
        self.tracker.lock().idle_started(now, now);
    }

    /// Guarda lo que ya no puede descontarse, pero sólo si pasó [`SAVE_EVERY`]
    /// desde la última vez.
    fn maybe_save(&self) {
        let due = {
            let mut last = self.last_save.lock();
            if last.elapsed() >= SAVE_EVERY {
                *last = Instant::now();
                true
            } else {
                false
            }
        };
        if due {
            self.save_finalized();
        }
    }

    /// Guarda lo que ya no puede descontarse. Lo llama el temporizador del
    /// servicio además de [`Recorder::maybe_save`].
    pub fn save_finalized(&self) {
        let _io = self.io.lock();
        let done = self.tracker.lock().finalize(Utc::now());
        self.save(done);
    }

    /// Guarda todo lo contado, sin esperar la ventana de descuento: antes de
    /// cerrar la sesión, apagar o apagar el registro.
    pub fn save_everything(&self) {
        let _io = self.io.lock();
        let done = self.tracker.lock().finalize_all(Utc::now());
        self.save(done);
    }

    /// Con `io` tomado.
    fn save(&self, done: Usage) {
        let mut unsaved = self.unsaved.lock();
        merge(&mut unsaved, &done);
        if let Some(store) = &self.store {
            save_pending(store, &mut unsaved);
        }
    }

    /// Lo guardado más lo que todavía no llegó al disco más lo vivo, de `from` a
    /// `to`.
    fn usage(&self, from: NaiveDate, to: NaiveDate) -> Usage {
        let mut usage = self
            .store
            .as_ref()
            .map(|store| store.range(from, to))
            .unwrap_or_default();
        merge(&mut usage, &self.unsaved.lock());
        merge(&mut usage, &self.tracker.lock().live(Utc::now()));
        usage.retain(|day, apps| *day >= from && *day <= to && !apps.is_empty());
        usage
    }

    /// Borra todo el historial: lo guardado y lo que todavía no se guardó.
    pub fn clear(&self) -> Result<(), String> {
        let _io = self.io.lock();
        self.tracker.lock().discard();
        self.unsaved.lock().clear();
        match &self.store {
            Some(store) => store.clear().map_err(|error| error.to_string()),
            None => Ok(()),
        }
    }

    /// Prende o apaga el registro. Al apagarlo se guarda lo que había; apagado
    /// no cuenta nada.
    pub fn set_enabled(&self, enabled: bool) {
        let was = self.enabled.swap(enabled, Ordering::SeqCst);
        if was && !enabled {
            tracing::info!("apagado");
            self.save_everything();
        } else if !was && enabled {
            tracing::info!("prendido");
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// El informe de `from` a `to` para quien lo consulte: los días con algo, el
    /// uso de cada aplicación con su categoría y su desglose por hora.
    pub fn report(&self, from: NaiveDate, to: NaiveDate) -> ScreenTimeReport {
        let usage = self.usage(from, to);
        let days = usage
            .into_iter()
            .map(|(date, apps)| DayUsage {
                date: date.to_string(),
                apps: apps
                    .into_iter()
                    .map(|(app_id, time)| AppUsage {
                        category: self.categories.of(&app_id),
                        app_id,
                        millis: time.total,
                        hours: time.hours.to_vec(),
                    })
                    .collect(),
            })
            .collect();
        ScreenTimeReport {
            enabled: self.is_enabled(),
            today: Local::now().date_naive().to_string(),
            first_day: self
                .store
                .as_ref()
                .and_then(Store::first_day)
                .map(|day| day.to_string())
                .unwrap_or_default(),
            days,
        }
    }
}

/// Valida un rango antes de leer nada.
pub fn parse_range(from: &str, to: &str) -> Result<(NaiveDate, NaiveDate), String> {
    let parse = |text: &str| {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| format!("fecha inválida: «{text}»"))
    };
    let (from, to) = (parse(from)?, parse(to)?);
    if from > to {
        return Err("el rango termina antes de empezar".into());
    }
    if (to - from).num_days() > MAX_RANGE_DAYS {
        return Err(format!("el rango no puede pasar de {MAX_RANGE_DAYS} días"));
    }
    Ok((from, to))
}

/// Guarda lo pendiente **un día por vez**, y saca de `unsaved` cada día recién
/// cuando llegó al disco. Si un día falla se corta ahí: los anteriores ya no
/// están en `unsaved`, así que el reintento no los vuelve a sumar.
fn save_pending(store: &Store, unsaved: &mut Usage) {
    for day in unsaved.keys().copied().collect::<Vec<_>>() {
        let Some(apps) = unsaved.get(&day).cloned() else {
            continue;
        };
        let mut one = Usage::new();
        one.insert(day, apps);
        match store.add(&one) {
            Ok(()) => {
                unsaved.remove(&day);
            }
            Err(error) => {
                tracing::error!("no se pudo guardar {day} (se reintenta): {error}");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::TempDir;
    use crate::tracker::AppTime;

    fn total(usage: &Usage, app: &str) -> u64 {
        usage
            .values()
            .filter_map(|apps| apps.get(app))
            .map(|time| time.total)
            .sum()
    }

    #[test]
    fn el_rango_se_valida_antes_de_leer_nada() {
        assert!(parse_range("2026-03-16", "2026-03-22").is_ok());
        assert!(parse_range("2026-03-16", "2026-03-16").is_ok());
        assert!(parse_range("2026-03-22", "2026-03-16").is_err());
        assert!(parse_range("ayer", "2026-03-16").is_err());
        assert!(parse_range("2026-01-01", "2026-12-31").is_err());
        // Un mes con su semana de antes entra.
        assert!(parse_range("2026-02-23", "2026-03-31").is_ok());
    }

    #[test]
    fn apagar_guarda_lo_que_había_y_deja_de_contar() {
        let temp = TempDir::new("apagar");
        let recorder = Recorder::new(Some(Store::new(temp.0.clone())), true);
        let start = Utc::now() - ChronoDuration::minutes(10);
        let mut now = start;
        while now <= Utc::now() {
            recorder.tracker.lock().tick(now, Some("firefox"), false);
            now += ChronoDuration::seconds(2);
        }
        recorder.set_enabled(false);
        assert!(!recorder.is_enabled());

        // Lo contado antes de apagar quedó guardado.
        let today = Local::now().date_naive();
        let yesterday = today.pred_opt().expect("ayer");
        assert!(total(&recorder.usage(yesterday, today), "firefox") > 0);

        // Y apagado, una muestra nueva no suma nada.
        let before = total(&recorder.usage(yesterday, today), "firefox");
        recorder.on_sample(Some("kitty"), false);
        assert_eq!(total(&recorder.usage(yesterday, today), "kitty"), 0);
        assert_eq!(total(&recorder.usage(yesterday, today), "firefox"), before);
    }

    #[test]
    fn si_falla_un_día_los_que_ya_se_guardaron_no_se_vuelven_a_sumar() {
        let temp = TempDir::new("por-dia");
        let store = Store::new(temp.0.clone());
        let day = |n| NaiveDate::from_ymd_opt(2026, 3, n).expect("fecha");
        // El 17 no se puede escribir: en su lugar hay un directorio.
        std::fs::create_dir_all(temp.0.join("2026-03-17.json")).expect("se crea");
        let one = |ms: u64| {
            let mut time = AppTime {
                total: ms,
                ..Default::default()
            };
            time.hours[0] = ms;
            std::collections::BTreeMap::from([("a".to_string(), time)])
        };
        let mut unsaved = Usage::new();
        unsaved.insert(day(16), one(10));
        unsaved.insert(day(17), one(20));

        save_pending(&store, &mut unsaved);
        assert_eq!(unsaved.keys().copied().collect::<Vec<_>>(), vec![day(17)]);
        save_pending(&store, &mut unsaved);
        // El 16 quedó en 10 y no en 20.
        assert_eq!(store.load(day(16))["a"].total, 10);

        std::fs::remove_dir_all(temp.0.join("2026-03-17.json")).expect("se borra");
        save_pending(&store, &mut unsaved);
        assert!(unsaved.is_empty());
        assert_eq!(store.load(day(17))["a"].total, 20);
    }

    /// El recorrido entero: lo que entrega el tracker llega al disco y vuelve en
    /// el informe, sin contarse dos veces; después se borra.
    #[test]
    fn lo_guardado_y_lo_vivo_se_suman_sin_repetir() {
        let temp = TempDir::new("recorder");
        let recorder = Recorder::new(Some(Store::new(temp.0.clone())), true);
        let start = Utc::now() - ChronoDuration::minutes(10);
        let mut now = start;
        while now <= Utc::now() {
            recorder.tracker.lock().tick(now, Some("firefox"), false);
            now += ChronoDuration::seconds(2);
        }
        recorder.save_finalized();
        let today = Local::now().date_naive();
        let yesterday = today.pred_opt().expect("ayer");

        let got = total(&recorder.usage(yesterday, today), "firefox");
        let expected = (Utc::now() - start).num_milliseconds() as u64;
        // Lo que tardó la prueba en correr es la diferencia; diez minutos con
        // una tolerancia de cinco segundos.
        assert!(got.abs_diff(expected) < 5_000, "{got} vs {expected}");
        assert!(std::fs::read_dir(&temp.0).expect("se lista").count() >= 1);

        recorder.clear().expect("se borra");
        assert!(recorder.usage(yesterday, today).is_empty());
        assert_eq!(std::fs::read_dir(&temp.0).expect("se lista").count(), 0);
    }
}

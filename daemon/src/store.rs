//! Dónde se guarda el tiempo de pantalla: un archivo por día.
//!
//! `$XDG_DATA_HOME/vasak-health-service/screen-time/AAAA-MM-DD.json`, con los
//! milisegundos de cada `app-id` ese día y su desglose por hora:
//!
//! ```json
//! {
//!   "version": 2,
//!   "date": "2026-03-16",
//!   "apps": { "firefox": { "total": 6540000, "hours": [ … 24 … ] } }
//! }
//! ```
//!
//! Un archivo por día y no una base: lo que se lee es siempre una semana y un
//! mes, son unos pocos kilobytes cada uno, y borrar un día —o todo— es borrar
//! archivos. No sale de la máquina: nadie más lo lee.
//!
//! # Compatibilidad con lo viejo
//!
//! La versión que vivía en el escritorio guardaba sólo el total
//! (`"firefox": 6540000`). Esos archivos se siguen leyendo —el total entra y las
//! horas quedan en cero— y se importan una sola vez desde la carpeta vieja
//! ([`Store::import_from`]), para no perder la historia al mudarse al servicio.
//!
//! # Escritura atómica
//!
//! Se escribe en un temporal **del mismo directorio**, se fuerza al disco y se
//! renombra encima del de verdad. `rename` dentro de un mismo sistema de
//! archivos es atómico: quien lea ve el archivo viejo entero o el nuevo entero,
//! nunca uno a medias, y un corte de luz en el medio deja el viejo. Un temporal
//! en `/tmp` no serviría: suele ser otro sistema de archivos, y ahí `rename`
//! falla o copia.
//!
//! # Un archivo roto no se pisa
//!
//! Si un día no se puede leer —editado a mano, de otra versión—, se aparta como
//! `.json.roto` en lugar de sobrescribirlo con lo de esta sesión: lo que había
//! se puede recuperar a mano, y el informe sigue funcionando.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use vasak_health_protocol::HOURS_IN_DAY;

use crate::tracker::{AppTime, DayApps, Usage};

const VERSION: u32 = 2;

/// Cómo se guarda una aplicación en el archivo. `untagged`: un objeto es la
/// versión 2 (con horas) y un número suelto es la 1 (sólo el total).
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
enum StoredApp {
    Detailed { total: u64, hours: Vec<u64> },
    Total(u64),
}

impl StoredApp {
    fn from(time: &AppTime) -> Self {
        StoredApp::Detailed {
            total: time.total,
            hours: time.hours.to_vec(),
        }
    }

    fn into_app_time(self) -> AppTime {
        match self {
            StoredApp::Detailed { total, hours } => {
                let mut buckets = [0_u64; HOURS_IN_DAY];
                for (slot, value) in buckets.iter_mut().zip(hours) {
                    *slot = value;
                }
                AppTime {
                    total,
                    hours: buckets,
                }
            }
            StoredApp::Total(total) => AppTime {
                total,
                hours: [0; HOURS_IN_DAY],
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct DayFile {
    version: u32,
    date: NaiveDate,
    apps: BTreeMap<String, StoredApp>,
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// `$XDG_DATA_HOME/vasak-health-service/screen-time`. `dirs` ya ignora un
    /// `XDG_DATA_HOME` relativo, como pide la especificación.
    pub fn default_dir() -> Option<PathBuf> {
        dirs::data_dir().map(|data| data.join("vasak-health-service").join("screen-time"))
    }

    /// La carpeta de la versión vieja, en el escritorio: de ahí se importa una
    /// sola vez.
    pub fn legacy_dir() -> Option<PathBuf> {
        dirs::data_dir().map(|data| data.join("vasak-desktop").join("screen-time"))
    }

    #[cfg(test)]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path_of(&self, day: NaiveDate) -> PathBuf {
        self.dir.join(format!("{}.json", day.format("%Y-%m-%d")))
    }

    /// Lo guardado de un día. Sin archivo, vacío.
    pub fn load(&self, day: NaiveDate) -> DayApps {
        let path = self.path_of(day);
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(_) => return DayApps::new(),
        };
        match serde_json::from_str::<DayFile>(&content) {
            Ok(file) if file.date == day => file
                .apps
                .into_iter()
                .map(|(app, stored)| (app, stored.into_app_time()))
                .collect(),
            _ => {
                set_aside(&path);
                DayApps::new()
            }
        }
    }

    /// Reemplaza lo guardado de un día, de forma atómica.
    pub fn save(&self, day: NaiveDate, apps: &DayApps) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let file = DayFile {
            version: VERSION,
            date: day,
            apps: apps
                .iter()
                .map(|(app, time)| (app.clone(), StoredApp::from(time)))
                .collect(),
        };
        let body = serde_json::to_vec_pretty(&file)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        write_atomically(&self.path_of(day), &body)
    }

    /// Suma lo de `usage` a lo guardado de cada día.
    pub fn add(&self, usage: &Usage) -> io::Result<()> {
        for (day, apps) in usage {
            if apps.is_empty() {
                continue;
            }
            let mut saved = self.load(*day);
            for (app, time) in apps {
                saved.entry(app.clone()).or_default().add(time);
            }
            self.save(*day, &saved)?;
        }
        Ok(())
    }

    /// Lo guardado de `from` a `to`, los dos incluidos. Los días sin nada no
    /// aparecen.
    pub fn range(&self, from: NaiveDate, to: NaiveDate) -> Usage {
        let mut out = Usage::new();
        let mut day = from;
        while day <= to {
            let apps = self.load(day);
            if !apps.is_empty() {
                out.insert(day, apps);
            }
            match day.succ_opt() {
                Some(next) => day = next,
                None => break,
            }
        }
        out
    }

    /// El primer día con algo guardado, si hay alguno.
    pub fn first_day(&self) -> Option<NaiveDate> {
        self.days().into_iter().min()
    }

    /// Trae a este almacén todo lo que haya en `legacy`, sumándolo. Se usa una
    /// sola vez, cuando este almacén todavía está vacío: ver [`Store::is_empty`].
    pub fn import_from(&self, legacy: &Store) -> io::Result<()> {
        for day in legacy.days() {
            let apps = legacy.load(day);
            if apps.is_empty() {
                continue;
            }
            let mut usage = Usage::new();
            usage.insert(day, apps);
            self.add(&usage)?;
        }
        Ok(())
    }

    /// Si no hay ningún día guardado todavía.
    pub fn is_empty(&self) -> bool {
        self.days().is_empty()
    }

    /// Borra todo el historial: los archivos de días, y nada más.
    pub fn clear(&self) -> io::Result<()> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let is_day = name.split_once('.').is_some_and(|(stem, rest)| {
                NaiveDate::parse_from_str(stem, "%Y-%m-%d").is_ok()
                    && matches!(rest, "json" | "json.roto")
            });
            let is_leftover_temp = name.starts_with('.') && name.contains(".json.tmp-");
            if is_day || is_leftover_temp {
                fs::remove_file(&path)?;
            }
        }
        Ok(())
    }

    fn days(&self) -> Vec<NaiveDate> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name();
                let stem = name.to_str()?.strip_suffix(".json")?.to_string();
                NaiveDate::parse_from_str(&stem, "%Y-%m-%d").ok()
            })
            .collect()
    }
}

/// Escribe `body` en `path` sin dejar nunca un archivo a medias: ver la
/// documentación del módulo.
pub fn write_atomically(path: &Path, body: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "sin directorio"))?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "sin nombre"))?
        .to_string_lossy()
        .into_owned();
    let temp = dir.join(format!(".{name}.tmp-{}", std::process::id()));

    let result = (|| {
        let mut file = fs::File::create(&temp)?;
        file.write_all(body)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        // El renombre queda en el directorio: sin esto, un corte justo después
        // puede volver al archivo viejo.
        if let Ok(directory) = fs::File::open(dir) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Aparta un archivo que no se pudo leer, para no pisarlo.
fn set_aside(path: &Path) {
    let mut aside = path.as_os_str().to_owned();
    aside.push(".roto");
    if fs::rename(path, &aside).is_ok() {
        tracing::warn!(
            "{} no se pudo leer; quedó como {}",
            path.display(),
            PathBuf::from(aside).display()
        );
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Un directorio propio en el temporal del sistema: **nunca** el historial
    /// de la persona. Se borra al soltarlo.
    pub struct TempDir(pub PathBuf);

    impl TempDir {
        pub fn new(label: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "vasak-health-{label}-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            fs::create_dir_all(&dir).expect("se crea el directorio temporal");
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn day(n: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 3, n).expect("fecha válida")
    }

    /// Una aplicación con todo su tiempo en una hora, para comprobar que las
    /// horas van y vuelven del disco.
    fn at_hour(hour: usize, ms: u64) -> AppTime {
        let mut time = AppTime {
            total: ms,
            ..Default::default()
        };
        time.hours[hour] = ms;
        time
    }

    fn apps(pairs: &[(&str, AppTime)]) -> DayApps {
        pairs
            .iter()
            .map(|(app, time)| (app.to_string(), time.clone()))
            .collect()
    }

    #[test]
    fn las_pruebas_no_tocan_el_historial_de_verdad() {
        let temp = TempDir::new("guardia");
        let store = Store::new(temp.0.clone());

        assert!(store.dir().starts_with(std::env::temp_dir()));
        if let Some(real) = Store::default_dir() {
            assert!(!store.dir().starts_with(&real));
        }
    }

    #[test]
    fn se_guarda_y_se_lee_un_día_con_sus_horas() {
        let temp = TempDir::new("ida-y-vuelta");
        let store = Store::new(temp.0.clone());

        let firefox = at_hour(10, 6_540_000);
        store
            .save(day(16), &apps(&[("firefox", firefox.clone())]))
            .expect("se guarda");
        assert_eq!(store.load(day(16)), apps(&[("firefox", firefox)]));
        // El archivo dice qué es.
        let text = fs::read_to_string(temp.0.join("2026-03-16.json")).expect("existe");
        assert!(text.contains("\"version\": 2"));
        assert!(text.contains("\"date\": \"2026-03-16\""));
        assert!(text.contains("\"total\": 6540000"));
    }

    #[test]
    fn sumar_se_agrega_a_lo_que_ya_había_total_y_horas() {
        let temp = TempDir::new("sumar");
        let store = Store::new(temp.0.clone());
        store
            .save(day(16), &apps(&[("firefox", at_hour(9, 1000))]))
            .expect("se guarda");

        let mut usage = Usage::new();
        usage.insert(
            day(16),
            apps(&[("firefox", at_hour(9, 500)), ("kitty", at_hour(10, 200))]),
        );
        usage.insert(day(17), apps(&[("kitty", at_hour(8, 50))]));
        store.add(&usage).expect("se suma");

        let dia16 = store.load(day(16));
        assert_eq!(dia16["firefox"].total, 1500);
        assert_eq!(dia16["firefox"].hours[9], 1500);
        assert_eq!(dia16["kitty"].total, 200);
        assert_eq!(store.load(day(17))["kitty"].total, 50);
    }

    #[test]
    fn un_archivo_de_la_versión_vieja_se_lee_como_total_sin_horas() {
        let temp = TempDir::new("v1");
        let store = Store::new(temp.0.clone());
        // El formato viejo: el valor de cada aplicación es un número suelto.
        fs::write(
            temp.0.join("2026-03-16.json"),
            r#"{"version":1,"date":"2026-03-16","apps":{"firefox":6540000}}"#,
        )
        .expect("se escribe");

        let loaded = store.load(day(16));
        assert_eq!(loaded["firefox"].total, 6_540_000);
        assert_eq!(loaded["firefox"].hours, [0; HOURS_IN_DAY]);
    }

    #[test]
    fn importar_de_la_carpeta_vieja_trae_todo_una_vez() {
        let vieja = TempDir::new("vieja");
        let nueva = TempDir::new("nueva");
        let legacy = Store::new(vieja.0.clone());
        let store = Store::new(nueva.0.clone());

        fs::write(
            vieja.0.join("2026-03-16.json"),
            r#"{"version":1,"date":"2026-03-16","apps":{"firefox":1000}}"#,
        )
        .expect("se escribe");

        assert!(store.is_empty());
        store.import_from(&legacy).expect("se importa");
        assert!(!store.is_empty());
        assert_eq!(store.load(day(16))["firefox"].total, 1000);
    }

    #[test]
    fn la_escritura_reemplaza_el_archivo_entero_y_no_lo_reescribe_encima() {
        // Un enlace duro al archivo viejo: si la escritura truncara y
        // reescribiera el mismo archivo, el enlace vería el contenido nuevo (o
        // uno a medias). Con temporal + rename, el enlace se queda con el viejo
        // entero y el nombre pasa a otro archivo con el nuevo.
        let temp = TempDir::new("atomica");
        let store = Store::new(temp.0.clone());
        store
            .save(day(16), &apps(&[("firefox", at_hour(1, 1))]))
            .expect("se guarda");
        let path = temp.0.join("2026-03-16.json");
        let witness = temp.0.join("testigo.json");
        fs::hard_link(&path, &witness).expect("se enlaza");
        let before = fs::read_to_string(&witness).expect("se lee");

        store
            .save(
                day(16),
                &apps(&[("firefox", at_hour(1, 2)), ("kitty", at_hour(2, 3))]),
            )
            .expect("se guarda");

        assert_eq!(fs::read_to_string(&witness).expect("se lee"), before);
        assert_eq!(store.load(day(16))["firefox"].total, 2);
        assert_eq!(store.load(day(16))["kitty"].total, 3);
    }

    #[test]
    fn la_escritura_no_deja_temporales_y_si_falla_tampoco() {
        let temp = TempDir::new("temporales");
        let store = Store::new(temp.0.clone());
        store
            .save(day(16), &apps(&[("firefox", at_hour(1, 1))]))
            .expect("se guarda");

        let names: Vec<String> = fs::read_dir(&temp.0)
            .expect("se lista")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["2026-03-16.json".to_string()]);

        // Un destino que es un directorio: el rename falla, el viejo queda y
        // el temporal se borra.
        let blocked = temp.0.join("ocupado.json");
        fs::create_dir_all(blocked.join("adentro")).expect("se crea");
        assert!(write_atomically(&blocked, b"{}").is_err());
        let leftovers = fs::read_dir(&temp.0)
            .expect("se lista")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-"))
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn un_archivo_roto_se_aparta_y_no_se_pisa() {
        let temp = TempDir::new("roto");
        let store = Store::new(temp.0.clone());
        fs::write(temp.0.join("2026-03-16.json"), "{ esto no es json").expect("se escribe");

        assert!(store.load(day(16)).is_empty());
        assert_eq!(
            fs::read_to_string(temp.0.join("2026-03-16.json.roto")).expect("se apartó"),
            "{ esto no es json"
        );
        // Y un archivo de otro día con el nombre cambiado tampoco se cree.
        store
            .save(day(17), &apps(&[("a", at_hour(0, 1))]))
            .expect("se guarda");
        fs::rename(
            temp.0.join("2026-03-17.json"),
            temp.0.join("2026-03-18.json"),
        )
        .expect("se mueve");
        assert!(store.load(day(18)).is_empty());
    }

    #[test]
    fn el_rango_trae_sólo_los_días_con_algo_y_el_primero_es_el_más_viejo() {
        let temp = TempDir::new("rango");
        let store = Store::new(temp.0.clone());
        assert_eq!(store.first_day(), None);
        assert!(store.range(day(1), day(31)).is_empty());

        store
            .save(day(20), &apps(&[("a", at_hour(0, 1))]))
            .expect("se guarda");
        store
            .save(day(3), &apps(&[("b", at_hour(0, 2))]))
            .expect("se guarda");
        store.save(day(25), &apps(&[])).expect("se guarda");

        let range = store.range(day(1), day(22));
        assert_eq!(
            range.keys().copied().collect::<Vec<_>>(),
            vec![day(3), day(20)]
        );
        assert_eq!(store.first_day(), Some(day(3)));
    }

    #[test]
    fn borrar_el_historial_borra_los_días_y_nada_más() {
        let temp = TempDir::new("borrar");
        let store = Store::new(temp.0.clone());
        store
            .save(day(16), &apps(&[("a", at_hour(0, 1))]))
            .expect("se guarda");
        fs::write(temp.0.join("2026-03-15.json.roto"), "x").expect("se escribe");
        fs::write(temp.0.join(".2026-03-14.json.tmp-99"), "x").expect("se escribe");
        fs::write(temp.0.join("notas.txt"), "mío").expect("se escribe");

        store.clear().expect("se borra");

        let left: Vec<String> = fs::read_dir(&temp.0)
            .expect("se lista")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, vec!["notas.txt".to_string()]);
        // Sin directorio, borrar no falla.
        let missing = Store::new(temp.0.join("no-existe"));
        assert!(missing.clear().is_ok());
    }
}

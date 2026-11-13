//! Capa de gamificación opt-in (vasak-health-service#5).
//!
//! Apagada por omisión. Prendida, distintos módulos del servicio pueden emitir
//! un [`Event`] y la capa decide XP, logros y rachas; el primer —y por ahora
//! único— consumidor son los recordatorios de pausa e hidratación. El estado
//! vive en texto plano al lado del tiempo de pantalla (no es secreto; nada de
//! llavero).
//!
//! El motor es puro: [`award`] toma el día como parámetro y no toca disco ni
//! reloj, así se prueba sin esperar un cambio de fecha. El acople con el disco
//! ([`load`]/[`save`]) y con el bucle (en `main.rs`) es la única parte que mira
//! el sistema de archivos.
//!
//! Fuera de alcance de este incremento: exponer el progreso por D-Bus y la UI
//! en Configuración; que otros módulos además de los recordatorios emitan
//! eventos (la interfaz ya lo permite, pero sólo se cablean los recordatorios).

use std::collections::BTreeSet;
use std::io;
use std::path::Path;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use vasak_health_protocol::GamificationProgress;

use crate::config;
use crate::store::write_atomically;

/// Nombre del archivo de estado, dentro de `dirs::data_dir()/vasak-health-service`
/// (al lado de `screen-time/`). Texto plano.
const STATE_FILE: &str = "gamification.json";

/// Umbrales de XP que desbloquean un logro, con su id estable. Son un **default
/// abierto**: los números y la cantidad de hitos pueden cambiarse sin tocar la
/// lógica, y los ids se mantienen estables para no re-desbloquear lo ya ganado.
const XP_ACHIEVEMENTS: &[(u64, &str)] = &[(100, "xp-100"), (500, "xp-500"), (1000, "xp-1000")];

/// La configuración de la gamificación, leída de `vasak.conf`.
#[derive(Debug, Clone, PartialEq)]
pub struct GamificationConfig {
    pub enabled: bool,
}

/// Lee la sección `gamification` de `vasak.conf`: `enabled` (apagada si falta o
/// no es un booleano). Un archivo ilegible o sin la sección deja la capa
/// apagada, igual que los recordatorios.
pub fn gamification_from_json(content: &str) -> GamificationConfig {
    let enabled = serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .as_ref()
        .and_then(|cfg| cfg.get("gamification"))
        .and_then(|section| section.get("enabled"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    GamificationConfig { enabled }
}

/// Lee `vasak.conf` y devuelve la configuración de la gamificación; apagada si
/// el archivo no se puede leer.
pub fn read_gamification_config() -> GamificationConfig {
    config::config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|content| gamification_from_json(&content))
        .unwrap_or(GamificationConfig { enabled: false })
}

/// Lo que emite un módulo para que la capa lo puntúe. Genérico a propósito: la
/// capa no se ata a ningún módulo en particular. `kind` identifica de qué se
/// trata (p. ej. `"break"`, `"hydration"`) y `points` cuánto suma.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub kind: String,
    pub points: u32,
}

/// El progreso acumulado de la persona. Se guarda y se lee tal cual de disco.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    /// Experiencia total acumulada.
    pub xp: u64,
    /// Días seguidos con al menos un evento.
    pub streak_days: u32,
    /// Último día con actividad, para decidir si la racha sigue o se corta.
    pub last_active_day: Option<NaiveDate>,
    /// Ids de los logros ya desbloqueados, sin repetir.
    pub achievements: Vec<String>,
    /// `kind`s ya vistos, para el logro de «primer evento de cada tipo».
    pub seen_kinds: BTreeSet<String>,
}

impl Progress {
    /// Arma el informe que viaja por D-Bus: XP, racha y logros, con el
    /// interruptor `enabled` que decide quien consulta. Los campos de
    /// contabilidad interna (`last_active_day`, `seen_kinds`) no salen: a quien
    /// muestra el progreso no le sirven.
    pub fn report(&self, enabled: bool) -> GamificationProgress {
        GamificationProgress {
            enabled,
            xp: self.xp,
            streak_days: self.streak_days,
            achievements: self.achievements.clone(),
        }
    }
}

/// El corazón, puro: aplica `event` a `progress` con `today` como el día actual.
/// Suma la XP, actualiza la racha y desbloquea los logros que correspondan. No
/// toca disco ni reloj; el día entra por parámetro para poder probarla.
pub fn award(progress: &mut Progress, event: &Event, today: NaiveDate) {
    progress.xp += u64::from(event.points);
    update_streak(progress, today);

    // Logro de «primer evento de cada kind»: si el tipo entra por primera vez a
    // `seen_kinds`, se desbloquea su logro. `insert` devuelve `true` sólo la
    // primera vez, así no se repite.
    if progress.seen_kinds.insert(event.kind.clone()) {
        unlock(progress, format!("first-{}", event.kind));
    }

    // Logros por umbral de XP.
    for (threshold, id) in XP_ACHIEVEMENTS {
        if progress.xp >= *threshold {
            unlock(progress, (*id).to_string());
        }
    }
}

/// Actualiza la racha con el día `today`:
/// - mismo día que el último → la racha no cambia (varios eventos en un día
///   cuentan como uno);
/// - el día siguiente al último → la racha crece en uno;
/// - un hueco, un salto hacia atrás, o el primer evento → la racha vuelve a 1.
fn update_streak(progress: &mut Progress, today: NaiveDate) {
    progress.streak_days = match progress.last_active_day {
        Some(last) if last == today => progress.streak_days,
        Some(last) if last.succ_opt() == Some(today) => progress.streak_days + 1,
        _ => 1,
    };
    progress.last_active_day = Some(today);
}

/// Desbloquea un logro si no estaba ya: un logro no se duplica.
fn unlock(progress: &mut Progress, id: String) {
    if !progress.achievements.contains(&id) {
        progress.achievements.push(id);
    }
}

/// El directorio base del estado: `dirs::data_dir()/vasak-health-service`, al
/// lado de `screen-time/`.
pub fn default_dir() -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|data| data.join("vasak-health-service"))
}

/// Lee el progreso guardado en `dir`. Si no hay archivo o no parsea, arranca de
/// cero (`Progress::default()`): un estado roto no tiene que voltear el demonio.
pub fn load(dir: &Path) -> Progress {
    std::fs::read_to_string(dir.join(STATE_FILE))
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

/// Guarda el progreso en `dir`, de forma atómica (mismo `write_atomically` que
/// el tiempo de pantalla).
pub fn save(dir: &Path, progress: &Progress) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let body = serde_json::to_vec_pretty(progress)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    write_atomically(&dir.join(STATE_FILE), &body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::TempDir;

    fn day(n: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 3, n).expect("fecha válida")
    }

    fn event(kind: &str, points: u32) -> Event {
        Event {
            kind: kind.to_string(),
            points,
        }
    }

    #[test]
    fn sin_configuracion_la_gamificacion_queda_apagada() {
        for content in [
            "{}",
            "no es json",
            "",
            r#"{"gamification":{}}"#,
            r#"{"gamification":{"enabled":"true"}}"#,
            r#"{"gamification":{"enabled":1}}"#,
        ] {
            assert!(!gamification_from_json(content).enabled, "{content}");
        }
    }

    #[test]
    fn prendida_sólo_si_la_clave_dice_true() {
        assert!(gamification_from_json(r#"{"gamification":{"enabled":true}}"#).enabled);
        assert!(!gamification_from_json(r#"{"gamification":{"enabled":false}}"#).enabled);
    }

    #[test]
    fn la_configuracion_se_lee_de_la_ruta_de_vasak_conf() {
        // `VASAK_CONFIG_PATH` es del proceso: serializamos con las otras pruebas
        // que la pisan (ver `config::CONFIG_ENV_LOCK`).
        let _env = crate::config::CONFIG_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let temp = TempDir::new("gamificacion-conf");
        let file = temp.0.join("vasak.conf");
        std::fs::write(&file, r#"{"gamification":{"enabled":true}}"#)
            .expect("se escribe la configuración");

        // Una sola prueba de este módulo toca la variable de entorno. Ojo: la
        // comparte con la de `reminders.rs`, que corre en paralelo.
        std::env::set_var("VASAK_CONFIG_PATH", &file);
        let leida = read_gamification_config();
        std::env::set_var("VASAK_CONFIG_PATH", temp.0.join("no-existe.conf"));
        let ausente = read_gamification_config();
        std::env::remove_var("VASAK_CONFIG_PATH");

        assert!(leida.enabled);
        assert!(!ausente.enabled);
    }

    #[test]
    fn la_xp_suma_los_puntos_y_acumula_entre_eventos() {
        let mut progress = Progress::default();
        award(&mut progress, &event("break", 10), day(1));
        assert_eq!(progress.xp, 10);
        award(&mut progress, &event("break", 15), day(1));
        award(&mut progress, &event("hydration", 5), day(1));
        assert_eq!(progress.xp, 30);
    }

    #[test]
    fn el_primer_evento_arranca_la_racha_en_uno() {
        let mut progress = Progress::default();
        award(&mut progress, &event("break", 1), day(5));
        assert_eq!(progress.streak_days, 1);
        assert_eq!(progress.last_active_day, Some(day(5)));
    }

    #[test]
    fn el_dia_siguiente_suma_a_la_racha() {
        // Distingue de la ingenua «siempre resetea»: acá tiene que llegar a 3.
        let mut progress = Progress::default();
        award(&mut progress, &event("break", 1), day(1));
        award(&mut progress, &event("break", 1), day(2));
        assert_eq!(progress.streak_days, 2);
        award(&mut progress, &event("break", 1), day(3));
        assert_eq!(progress.streak_days, 3);
        assert_eq!(progress.last_active_day, Some(day(3)));
    }

    #[test]
    fn varios_eventos_el_mismo_dia_no_inflan_la_racha() {
        // Distingue de la ingenua «siempre +1»: el mismo día no cuenta dos veces.
        let mut progress = Progress::default();
        award(&mut progress, &event("break", 1), day(1));
        award(&mut progress, &event("break", 1), day(2));
        award(&mut progress, &event("break", 1), day(2));
        award(&mut progress, &event("break", 1), day(2));
        assert_eq!(progress.streak_days, 2);
        assert_eq!(progress.last_active_day, Some(day(2)));
    }

    #[test]
    fn un_hueco_de_dias_reinicia_la_racha() {
        let mut progress = Progress::default();
        award(&mut progress, &event("break", 1), day(1));
        award(&mut progress, &event("break", 1), day(2));
        award(&mut progress, &event("break", 1), day(4));
        assert_eq!(progress.streak_days, 1);
        assert_eq!(progress.last_active_day, Some(day(4)));
    }

    #[test]
    fn una_fecha_hacia_atras_reinicia_la_racha() {
        let mut progress = Progress::default();
        award(&mut progress, &event("break", 1), day(10));
        award(&mut progress, &event("break", 1), day(11));
        award(&mut progress, &event("break", 1), day(3));
        assert_eq!(progress.streak_days, 1);
        assert_eq!(progress.last_active_day, Some(day(3)));
    }

    #[test]
    fn los_umbrales_de_xp_desbloquean_su_logro_al_cruzarlos() {
        let mut progress = Progress::default();
        award(&mut progress, &event("break", 99), day(1));
        assert!(!progress.achievements.contains(&"xp-100".to_string()));
        award(&mut progress, &event("break", 1), day(1));
        assert!(progress.achievements.contains(&"xp-100".to_string()));
        assert!(!progress.achievements.contains(&"xp-500".to_string()));

        award(&mut progress, &event("break", 900), day(1));
        for id in ["xp-500", "xp-1000"] {
            assert!(progress.achievements.contains(&id.to_string()), "{id}");
        }
    }

    #[test]
    fn un_logro_de_umbral_no_se_duplica() {
        // Distingue de la ingenua «push siempre»: ya con 100 de XP, seguir
        // sumando no vuelve a agregar `xp-100`.
        let mut progress = Progress::default();
        award(&mut progress, &event("break", 100), day(1));
        award(&mut progress, &event("break", 10), day(1));
        award(&mut progress, &event("break", 10), day(2));
        let cuantos = progress
            .achievements
            .iter()
            .filter(|id| *id == "xp-100")
            .count();
        assert_eq!(cuantos, 1);
    }

    #[test]
    fn el_primer_evento_de_cada_tipo_da_su_logro_una_sola_vez() {
        let mut progress = Progress::default();
        award(&mut progress, &event("break", 1), day(1));
        award(&mut progress, &event("break", 1), day(1));
        assert_eq!(progress.achievements, vec!["first-break".to_string()]);

        award(&mut progress, &event("hydration", 1), day(1));
        assert_eq!(
            progress.achievements,
            vec!["first-break".to_string(), "first-hydration".to_string()]
        );
    }

    #[test]
    fn el_progreso_vuelve_igual_de_disco() {
        let temp = TempDir::new("gamificacion-ida-y-vuelta");
        let mut progress = Progress::default();
        award(&mut progress, &event("break", 120), day(1));
        award(&mut progress, &event("hydration", 5), day(2));

        save(&temp.0, &progress).expect("se guarda");
        assert_eq!(load(&temp.0), progress);
    }

    #[test]
    fn sin_archivo_o_con_archivo_ilegible_se_arranca_de_cero() {
        let temp = TempDir::new("gamificacion-vacio");
        assert_eq!(load(&temp.0), Progress::default());

        std::fs::write(temp.0.join(STATE_FILE), "esto no es json").expect("se escribe");
        assert_eq!(load(&temp.0), Progress::default());
    }

    fn progreso_con_datos() -> Progress {
        Progress {
            xp: 120,
            streak_days: 4,
            last_active_day: Some(day(5)),
            achievements: vec!["primer_aviso".to_string(), "racha_3".to_string()],
            seen_kinds: ["reminder".to_string()].into_iter().collect(),
        }
    }

    #[test]
    fn el_informe_copia_xp_racha_y_logros_tal_cual() {
        let informe = progreso_con_datos().report(true);
        assert_eq!(informe.xp, 120);
        assert_eq!(informe.streak_days, 4);
        assert_eq!(informe.achievements, vec!["primer_aviso", "racha_3"]);
    }

    #[test]
    fn el_interruptor_del_informe_sale_del_parametro_y_no_del_contenido() {
        let apagado = progreso_con_datos().report(false);
        assert!(!apagado.enabled);
        assert_eq!(apagado.xp, 120);
        assert_eq!(apagado.streak_days, 4);

        let prendido_vacio = Progress::default().report(true);
        assert!(prendido_vacio.enabled);
        assert_eq!(prendido_vacio.xp, 0);
        assert_eq!(prendido_vacio.streak_days, 0);
        assert!(prendido_vacio.achievements.is_empty());
    }
}

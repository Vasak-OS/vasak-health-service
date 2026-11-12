//! Recordatorios opt-in de pausa (vasak-health-service#4, v1).
//!
//! Apagados por omisión. Prendidos, el servicio avisa de hacer una pausa cada
//! tanto tiempo *activo* de pantalla —desbloqueada y sin inactividad, que es lo
//! único que este demonio sabe sin mirar el compositor—.
//!
//! Quedan como follow-up, fuera de este v1: distinguir agua de pausa, la franja
//! horaria, y respetar «no molestar» / pantalla completa / modo juego, que
//! necesita que el escritorio le empuje esos estados (hoy sólo empuja la
//! aplicación enfocada, el bloqueo y la inactividad).
//!
//! El aviso lo manda el propio demonio por `org.freedesktop.Notifications`: es
//! una llamada de D-Bus, sin ventana, así que no rompe que el servicio no dibuje
//! nada —no es un informe, es un aviso del momento—.

use std::collections::HashMap;
use std::time::Duration;

use crate::config;

/// Cada cuánto tiempo activo avisa, por omisión, cuando se lo prende.
const DEFAULT_EVERY_MINUTES: u64 = 50;

/// La configuración de los recordatorios, leída de `vasak.conf`.
#[derive(Debug, Clone, PartialEq)]
pub struct BreakReminders {
    pub enabled: bool,
    pub every: Duration,
}

impl BreakReminders {
    /// Apagados, con el intervalo por omisión: lo que vale sin configuración o
    /// con un archivo que no se pudo leer.
    fn off() -> Self {
        Self {
            enabled: false,
            every: Duration::from_secs(DEFAULT_EVERY_MINUTES * 60),
        }
    }
}

/// Lee la sección `break_reminders` de `vasak.conf`: `enabled` (apagado si falta)
/// y `every_minutes` (50 si falta o es inválido; cero no avisa nunca). Un archivo
/// ilegible o sin la sección deja los recordatorios apagados.
pub fn break_reminders_from_json(content: &str) -> BreakReminders {
    let section = serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|cfg| cfg.get("break_reminders").cloned());
    let enabled = section
        .as_ref()
        .and_then(|s| s.get("enabled"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let minutes = section
        .as_ref()
        .and_then(|s| s.get("every_minutes"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(DEFAULT_EVERY_MINUTES);
    BreakReminders {
        enabled,
        every: Duration::from_secs(minutes * 60),
    }
}

/// Lee `vasak.conf` y devuelve la configuración de recordatorios.
pub fn read_break_reminders() -> BreakReminders {
    config::config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|content| break_reminders_from_json(&content))
        .unwrap_or_else(BreakReminders::off)
}

/// Acumula tiempo activo de pantalla y dice cuándo toca avisar.
#[derive(Default)]
pub struct BreakReminder {
    accumulated: Duration,
}

impl BreakReminder {
    /// Suma `dt` de tiempo activo. Devuelve `true` (y reinicia la cuenta) cuando
    /// el acumulado alcanza `every`. Con `every` en cero no avisa nunca y la
    /// cuenta queda en cero.
    pub fn add_active(&mut self, dt: Duration, every: Duration) -> bool {
        if every.is_zero() {
            self.accumulated = Duration::ZERO;
            return false;
        }
        self.accumulated += dt;
        if self.accumulated >= every {
            self.accumulated = Duration::ZERO;
            true
        } else {
            false
        }
    }

    /// Reinicia la cuenta: al apagar los recordatorios no se arrastra lo
    /// acumulado hasta la próxima vez que se los prenda.
    pub fn reset(&mut self) {
        self.accumulated = Duration::ZERO;
    }

    /// Una vuelta del temporizador: si están prendidos y la persona está activa,
    /// suma `step` y dice si toca avisar; apagados, reinicia la cuenta para no
    /// arrastrarla. Es lo que decide el bucle, apartado acá para poder probarlo
    /// sin el reloj ni el D-Bus.
    pub fn due(&mut self, config: &BreakReminders, active: bool, step: Duration) -> bool {
        if !config.enabled {
            self.reset();
            return false;
        }
        active && self.add_active(step, config.every)
    }
}

/// Manda el aviso de pausa por `org.freedesktop.Notifications`. Si no hay un
/// servicio de notificaciones, se registra y se sigue: un recordatorio que no se
/// pudo mostrar no tiene que voltear el demonio.
pub async fn notify_break(connection: &zbus::Connection) {
    // Notify(app_name, replaces_id, app_icon, summary, body, actions, hints,
    // expire_timeout): sin acciones ni pistas, y expiración en -1 que la decide
    // el servicio. Se llama el método crudo para no armar un proxy de un solo uso.
    let hints: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
    let aviso = connection
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                "VasakOS",
                0u32,
                "appointment-soon",
                "Hora de una pausa",
                "Estirá un poco, tomá agua y descansá la vista de la pantalla.",
                Vec::<&str>::new(),
                hints,
                -1i32,
            ),
        )
        .await;
    if let Err(error) = aviso {
        tracing::warn!("no se pudo mostrar el recordatorio: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn sin_configuracion_los_recordatorios_quedan_apagados() {
        for content in ["{}", "no es json", r#"{"break_reminders":{}}"#] {
            assert!(!break_reminders_from_json(content).enabled, "{content}");
        }
    }

    #[test]
    fn prendidos_sólo_si_la_clave_dice_true() {
        let on = break_reminders_from_json(r#"{"break_reminders":{"enabled":true}}"#);
        assert!(on.enabled);
        let off = break_reminders_from_json(r#"{"break_reminders":{"enabled":false}}"#);
        assert!(!off.enabled);
        let texto = break_reminders_from_json(r#"{"break_reminders":{"enabled":"true"}}"#);
        assert!(!texto.enabled);
    }

    #[test]
    fn el_intervalo_sale_de_los_minutos_o_del_valor_por_omisión() {
        assert_eq!(break_reminders_from_json("{}").every, 50 * MIN);
        let con = |m: &str| {
            break_reminders_from_json(&format!(r#"{{"break_reminders":{{"every_minutes":{m}}}}}"#))
                .every
        };
        assert_eq!(con("30"), 30 * MIN);
        assert_eq!(con(r#""mucho""#), 50 * MIN);
        assert_eq!(con("0"), Duration::ZERO);
    }

    #[test]
    fn no_avisa_antes_del_intervalo_y_avisa_justo_al_alcanzarlo() {
        let mut r = BreakReminder::default();
        let every = 10 * MIN;
        assert!(!r.add_active(4 * MIN, every));
        assert!(!r.add_active(5 * MIN, every));
        assert!(r.add_active(MIN, every));
    }

    #[test]
    fn tras_avisar_la_cuenta_vuelve_a_cero() {
        let mut r = BreakReminder::default();
        let every = 10 * MIN;
        assert!(r.add_active(every, every));
        assert!(!r.add_active(9 * MIN, every));
        assert!(r.add_active(MIN, every));
    }

    #[test]
    fn con_intervalo_cero_nunca_avisa() {
        let mut r = BreakReminder::default();
        for _ in 0..5 {
            assert!(!r.add_active(100 * MIN, Duration::ZERO));
        }
        // Y no arrastra nada al prenderlo después.
        assert!(!r.add_active(MIN, 10 * MIN));
    }

    #[test]
    fn reiniciar_borra_lo_acumulado() {
        let mut r = BreakReminder::default();
        let every = 10 * MIN;
        assert!(!r.add_active(9 * MIN, every));
        r.reset();
        assert!(!r.add_active(2 * MIN, every));
    }

    fn config(enabled: bool, every: Duration) -> BreakReminders {
        BreakReminders { enabled, every }
    }

    #[test]
    fn apagado_nunca_avisa_y_reinicia_la_cuenta() {
        let mut r = BreakReminder::default();
        let on = config(true, 2 * MIN);
        assert!(!r.due(&on, true, MIN));
        assert!(!r.due(&config(false, 2 * MIN), true, 10 * MIN));
        // Al volver a prenderlo la cuenta empezó de cero: tarda las dos vueltas.
        assert!(!r.due(&on, true, MIN));
        assert!(r.due(&on, true, MIN));
    }

    #[test]
    fn prendido_y_activo_avisa_al_alcanzar_el_intervalo_y_reinicia() {
        let mut r = BreakReminder::default();
        let on = config(true, 2 * MIN);
        assert!(!r.due(&on, true, MIN));
        assert!(r.due(&on, true, MIN));
        assert!(!r.due(&on, true, MIN));
    }

    #[test]
    fn prendido_pero_inactivo_no_avisa_ni_acumula() {
        let mut r = BreakReminder::default();
        let on = config(true, 2 * MIN);
        for _ in 0..10 {
            assert!(!r.due(&on, false, MIN));
        }
        assert!(!r.due(&on, true, MIN));
        assert!(r.due(&on, true, MIN));
    }

    #[test]
    fn la_configuracion_se_lee_de_la_ruta_de_vasak_conf() {
        use crate::store::tests::TempDir;
        let temp = TempDir::new("recordatorios");
        let file = temp.0.join("vasak.conf");
        std::fs::write(
            &file,
            r#"{"break_reminders":{"enabled":true,"every_minutes":20}}"#,
        )
        .expect("se escribe la configuración");

        // Una sola prueba toca la variable de entorno: las pruebas corren en paralelo.
        std::env::set_var("VASAK_CONFIG_PATH", &file);
        let leida = read_break_reminders();
        std::env::set_var("VASAK_CONFIG_PATH", temp.0.join("no-existe.conf"));
        let ausente = read_break_reminders();
        std::env::remove_var("VASAK_CONFIG_PATH");

        assert!(leida.enabled);
        assert_eq!(leida.every, Duration::from_secs(1200));
        assert!(!ausente.enabled);
    }
}

//! Recordatorios opt-in de pausa y de hidratación (vasak-health-service#4).
//!
//! Apagados por omisión. Prendidos, el servicio avisa de hacer una pausa, o de
//! tomar agua, cada tanto tiempo *activo* de pantalla —desbloqueada y sin
//! inactividad, que es lo único que este demonio sabe sin mirar el compositor—.
//! Una franja horaria opcional (`reminders_window`) limita los dos tipos a un
//! rango de horas locales.
//!
//! De los tres follow-ups del v1 ya están hechos dos: distinguir agua de pausa
//! y la franja horaria. Queda el tercero, fuera de alcance acá: respetar «no
//! molestar» / pantalla completa / modo juego, que necesita que el escritorio le
//! empuje esos estados (hoy sólo empuja la aplicación enfocada, el bloqueo y la
//! inactividad).
//!
//! El aviso lo manda el propio demonio por `org.freedesktop.Notifications`: es
//! una llamada de D-Bus, sin ventana, así que no rompe que el servicio no dibuje
//! nada —no es un informe, es un aviso del momento—.

use std::collections::HashMap;
use std::time::Duration;

use crate::config;

/// Cada cuánto tiempo activo avisa cada tipo, por omisión, al prenderlo.
const DEFAULT_BREAK_MINUTES: u64 = 50;
const DEFAULT_HYDRATION_MINUTES: u64 = 30;

/// La franja por omisión: todo el día, sin restricción.
const DEFAULT_WINDOW: Window = Window {
    from_hour: 0,
    to_hour: 24,
};

/// La configuración de un recordatorio, leída de `vasak.conf`. Vale igual para la
/// pausa y para la hidratación, que tienen la misma forma.
#[derive(Debug, Clone, PartialEq)]
pub struct ReminderConfig {
    pub enabled: bool,
    pub every: Duration,
}

/// Nombre histórico de la configuración de los recordatorios de pausa.
pub type BreakReminders = ReminderConfig;

/// Lee una sección de recordatorios de `vasak.conf` (`break_reminders` o
/// `hydration_reminders`): `enabled` (apagado si falta) y `every_minutes`
/// (`default_minutes` si falta o es inválido; cero no avisa nunca). Un archivo
/// ilegible o sin la sección deja el recordatorio apagado.
fn reminders_from_json(content: &str, section: &str, default_minutes: u64) -> ReminderConfig {
    let section = serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|cfg| cfg.get(section).cloned());
    let enabled = section
        .as_ref()
        .and_then(|s| s.get("enabled"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let minutes = section
        .as_ref()
        .and_then(|s| s.get("every_minutes"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(default_minutes);
    ReminderConfig {
        enabled,
        every: Duration::from_secs(minutes * 60),
    }
}

/// Configuración de los recordatorios de pausa (sección `break_reminders`).
pub fn break_reminders_from_json(content: &str) -> ReminderConfig {
    reminders_from_json(content, "break_reminders", DEFAULT_BREAK_MINUTES)
}

/// Configuración de los recordatorios de hidratación (sección `hydration_reminders`).
pub fn hydration_reminders_from_json(content: &str) -> ReminderConfig {
    reminders_from_json(content, "hydration_reminders", DEFAULT_HYDRATION_MINUTES)
}

/// Lee `vasak.conf` y lo parsea con `parse`; si el archivo no se puede leer,
/// devuelve el recordatorio apagado con su intervalo por omisión.
fn read_reminders(parse: impl Fn(&str) -> ReminderConfig, default_minutes: u64) -> ReminderConfig {
    config::config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|content| parse(&content))
        .unwrap_or_else(|| ReminderConfig {
            enabled: false,
            every: Duration::from_secs(default_minutes * 60),
        })
}

/// Configuración de los recordatorios de pausa, leída de `vasak.conf`.
pub fn read_break_reminders() -> ReminderConfig {
    read_reminders(break_reminders_from_json, DEFAULT_BREAK_MINUTES)
}

/// Configuración de los recordatorios de hidratación, leída de `vasak.conf`.
pub fn read_hydration_reminders() -> ReminderConfig {
    read_reminders(hydration_reminders_from_json, DEFAULT_HYDRATION_MINUTES)
}

/// Una franja horaria `[from_hour, to_hour)` en horas locales (0–24). Fuera de
/// ella no se avisa. Se aplica a los dos tipos de recordatorio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Window {
    pub from_hour: u32,
    pub to_hour: u32,
}

impl Default for Window {
    fn default() -> Self {
        DEFAULT_WINDOW
    }
}

/// Lee la sección `reminders_window` de `vasak.conf`: `from_hour`/`to_hour`
/// (0–24, acotados a ese rango). Sin la sección, la franja es todo el día.
pub fn window_from_json(content: &str) -> Window {
    let section = serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|cfg| cfg.get("reminders_window").cloned());
    let hour = |key: &str, default: u32| {
        section
            .as_ref()
            .and_then(|s| s.get(key))
            .and_then(serde_json::Value::as_u64)
            .map(|h| h.min(24) as u32)
            .unwrap_or(default)
    };
    Window {
        from_hour: hour("from_hour", DEFAULT_WINDOW.from_hour),
        to_hour: hour("to_hour", DEFAULT_WINDOW.to_hour),
    }
}

/// Lee `vasak.conf` y devuelve la franja horaria de los recordatorios; todo el
/// día si el archivo no se puede leer.
pub fn read_reminder_window() -> Window {
    config::config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|content| window_from_json(&content))
        .unwrap_or_default()
}

/// Dice si `hour` (0–23, hora local) cae dentro de la franja `[from_hour,
/// to_hour)`. Si la franja cruza medianoche (`from_hour > to_hour`, p. ej. 22→6)
/// cubre `[from, 24) ∪ [0, to)`. Con los dos extremos iguales no hay restricción,
/// para no silenciar todos los avisos por una franja vacía. Pura: toma la hora
/// como parámetro para poder probarla sin reloj.
pub fn within_window(hour: u32, window: &Window) -> bool {
    let Window { from_hour, to_hour } = *window;
    if from_hour == to_hour {
        return true;
    }
    if from_hour < to_hour {
        hour >= from_hour && hour < to_hour
    } else {
        hour >= from_hour || hour < to_hour
    }
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
/// El texto de un aviso: ícono del tema, título y cuerpo. Es puro para fijar en
/// una prueba qué dice cada recordatorio sin levantar un bus.
pub struct Notification {
    pub icon: &'static str,
    pub summary: &'static str,
    pub body: &'static str,
}

/// El aviso de pausa.
pub fn break_notification() -> Notification {
    Notification {
        icon: "appointment-soon",
        summary: "Hora de una pausa",
        body: "Estirá un poco, tomá agua y descansá la vista de la pantalla.",
    }
}

/// El aviso de hidratación. Ícono de gotas (freedesktop `weather-showers`); si el
/// tema no lo trae, el aviso se muestra igual sin ícono.
pub fn hydration_notification() -> Notification {
    Notification {
        icon: "weather-showers",
        summary: "Hora de tomar agua",
        body: "Tomá un vaso de agua para mantenerte hidratado.",
    }
}

/// Manda un aviso por `org.freedesktop.Notifications`. Si no hay un servicio de
/// notificaciones, se registra y se sigue: un recordatorio que no se pudo mostrar
/// no tiene que voltear el demonio.
async fn notify(connection: &zbus::Connection, n: Notification) {
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
                n.icon,
                n.summary,
                n.body,
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

pub async fn notify_break(connection: &zbus::Connection) {
    notify(connection, break_notification()).await
}

/// Manda el aviso de hidratación por `org.freedesktop.Notifications`, igual que
/// [`notify_break`]: si no hay un servicio de notificaciones, se registra y se
/// sigue.
pub async fn notify_hydration(connection: &zbus::Connection) {
    notify(connection, hydration_notification()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn cada_aviso_tiene_su_texto_y_su_icono() {
        let pausa = break_notification();
        assert_eq!(pausa.icon, "appointment-soon");
        assert_eq!(pausa.summary, "Hora de una pausa");
        assert!(pausa.body.contains("pantalla"));

        let agua = hydration_notification();
        assert_eq!(agua.icon, "weather-showers");
        assert_eq!(agua.summary, "Hora de tomar agua");
        assert!(agua.body.contains("agua"));

        // Son avisos distintos: no se confunde uno con el otro.
        assert_ne!(pausa.summary, agua.summary);
    }

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
        // `VASAK_CONFIG_PATH` es del proceso: serializamos con las otras pruebas
        // que la pisan (ver `config::CONFIG_ENV_LOCK`).
        let _env = crate::config::CONFIG_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let temp = TempDir::new("recordatorios");
        let file = temp.0.join("vasak.conf");
        std::fs::write(
            &file,
            r#"{"break_reminders":{"enabled":true,"every_minutes":20},
                "hydration_reminders":{"enabled":true,"every_minutes":45},
                "reminders_window":{"from_hour":22,"to_hour":6}}"#,
        )
        .expect("se escribe la configuración");

        // Una sola prueba toca la variable de entorno (las pruebas corren en
        // paralelo), así que todas las lecturas de archivo van juntas acá.
        std::env::set_var("VASAK_CONFIG_PATH", &file);
        let leida = read_break_reminders();
        let hidratacion = read_hydration_reminders();
        let franja = read_reminder_window();
        std::env::set_var("VASAK_CONFIG_PATH", temp.0.join("no-existe.conf"));
        let ausente = read_break_reminders();
        let hidratacion_ausente = read_hydration_reminders();
        let franja_ausente = read_reminder_window();
        std::env::remove_var("VASAK_CONFIG_PATH");

        assert!(leida.enabled);
        assert_eq!(leida.every, Duration::from_secs(1200));
        assert!(hidratacion.enabled);
        assert_eq!(hidratacion.every, 45 * MIN);
        assert_eq!(
            franja,
            Window {
                from_hour: 22,
                to_hour: 6
            }
        );
        assert!(!ausente.enabled);
        assert!(!hidratacion_ausente.enabled);
        assert_eq!(hidratacion_ausente.every, 30 * MIN);
        assert_eq!(franja_ausente, Window::default());
    }

    #[test]
    fn la_hidratacion_queda_apagada_sin_seccion_o_con_enabled_inválido() {
        for content in [
            "{}",
            "no es json",
            r#"{"hydration_reminders":{}}"#,
            r#"{"hydration_reminders":{"enabled":"true"}}"#,
            r#"{"break_reminders":{"enabled":true}}"#,
        ] {
            assert!(!hydration_reminders_from_json(content).enabled, "{content}");
        }
        let on = hydration_reminders_from_json(r#"{"hydration_reminders":{"enabled":true}}"#);
        assert!(on.enabled);
    }

    #[test]
    fn el_intervalo_de_hidratacion_es_30_minutos_por_omisión() {
        assert_eq!(hydration_reminders_from_json("{}").every, 30 * MIN);
        let con = |m: &str| {
            hydration_reminders_from_json(&format!(
                r#"{{"hydration_reminders":{{"every_minutes":{m}}}}}"#
            ))
            .every
        };
        assert_eq!(con("45"), 45 * MIN);
        assert_eq!(con(r#""mucho""#), 30 * MIN);
        assert_eq!(con("0"), Duration::ZERO);
    }

    fn franja(from_hour: u32, to_hour: u32) -> Window {
        Window { from_hour, to_hour }
    }

    #[test]
    fn sin_seccion_la_franja_es_todo_el_dia() {
        for content in ["{}", "no es json", r#"{"reminders_window":{}}"#] {
            assert_eq!(window_from_json(content), franja(0, 24), "{content}");
        }
    }

    #[test]
    fn la_franja_lee_las_horas_y_las_acota_a_0_24() {
        let leer = |s: &str| window_from_json(&format!(r#"{{"reminders_window":{s}}}"#));
        assert_eq!(leer(r#"{"from_hour":9,"to_hour":18}"#), franja(9, 18));
        assert_eq!(leer(r#"{"from_hour":9}"#), franja(9, 24));
        assert_eq!(leer(r#"{"to_hour":18}"#), franja(0, 18));
        assert_eq!(leer(r#"{"from_hour":30,"to_hour":99}"#), franja(24, 24));
        // Un valor que no es entero no negativo cae en el valor por omisión.
        assert_eq!(leer(r#"{"from_hour":-3,"to_hour":"tarde"}"#), franja(0, 24));
    }

    #[test]
    fn una_franja_normal_avisa_dentro_y_no_fuera_con_el_final_exclusivo() {
        let w = franja(9, 18);
        for h in [9, 12, 17] {
            assert!(within_window(h, &w), "{h} está dentro");
        }
        // 18 es el final: exclusivo.
        for h in [0, 8, 18, 23] {
            assert!(!within_window(h, &w), "{h} está fuera");
        }
    }

    #[test]
    fn con_los_extremos_iguales_no_hay_restricción() {
        let w = franja(9, 9);
        for h in 0..24 {
            assert!(within_window(h, &w), "{h}");
        }
    }

    #[test]
    fn una_franja_que_cruza_medianoche_cubre_la_noche_y_la_madrugada() {
        let w = franja(22, 6);
        for h in [22, 23, 0, 5] {
            assert!(within_window(h, &w), "{h} está dentro");
        }
        // 6 es el final: exclusivo; 21 es justo antes del comienzo.
        for h in [6, 12, 21] {
            assert!(!within_window(h, &w), "{h} está fuera");
        }
    }

    #[test]
    fn la_franja_de_todo_el_dia_siempre_avisa() {
        for h in 0..24 {
            assert!(within_window(h, &franja(0, 24)), "{h}");
            assert!(within_window(h, &Window::default()), "{h}");
        }
    }
}

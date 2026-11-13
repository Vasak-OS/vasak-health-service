# Changelog

Todas las novedades de este repositorio, por paquete.

## vasak-health-service 0.5.0

El progreso de gamificación ahora se puede consultar por D-Bus
(vasak-health-service#5).

- **`protocol`** — tipo `GamificationProgress` (enabled, XP, racha, logros) y
  método `Gamification` en la interfaz.
- **`daemon`** — el servicio responde el progreso leyéndolo del disco en cada
  consulta (el bucle lo guarda de forma atómica tras cada evento, así que está
  al día). Apagada la capa, responde en cero sin leer nada. Queda para después
  la interfaz en Configuración que lo muestre.

## vasak-health-service 0.4.0

Capa de logros y rachas opcional (vasak-health-service#5).

- **`daemon`** — con `gamification.enabled` en `vasak.conf` (apagada por
  omisión), el servicio lleva XP, una racha de días y logros a partir de
  eventos que emiten los módulos; hoy el primer consumidor son los
  recordatorios de pausa e hidratación, que suman al cumplirse. El estado vive
  en texto plano junto a los datos de tiempo de pantalla; no son secretos.
  Queda para después la interfaz de activación y de ver el progreso (en
  Configuración), y que otros módulos emitan eventos.

## vasak-health-service 0.3.0

Recordatorios de hidratación y franja horaria (vasak-health-service#4).

- **`daemon`** — además de los de pausa, con `hydration_reminders.enabled` en
  `vasak.conf` el servicio avisa de tomar agua cada tanto tiempo *activo* de
  pantalla; el intervalo sale de `hydration_reminders.every_minutes`, 30 por
  omisión. Y una franja horaria, `reminders_window.from_hour`/`to_hour` (0–24,
  todo el día si falta; cruza la medianoche cuando `from > to`), deja los avisos
  —de pausa y de agua— sólo dentro de esas horas. Sigue pendiente respetar «no
  molestar» / pantalla completa / modo juego, que necesita que el escritorio le
  empuje esos estados (hoy no le llegan).

## vasak-health-service 0.2.0

Recordatorios de pausa, apagados por omisión (vasak-health-service#4).

- **`daemon`** — con `break_reminders.enabled` en `vasak.conf`, el servicio avisa
  de hacer una pausa cada tanto tiempo *activo* de pantalla (desbloqueada y sin
  inactividad); el intervalo sale de `break_reminders.every_minutes`, 50 por
  omisión. El aviso va por `org.freedesktop.Notifications`, sin ventana propia.
  Queda para después: distinguir agua de pausa, la franja horaria, y respetar
  «no molestar» / pantalla completa / modo juego —eso necesita que el escritorio
  le empuje esos estados, que hoy no le llegan—.

## vasak-health-service 0.1.0

Primera versión. El servicio de salud de VasakOS, con su primera medida: el
tiempo de pantalla por aplicación.

- **`protocol`** — el contrato D-Bus (`ar.net.vasak.os.Health`): las direcciones
  del bus, los tipos que viajan (`AppUsage`, `DayUsage`, `ScreenTimeReport`) y el
  proxy `Health`.
- **`daemon`** — el servicio:
  - La contabilidad (`tracker`), traída del escritorio y con un escalón más fino:
    además del total del día, el desglose **por hora**. Descuenta la inactividad,
    reparte lo que cruza la medianoche y cada hora, y no cuenta los saltos del
    reloj.
  - El almacén por día (`store`), un JSON por día con escritura atómica. Nuevo
    esquema versión 2 con las horas; sigue leyendo los archivos de la versión 1
    (sólo el total) e **importa una vez** lo que hubiera dejado el escritorio en
    `vasak-desktop/screen-time/`.
  - La **categoría** de cada aplicación (`category`), sacada de su `.desktop`.
  - El interruptor (`config` + `SetEnabled`), que lee `screen_time.enabled` de
    `vasak.conf` al arrancar y lo cambia en el acto.
  - La interfaz D-Bus (`service`): las muestras que empuja el escritorio
    (`PushSample`, `IdleStarted`, `IdleEnded`, `IdleMonitorLost`, `Flush`) y las
    consultas de cualquiera (`ScreenTime`, `ClearScreenTime`, `SetEnabled`,
    `Enabled`).
- **`packaging`** — la unidad de usuario de systemd y la activación por D-Bus.

# Changelog

Todas las novedades de este repositorio, por paquete.

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
